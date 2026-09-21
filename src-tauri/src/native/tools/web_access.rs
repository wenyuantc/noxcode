//! WebFetch runs on the agent host. Network trust is deliberately independent
//! from generic tool allow rules, permission hooks, and workspace location.
use std::collections::HashSet;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;
use tokio::time::Instant;

use reqwest::Url;
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, Mutex};

use super::dispatch::{PermissionPrompt, ToolCtx};
use super::permission::{NativePermissionDecision, NativeToolRiskKind, RuleDecision};
use super::web_dns::PinnedResolver;
use super::web_policy::{bypass_proxy, is_public_address};
use crate::app::network_settings::NetworkSettings;

pub type SettingsProvider = Arc<dyn Fn() -> Result<NetworkSettings, String> + Send + Sync>;

pub struct NetworkState {
    pub cache_scope: String,
    grants: std::sync::Mutex<Grants>,
    prompt_lock: Mutex<()>,
}

#[derive(Default)]
struct Grants {
    config: String,
    revision: u64,
    proxy: bool,
    origins: HashSet<String>,
}

impl Default for NetworkState {
    fn default() -> Self {
        Self {
            cache_scope: uuid::Uuid::new_v4().to_string(),
            grants: std::sync::Mutex::new(Grants::default()),
            prompt_lock: Mutex::new(()),
        }
    }
}

pub struct NetworkBudget(Duration);

impl NetworkBudget {
    pub fn new(limit: Duration) -> Self {
        Self(limit)
    }

    pub async fn run<T>(
        &mut self,
        ctx: &ToolCtx,
        operation: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        if self.0.is_zero() {
            return Err("WebFetch 网络操作超过 30 秒已中止".into());
        }
        let started = Instant::now();
        let result = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => Err("WebFetch 已取消".into()),
            result = tokio::time::timeout(self.0, operation) =>
                result.unwrap_or_else(|_| Err("WebFetch 网络操作超过 30 秒已中止".into())),
        };
        self.0 = self.0.saturating_sub(started.elapsed());
        result
    }
}

pub struct FetchSettings {
    settings: NetworkSettings,
    certificates: Vec<reqwest::Certificate>,
    pub identity: String,
    revision: u64,
}

impl FetchSettings {
    pub async fn load(ctx: &ToolCtx) -> Result<Self, String> {
        let settings = if let Some(provider) = ctx.web_settings_provider.clone() {
            tokio::task::spawn_blocking(move || provider())
                .await
                .map_err(|_| "读取网络设置失败".to_string())??
        } else {
            ctx.web_settings.clone()
        };
        if let Some(proxy) = settings.http_proxy.as_deref() {
            parse_url(proxy)?;
        }
        let pem = match settings.ca_cert_path.as_deref() {
            Some(path) => {
                use tokio::io::AsyncReadExt;
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|_| "读取自定义证书失败".to_string())?;
                let mut pem = Vec::new();
                file.take(1024 * 1024 + 1)
                    .read_to_end(&mut pem)
                    .await
                    .map_err(|_| "读取自定义证书失败".to_string())?;
                if pem.len() > 1024 * 1024 {
                    return Err("自定义证书超过 1 MiB".into());
                }
                pem
            }
            None => Vec::new(),
        };
        let certificates = if settings.ca_cert_path.is_some() {
            let certs = reqwest::Certificate::from_pem_bundle(&pem)
                .map_err(|_| "自定义证书不是合法 PEM".to_string())?;
            if certs.is_empty() {
                return Err("自定义证书不是合法 PEM".into());
            }
            certs
        } else {
            Vec::new()
        };
        // Include credentials and certificate contents in identity, never in
        // user-visible metadata. Replacing a CA at the same path invalidates it.
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&settings).map_err(|_| "网络设置无效")?);
        hash.update(&pem);
        let identity = format!("{:x}", hash.finalize());
        // Observe every fetch, including public/direct routes that need no
        // prompt. Revision also prevents a pending A grant surviving A→B→A.
        let revision = {
            let mut grants = ctx
                .web_network
                .grants
                .lock()
                .map_err(|_| "网络授权状态不可用")?;
            if grants.config != identity {
                grants.config = identity.clone();
                grants.revision = grants.revision.wrapping_add(1);
                grants.proxy = false;
            }
            grants.revision
        };
        Ok(Self {
            settings,
            certificates,
            identity,
            revision,
        })
    }

    pub fn proxy_for(&self, url: &Url) -> Option<&str> {
        self.settings.http_proxy.as_deref().filter(|_| {
            !bypass_proxy(
                url.host_str().unwrap_or_default(),
                self.settings.no_proxy.as_deref(),
            )
        })
    }
}

