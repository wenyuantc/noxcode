//! 远程 MCP 服务器（http / sse 传输）的 OAuth 2.1 授权。
//!
//! 采用授权码 + PKCE（S256）流程：
//! 1. `start_mcp_oauth` 生成 `state` 与 PKCE verifier，在 `127.0.0.1` 起一个一次性的
//!    loopback 监听器，拼出 authorize URL 并交给系统浏览器打开；
//! 2. 浏览器回跳到 `http://127.0.0.1:{port}/callback?code=…&state=…`，监听器校验
//!    `state` 后向 `token_url` 交换令牌，写入系统凭据库（服务名 `noxcode-mcp-oauth`，
//!    账户名为服务器 id），并通过 `native-mcp-oauth` 事件通知前端；
//! 3. 连接服务器时 `bearer_token_for` 读取令牌；若已过期且带 refresh_token 则自动刷新。
//!
//! 说明：不做授权服务器元数据发现和动态客户端注册，`authorize_url` / `token_url` /
//! `client_id` 由用户在服务器配置里填写。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::db::models::{McpOAuthConfig, McpServerConfig};
use crate::native::mcp_servers::load_mcp_document;

const KEYRING_SERVICE: &str = "noxcode-mcp-oauth";
const CALLBACK_PATH: &str = "/callback";
/// 等待浏览器回跳的最长时间。
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// 令牌视为过期前预留的余量，避免边界时刻带着刚过期的令牌发请求。
const EXPIRY_SKEW_SECS: i64 = 30;
const MAX_REQUEST_BYTES: usize = 8 * 1024;
pub const MCP_OAUTH_EVENT: &str = "native-mcp-oauth";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthToken {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    /// Unix 秒；`None` 表示服务器未给出有效期。
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl McpOAuthToken {
    pub fn is_expired_at(&self, now_secs: i64) -> bool {
        match self.expires_at {
            Some(expires_at) => now_secs + EXPIRY_SKEW_SECS >= expires_at,
            None => false,
        }
    }
}

/// 令牌端点的标准响应（RFC 6749 §5.1）。
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStart {
    pub server_id: String,
    pub authorize_url: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStatus {
    pub server_id: String,
    pub authorized: bool,
    pub expires_at: Option<i64>,
    pub has_refresh_token: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthEvent {
    pub server_id: String,
    pub ok: bool,
    pub message: String,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 纯函数：PKCE / URL / 回调解析
// ---------------------------------------------------------------------------

/// 生成 PKCE code_verifier（43 个 base64url 字符）与对应的 S256 challenge。
pub fn pkce_pair() -> (String, String) {
    let mut seed = Vec::with_capacity(32);
    seed.extend_from_slice(Uuid::new_v4().as_bytes());
    seed.extend_from_slice(Uuid::new_v4().as_bytes());
    let verifier = URL_SAFE_NO_PAD.encode(seed);
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

pub fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn build_authorize_url(
    oauth: &McpOAuthConfig,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> Result<String, String> {
    let mut url = reqwest::Url::parse(oauth.authorize_url.trim())
        .map_err(|error| format!("authorize_url 无效: {error}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", oauth.client_id.trim());
        pairs.append_pair("redirect_uri", redirect_uri);
        pairs.append_pair("state", state);
        pairs.append_pair("code_challenge", code_challenge);
        pairs.append_pair("code_challenge_method", "S256");
        let scopes = oauth
            .scopes
            .iter()
            .map(|scope| scope.trim())
            .filter(|scope| !scope.is_empty())
            .collect::<Vec<_>>();
        if !scopes.is_empty() {
            pairs.append_pair("scope", &scopes.join(" "));
        }
    }
    Ok(url.to_string())
}

pub fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let decoded = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                match decoded {
                    Some(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    None => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (percent_decode(key), percent_decode(value)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// 解析回调 HTTP 请求的首行，返回授权码（已校验 state）。
pub fn parse_callback_request(request: &str, expected_state: &str) -> Result<String, String> {
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(format!("回调请求方法应为 GET，收到 {method}"));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Err(format!("回调路径不匹配: {path}"));
    }
    let params = parse_query(query);
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .map(|value| format!("：{value}"))
            .unwrap_or_default();
        return Err(format!("授权服务器返回错误 {error}{description}"));
    }
    let state = params.get("state").map(String::as_str).unwrap_or_default();
    if state != expected_state {
        return Err("state 不匹配，可能是过期或伪造的回调".to_string());
    }
    params
        .get("code")
        .filter(|code| !code.is_empty())
        .cloned()
        .ok_or_else(|| "回调缺少 code 参数".to_string())
}

fn token_from_response(response: TokenResponse, now_secs: i64) -> McpOAuthToken {
    McpOAuthToken {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        token_type: response.token_type,
        expires_at: response
            .expires_in
            .filter(|seconds| *seconds > 0)
            .map(|seconds| now_secs + seconds),
        scope: response.scope,
    }
}

fn callback_html(ok: bool, message: &str) -> String {
    let title = if ok {
        "noxcode：授权成功"
    } else {
        "noxcode：授权失败"
    };
    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>{title}</title>\
<style>body{{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#0f172a;color:#e2e8f0}}\
main{{text-align:center;max-width:32rem;padding:2rem}}h1{{font-size:1.25rem}}p{{color:#94a3b8}}</style></head>\
<body><main><h1>{title}</h1><p>{}</p><p>现在可以关闭此页面并回到 noxcode。</p></main></body></html>",
        html_escape(message)
    )
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn http_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

// ---------------------------------------------------------------------------
// 令牌持久化（系统凭据库）
// ---------------------------------------------------------------------------

fn keyring_entry(server_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, server_id)
        .map_err(|error| format!("访问系统凭据库失败: {error}"))
}

pub fn load_token(server_id: &str) -> Result<Option<McpOAuthToken>, String> {
    match keyring_entry(server_id)?.get_password() {
        Ok(raw) => serde_json::from_str::<McpOAuthToken>(&raw)
            .map(Some)
            .map_err(|error| format!("解析已保存的 OAuth 令牌失败: {error}")),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("读取 OAuth 令牌失败: {error}")),
    }
}

pub fn save_token(server_id: &str, token: &McpOAuthToken) -> Result<(), String> {
    let raw =
        serde_json::to_string(token).map_err(|error| format!("序列化 OAuth 令牌失败: {error}"))?;
    keyring_entry(server_id)?
        .set_password(&raw)
        .map_err(|error| format!("保存 OAuth 令牌失败: {error}"))
}

pub fn delete_token(server_id: &str) -> Result<(), String> {
    match keyring_entry(server_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("删除 OAuth 令牌失败: {error}")),
    }
}

// ---------------------------------------------------------------------------
// 令牌端点交互
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))
}

async fn post_token_request(
    oauth: &McpOAuthConfig,
    form: Vec<(&str, String)>,
) -> Result<McpOAuthToken, String> {
    let client = http_client()?;
    let mut form = form;
    form.push(("client_id", oauth.client_id.trim().to_string()));
    if let Some(secret) = oauth
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        form.push(("client_secret", secret.to_string()));
    }
    let response = client
        .post(oauth.token_url.trim())
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("请求令牌端点失败: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取令牌响应失败: {error}"))?;
    if !status.is_success() {
        let snippet: String = body.chars().take(300).collect();
        return Err(format!("令牌端点返回 HTTP {}: {snippet}", status.as_u16()));
    }
    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|error| format!("解析令牌响应失败: {error}"))?;
    Ok(token_from_response(parsed, now_secs()))
}

async fn exchange_code(
    oauth: &McpOAuthConfig,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<McpOAuthToken, String> {
    post_token_request(
        oauth,
        vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("code_verifier", code_verifier.to_string()),
        ],
    )
    .await
}

async fn refresh_token(
    oauth: &McpOAuthConfig,
    previous: &McpOAuthToken,
    refresh: &str,
) -> Result<McpOAuthToken, String> {
    let mut token = post_token_request(
        oauth,
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh.to_string()),
        ],
    )
    .await?;
    // 刷新响应可以省略 refresh_token，此时沿用旧值。
    if token.refresh_token.is_none() {
        token.refresh_token = previous.refresh_token.clone();
    }
    Ok(token)
}