pub fn parse_url(raw: &str) -> Result<Url, String> {
    let mut url = Url::parse(raw).map_err(|_| "url 不是合法地址".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("WebFetch 仅允许带主机名的 http/https 地址".into());
    }
    url.set_fragment(None);
    Ok(url)
}

pub fn origin(url: &Url) -> String {
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    format!(
        "{}://{}:{}",
        url.scheme(),
        host,
        url.port_or_known_default().unwrap_or(0)
    )
}

pub fn sanitized_url(url: &Url) -> String {
    let mut display = url.clone();
    let _ = display.set_username("");
    let _ = display.set_password(None);
    display.to_string()
}

pub fn deny_check(ctx: &ToolCtx, url: &Url) -> Result<(), String> {
    let arguments = serde_json::json!({"url":url.as_str()}).to_string();
    let contract = super::contract::builtin_contract("WebFetch").ok_or("WebFetch 契约缺失")?;
    let rules = ctx.permission_rules.read().map_err(|_| "权限规则不可用")?;
    if let RuleDecision::Deny(_) = rules.evaluate(
        contract,
        "WebFetch",
        &arguments,
        Some(&ctx.rules_workspace_root()),
    ) {
        return Err("WebFetch 权限规则拒绝目标地址".into());
    }
    Ok(())
}

async fn trust(
    ctx: &ToolCtx,
    settings: &FetchSettings,
    kind: NativeToolRiskKind,
    key: &str,
    summary: String,
) -> Result<(), String> {
    ctx.check_execution()?;
    if ctx.cancel.is_cancelled() {
        return Err("WebFetch 已取消".into());
    }
    if ctx.allow_all_high_risk.load(Ordering::SeqCst) {
        return Ok(());
    }
    let _prompt_guard = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => return Err("WebFetch 已取消".into()),
        guard = ctx.web_network.prompt_lock.lock() => guard,
    };
    ctx.check_execution()?;
    let proxy = kind == NativeToolRiskKind::NetworkProxy;
    {
        let grants = ctx
            .web_network
            .grants
            .lock()
            .map_err(|_| "网络授权状态不可用")?;
        let matching_config =
            grants.config == settings.identity && grants.revision == settings.revision;
        if (proxy && matching_config && grants.proxy) || (!proxy && grants.origins.contains(key)) {
            return Ok(());
        }
    }
    let requester = ctx
        .request_permission
        .as_ref()
        .ok_or("当前没有可用的权限确认通道，WebFetch 未执行")?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    requester(
        PermissionPrompt {
            origin: ctx.request_origin(),
            request_id: request_id.clone(),
            tool_name: "WebFetch".into(),
            kind,
            summary,
            remote: false,
            mcp_server_id: None,
            suggested_rule: None,
            file_access: None,
            allow_once_only: false,
        },
        tx,
    );
    let decision = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => None,
        _ = async {
            if ctx.permission_timeout.is_zero() { std::future::pending::<()>().await; }
            else { tokio::time::sleep(ctx.permission_timeout).await; }
        } => None,
        result = rx => result.ok(),
    };
    if decision.is_none() {
        if let Some(expire) = &ctx.expire_permission {
            let _ = expire(request_id).await;
        }
    }
    ctx.check_execution()?;
    if ctx.cancel.is_cancelled() {
        return Err("WebFetch 已取消".into());
    }
    match decision {
        Some(NativePermissionDecision::AllowOnce) => Ok(()),
        Some(NativePermissionDecision::AllowSession) => {
            let mut grants = ctx
                .web_network
                .grants
                .lock()
                .map_err(|_| "网络授权状态不可用")?;
            if proxy {
                if grants.config == settings.identity && grants.revision == settings.revision {
                    grants.proxy = true;
                }
            } else {
                grants.origins.insert(key.to_string());
            }
            Ok(())
        }
        _ => Err("WebFetch 网络权限确认已拒绝或失效".into()),
    }
}

async fn resolve(ctx: &ToolCtx, url: &Url) -> Result<Vec<SocketAddr>, String> {
    let host = url
        .host_str()
        .ok_or("url 缺少主机名")?
        .trim_matches(&['[', ']'][..]);
    let port = url.port_or_known_default().ok_or("url 缺少端口")?;
    #[cfg(test)]
    if let Some(resolver) = &ctx.web_test_resolver {
        return resolver(host.to_string(), port);
    }
    let _ = ctx;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| "WebFetch DNS 解析失败".to_string())?
        .collect();
    if addresses.is_empty() {
        return Err("WebFetch DNS 没有返回地址".into());
    }
    Ok(addresses)
}

/// Every hop (including cached provenance) runs this check before any content
/// is exposed. The client has neither fallback DNS nor environment proxy routes.
pub async fn authorize_hop(
    ctx: &ToolCtx,
    settings: &FetchSettings,
    url: &Url,
    budget: &mut NetworkBudget,
) -> Result<reqwest::Client, String> {
    ctx.check_execution()?;
    deny_check(ctx, url)?;
    let proxy = settings.proxy_for(url);
    let addresses = if proxy.is_none() {
        budget.run(ctx, resolve(ctx, url)).await?
    } else {
        Vec::new()
    };
    let literal = url
        .host_str()
        .unwrap_or_default()
        .trim_matches(&['[', ']'][..])
        .parse::<IpAddr>()
        .ok();
    let private = literal.is_some_and(|ip| !is_public_address(ip))
        || addresses.iter().any(|addr| !is_public_address(addr.ip()));
    let origin = origin(url);
    if private {
        trust(ctx, settings, NativeToolRiskKind::NetworkOrigin, &origin,
            format!("本机 WebFetch 非公网目标：{origin}\n允许该来源访问非公网、专用或特殊用途地址。仅授权此协议、主机和端口。" )).await?;
    }
    if let Some(proxy) = proxy {
        let proxy_origin = origin_for_proxy(proxy)?;
        trust(ctx, settings, NativeToolRiskKind::NetworkProxy, &settings.identity,
            format!("本机 WebFetch 代理信任：{proxy_origin}\n目标来源 {origin}；代理解析目标 DNS，本机无法核验目标 IP。此信任允许代理连接它可访问的目标（包括非公网目标）；配置改变后需重新确认。" )).await?;
    }
    // A rule can change while the user is considering the prompt.
    deny_check(ctx, url)?;
    ctx.check_execution()?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|_| "HTTP 代理无效")?
                .no_proxy(None),
        );
    } else {
        #[cfg(test)]
        let addresses = if let Some(connect) = &ctx.web_test_connect {
            connect(url, addresses)?
        } else {
            addresses
        };
        builder = builder.dns_resolver(Arc::new(PinnedResolver::new(
            url.host_str().unwrap_or_default(),
            addresses,
        )?));
    }
    for certificate in &settings.certificates {
        builder = builder.add_root_certificate(certificate.clone());
    }
    builder
        .build()
        .map_err(|_| "创建 WebFetch HTTP 客户端失败".into())
}

fn origin_for_proxy(proxy: &str) -> Result<String, String> {
    Ok(origin(&parse_url(proxy)?))
}

#[cfg(test)]
pub type TestResolver = Arc<dyn Fn(String, u16) -> Result<Vec<SocketAddr>, String> + Send + Sync>;
#[cfg(test)]
pub type TestConnect =
    Arc<dyn Fn(&Url, Vec<SocketAddr>) -> Result<Vec<SocketAddr>, String> + Send + Sync>;