/// 连接 http / sse 服务器时取 Bearer 令牌：无 OAuth 配置返回 `Ok(None)`；
/// 已授权返回令牌（必要时先刷新）；未授权或无法刷新返回错误说明。
pub async fn bearer_token_for<R: Runtime>(
    _app: &AppHandle<R>,
    server: &McpServerConfig,
) -> Result<Option<String>, String> {
    let Some(oauth) = server.oauth.as_ref() else {
        return Ok(None);
    };
    let Some(token) = load_token(&server.id)? else {
        return Err("尚未完成 OAuth 授权，请在设置 → MCP 服务器中点击「授权」".to_string());
    };
    if !token.is_expired_at(now_secs()) {
        return Ok(Some(token.access_token));
    }
    let Some(refresh) = token
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Err("OAuth 令牌已过期且没有 refresh_token，请重新授权".to_string());
    };
    let refreshed = refresh_token(oauth, &token, refresh)
        .await
        .map_err(|error| format!("刷新 OAuth 令牌失败: {error}"))?;
    save_token(&server.id, &refreshed)?;
    Ok(Some(refreshed.access_token))
}

// ---------------------------------------------------------------------------
// 授权流程
// ---------------------------------------------------------------------------

fn pending_flows() -> &'static Mutex<HashMap<String, tokio::task::JoinHandle<()>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn replace_pending_flow(server_id: &str, handle: Option<tokio::task::JoinHandle<()>>) {
    if let Ok(mut guard) = pending_flows().lock() {
        if let Some(previous) = guard.remove(server_id) {
            previous.abort();
        }
        if let Some(handle) = handle {
            guard.insert(server_id.to_string(), handle);
        }
    }
}

fn find_server<R: Runtime>(app: &AppHandle<R>, server_id: &str) -> Result<McpServerConfig, String> {
    load_mcp_document(app)?
        .servers
        .into_iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| format!("未找到 MCP 服务器 {server_id}"))
}

async fn read_request_head(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("读取回调请求失败: {error}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<(String, tokio::net::TcpStream), (String, Option<tokio::net::TcpStream>)> {
    let accepted = tokio::time::timeout(CALLBACK_TIMEOUT, listener.accept()).await;
    let (mut stream, _) = match accepted {
        Ok(Ok(pair)) => pair,
        Ok(Err(error)) => return Err((format!("接受回调连接失败: {error}"), None)),
        Err(_) => return Err(("等待浏览器授权回跳超时（5 分钟）".to_string(), None)),
    };
    let head = match read_request_head(&mut stream).await {
        Ok(head) => head,
        Err(error) => return Err((error, Some(stream))),
    };
    match parse_callback_request(&head, expected_state) {
        Ok(code) => Ok((code, stream)),
        Err(error) => Err((error, Some(stream))),
    }
}

async fn run_flow<R: Runtime>(
    app: AppHandle<R>,
    server: McpServerConfig,
    oauth: McpOAuthConfig,
    listener: TcpListener,
    redirect_uri: String,
    state: String,
    verifier: String,
) {
    let outcome: Result<(), (String, Option<tokio::net::TcpStream>)> =
        match wait_for_callback(listener, &state).await {
            Ok((code, mut stream)) => match exchange_code(&oauth, &code, &redirect_uri, &verifier).await
            {
                Ok(token) => match save_token(&server.id, &token) {
                    Ok(()) => {
                        let _ = stream
                            .write_all(&http_response(
                                "200 OK",
                                &callback_html(true, &format!("已为「{}」保存访问令牌。", server.name)),
                            ))
                            .await;
                        let _ = stream.shutdown().await;
                        Ok(())
                    }
                    Err(error) => Err((error, Some(stream))),
                },
                Err(error) => Err((error, Some(stream))),
            },
            Err(failure) => Err(failure),
        };
    let (ok, message) = match outcome {
        Ok(()) => (true, format!("「{}」OAuth 授权成功", server.name)),
        Err((error, stream)) => {
            if let Some(mut stream) = stream {
                let _ = stream
                    .write_all(&http_response("400 Bad Request", &callback_html(false, &error)))
                    .await;
                let _ = stream.shutdown().await;
            }
            (false, format!("「{}」OAuth 授权失败：{error}", server.name))
        }
    };
    let _ = app.emit(
        MCP_OAUTH_EVENT,
        McpOAuthEvent {
            server_id: server.id.clone(),
            ok,
            message,
        },
    );
    if let Ok(mut guard) = pending_flows().lock() {
        guard.remove(&server.id);
    }
}

#[tauri::command]
pub async fn start_mcp_oauth<R: Runtime>(
    app: AppHandle<R>,
    server_id: String,
) -> Result<McpOAuthStart, String> {
    let server = find_server(&app, &server_id)?;
    let oauth = server
        .oauth
        .clone()
        .ok_or_else(|| format!("MCP 服务器「{}」未配置 OAuth", server.name))?;
    if oauth.client_id.trim().is_empty() {
        return Err("OAuth 配置缺少 client_id".to_string());
    }
    if oauth.token_url.trim().is_empty() {
        return Err("OAuth 配置缺少 token_url".to_string());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("无法监听本地回调端口: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("读取回调端口失败: {error}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let state = Uuid::new_v4().simple().to_string();
    let (verifier, challenge) = pkce_pair();
    let authorize_url = build_authorize_url(&oauth, &redirect_uri, &state, &challenge)?;

    let handle = tokio::spawn(run_flow(
        app.clone(),
        server.clone(),
        oauth,
        listener,
        redirect_uri.clone(),
        state,
        verifier,
    ));
    replace_pending_flow(&server_id, Some(handle));

    if let Err(error) = tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(authorize_url.clone(), None::<&str>)
    {
        // 打开浏览器失败不算致命：前端仍可展示链接让用户手动打开。
        eprintln!("[mcp-oauth] 打开浏览器失败: {error}");
    }

    Ok(McpOAuthStart {
        server_id,
        authorize_url,
        redirect_uri,
    })
}

#[tauri::command]
pub async fn get_mcp_oauth_status<R: Runtime>(
    _app: AppHandle<R>,
    server_id: String,
) -> Result<McpOAuthStatus, String> {
    let token = load_token(&server_id)?;
    Ok(McpOAuthStatus {
        server_id,
        authorized: token.is_some(),
        expires_at: token.as_ref().and_then(|token| token.expires_at),
        has_refresh_token: token
            .as_ref()
            .and_then(|token| token.refresh_token.as_deref())
            .is_some_and(|value| !value.is_empty()),
    })
}

#[tauri::command]
pub async fn clear_mcp_oauth<R: Runtime>(
    _app: AppHandle<R>,
    server_id: String,
) -> Result<(), String> {
    replace_pending_flow(&server_id, None);
    delete_token(&server_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_oauth() -> McpOAuthConfig {
        McpOAuthConfig {
            client_id: "noxcode-client".to_string(),
            client_secret: None,
            authorize_url: "https://auth.example.com/authorize?audience=mcp".to_string(),
            token_url: "https://auth.example.com/token".to_string(),
            scopes: vec!["mcp:tools".to_string(), " offline_access ".to_string()],
        }
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 附录 B 的示例向量。
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (generated, challenge) = pkce_pair();
        assert_eq!(generated.len(), 43);
        assert_eq!(pkce_challenge(&generated), challenge);
    }

    #[test]
    fn authorize_url_carries_pkce_state_and_scopes() {
        let url = build_authorize_url(
            &sample_oauth(),
            "http://127.0.0.1:4321/callback",
            "state-1",
            "challenge-1",
        )
        .expect("url");
        assert!(url.starts_with("https://auth.example.com/authorize?audience=mcp&"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=noxcode-client"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A4321%2Fcallback"));
        assert!(url.contains("state=state-1"));
        assert!(url.contains("code_challenge=challenge-1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=mcp%3Atools+offline_access"));
    }

    #[test]
    fn callback_request_yields_code_when_state_matches() {
        let request = "GET /callback?code=abc%2Fdef&state=s1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_callback_request(request, "s1").expect("code"), "abc/def");
        let mismatch = parse_callback_request(request, "other").expect_err("state");
        assert!(mismatch.contains("state"));
        let denied = parse_callback_request(
            "GET /callback?error=access_denied&error_description=user+said+no&state=s1 HTTP/1.1\r\n\r\n",
            "s1",
        )
        .expect_err("error");
        assert!(denied.contains("access_denied"));
        assert!(denied.contains("user said no"));
        assert!(parse_callback_request("GET /favicon.ico HTTP/1.1\r\n\r\n", "s1").is_err());
        assert!(parse_callback_request("POST /callback?code=x&state=s1 HTTP/1.1\r\n\r\n", "s1").is_err());
    }

    #[test]
    fn token_expiry_uses_skew() {
        let token = token_from_response(
            TokenResponse {
                access_token: "at".to_string(),
                token_type: Some("Bearer".to_string()),
                expires_in: Some(3600),
                refresh_token: Some("rt".to_string()),
                scope: None,
            },
            1_000,
        );
        assert_eq!(token.expires_at, Some(4_600));
        assert!(!token.is_expired_at(1_000));
        assert!(!token.is_expired_at(4_569));
        assert!(token.is_expired_at(4_570));
        let forever = token_from_response(
            TokenResponse {
                access_token: "at".to_string(),
                token_type: None,
                expires_in: None,
                refresh_token: None,
                scope: None,
            },
            1_000,
        );
        assert_eq!(forever.expires_at, None);
        assert!(!forever.is_expired_at(i64::MAX / 2));
    }

    #[test]
    fn percent_decode_handles_plus_and_invalid_escapes() {
        assert_eq!(percent_decode("a+b%20c%2Fd"), "a b c/d");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
        assert_eq!(percent_decode("trail%2"), "trail%2");
    }

    #[test]
    fn callback_html_escapes_message() {
        let html = callback_html(false, "<script>alert(1)</script>");
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        let response = http_response("200 OK", "hi");
        let text = String::from_utf8(response).expect("utf8");
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.ends_with("\r\n\r\nhi"));
    }
}
