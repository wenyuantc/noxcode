use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::dispatch::{execute_tool, PermissionPrompt, ToolCtx};
use super::permission::{NativePermissionDecision as Decision, NativeToolRiskKind as Kind};
use super::web_access::{authorize_hop, parse_url, FetchSettings, NetworkBudget};
use super::LocalWorkspace;
use crate::app::network_settings::NetworkSettings;

struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(reply: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let reply = Arc::new(reply);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let captured = captured.clone();
                let reply = reply.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                        match stream.read_u8().await {
                            Ok(byte) => request.push(byte),
                            Err(_) => return,
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    captured.lock().unwrap().push(request.clone());
                    let _ = stream.write_all(reply(&request).as_bytes()).await;
                });
            }
        });
        Self {
            address,
            requests,
            task,
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://fixture.invalid:{}{path}", self.address.port())
    }
    fn context(&self) -> ToolCtx {
        let mut ctx = context();
        ctx.web_test_resolver = Some(Arc::new(|_, port| {
            Ok(vec![SocketAddr::new("8.8.8.8".parse().unwrap(), port)])
        }));
        let address = self.address;
        ctx.web_test_connect = Some(Arc::new(move |_, checked| {
            assert!(!checked.is_empty());
            Ok(vec![address])
        }));
        ctx
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
fn context() -> ToolCtx {
    ToolCtx::new(LocalWorkspace::new(std::env::temp_dir()))
}
fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn redirect(location: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}
async fn fetch(ctx: &ToolCtx, url: &str) -> Result<String, String> {
    execute_tool(ctx, "WebFetch", &serde_json::json!({"url":url}).to_string()).await
}
fn permissions(ctx: &mut ToolCtx, decision: Decision) -> Arc<Mutex<Vec<PermissionPrompt>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    ctx.request_permission = Some(Arc::new(move |prompt, tx| {
        captured.lock().unwrap().push(prompt);
        let _ = tx.send(decision);
    }));
    seen
}
fn rule(pattern: &str) -> super::permission::PermissionRule {
    serde_json::from_value(serde_json::json!({"capability":"web_fetch","pattern":pattern,"source":"input","scope":"global"})).unwrap()
}

#[tokio::test]
async fn public_dns_is_pinned_to_checked_candidates_and_preserves_host() {
    let server = Server::start(|_| ok("public")).await;
    let ctx = server.context();
    assert!(fetch(&ctx, &server.url("/one"))
        .await
        .unwrap()
        .contains("public"));
    assert!(server.requests.lock().unwrap()[0]
        .contains(&format!("host: fixture.invalid:{}", server.address.port())));
}

#[tokio::test]
async fn mixed_dns_and_ipv6_require_origin_trust_before_connection() {
    let server = Server::start(|_| ok("private")).await;
    for addresses in [
        vec!["8.8.8.8", "10.0.0.1"],
        vec!["::1"],
        vec!["::ffff:127.0.0.1"],
        vec!["64:ff9b::a00:1"],
    ] {
        let mut ctx = server.context();
        let ips: Vec<std::net::IpAddr> = addresses.iter().map(|ip| ip.parse().unwrap()).collect();
        ctx.web_test_resolver = Some(Arc::new(move |_, port| {
            Ok(ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect())
        }));
        let seen = permissions(&mut ctx, Decision::Deny);
        assert!(fetch(&ctx, &server.url("/")).await.is_err());
        assert_eq!(seen.lock().unwrap()[0].kind, Kind::NetworkOrigin);
        assert_eq!(server.count(), 0);
    }
}

#[tokio::test]
async fn once_and_session_grants_are_origin_scoped_and_cache_never_bypasses_them() {
    let server = Server::start(|_| ok("secret")).await;
    let url = format!("http://{}/", server.address);
    let mut ctx = context();
    let seen = permissions(&mut ctx, Decision::AllowOnce);
    fetch(&ctx, &url).await.unwrap();
    fetch(&ctx, &url).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(server.count(), 1);
    let denied = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, &url).await.is_err());
    assert_eq!(denied.lock().unwrap().len(), 1);
    let mut other = context();
    permissions(&mut other, Decision::Deny);
    assert!(fetch(&other, &url).await.is_err());
    let seen = permissions(&mut ctx, Decision::AllowSession);
    fetch(&ctx, &url).await.unwrap();
    fetch(&ctx, &url).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    // Same host, different effective port must prompt again, before connection.
    permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, "http://127.0.0.1:1/").await.is_err());
    assert!(!ctx.allow_all_high_risk.load(Ordering::SeqCst));
}

#[tokio::test]
async fn redirects_to_private_and_cached_redirect_chains_repeat_authorization() {
    let private = Server::start(|_| ok("private destination")).await;
    let destination = format!("http://{}/", private.address);
    let public = Server::start(move |_| redirect(&destination)).await;
    let mut ctx = public.context();
    // Literal IPs retain their checked literal socket; only public hostname is mapped.
    let public_addr = public.address;
    ctx.web_test_connect = Some(Arc::new(move |url, checked| {
        if url.host_str() == Some("fixture.invalid") {
            Ok(vec![public_addr])
        } else {
            Ok(checked)
        }
    }));
    ctx.web_test_resolver = Some(Arc::new(|host, port| {
        Ok(vec![SocketAddr::new(
            host.parse().unwrap_or("8.8.8.8".parse().unwrap()),
            port,
        )])
    }));
    let denied = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, &public.url("/")).await.is_err());
    assert_eq!(private.count(), 0);
    assert_eq!(denied.lock().unwrap()[0].kind, Kind::NetworkOrigin);
    let allowed = permissions(&mut ctx, Decision::AllowOnce);
    fetch(&ctx, &public.url("/")).await.unwrap();
    fetch(&ctx, &public.url("/")).await.unwrap();
    assert_eq!(allowed.lock().unwrap().len(), 2);
    assert_eq!(public.count(), 2);
    assert_eq!(private.count(), 1);
    ctx.permission_rules
        .write()
        .unwrap()
        .deny
        .push(rule("**127.0.0.1**"));
    assert!(fetch(&ctx, &public.url("/"))
        .await
        .unwrap_err()
        .contains("拒绝"));
}

#[tokio::test]
async fn redirect_loop_limit_and_credential_forwarding_are_enforced() {
    let server = Server::start(|request| {
        let path = request.split_whitespace().nth(1).unwrap();
        if path == "/loop" {
            redirect("/loop")
        } else if path == "/start" {
            redirect("http://injected:password@other.invalid/final")
        } else if path == "/final" {
            ok("done")
        } else {
            let n = path.trim_start_matches('/').parse::<usize>().unwrap_or(0);
            redirect(&format!("/{}", n + 1))
        }
    })
    .await;
    let ctx = server.context();
    assert!(fetch(&ctx, &server.url("/loop"))
        .await
        .unwrap_err()
        .contains("循环"));
    let before = server.count();
    assert!(fetch(&ctx, &server.url("/0"))
        .await
        .unwrap_err()
        .contains("5 次"));
    assert_eq!(server.count() - before, 6);
    let url = server
        .url("/start")
        .replace("http://", "http://original:secret@");
    fetch(&ctx, &url).await.unwrap();
    let requests = server.requests.lock().unwrap();
    assert!(requests[requests.len() - 2].contains("authorization: Basic"));
    assert!(!requests
        .last()
        .unwrap()
        .to_ascii_lowercase()
        .contains("authorization:"));
}

#[tokio::test]
async fn current_proxy_settings_invalidate_trust_cache_and_never_fallback() {
    let proxy = Server::start(|_| ok("from proxy")).await;
    let settings = Arc::new(Mutex::new(NetworkSettings {
        http_proxy: Some(format!("http://user:secret@{}", proxy.address)),
        ..Default::default()
    }));
    let current = settings.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    let mut ctx = context();
    ctx.web_settings_provider = Some(Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(current.lock().unwrap().clone())
    }));
    let seen = permissions(&mut ctx, Decision::AllowSession);
    let url = "http://unresolvable.invalid/resource";
    fetch(&ctx, url).await.unwrap();
    fetch(&ctx, url).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(proxy.count(), 1);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    let prompt = seen.lock().unwrap()[0].clone();
    assert_eq!(prompt.kind, Kind::NetworkProxy);
    assert!(!prompt.remote);
    assert!(prompt.summary.contains("无法核验目标 IP"));
    assert!(!prompt.summary.contains("secret"));
    assert!(!prompt.summary.contains("user:"));
    settings.lock().unwrap().no_proxy = Some("unrelated.invalid".into());
    fetch(&ctx, url).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(proxy.count(), 2);
    settings.lock().unwrap().http_proxy = Some("http://127.0.0.1:1".into());
    let err = fetch(&ctx, "http://127.0.0.1:2/").await.unwrap_err();
    assert!(err.contains("请求失败"));
    assert_eq!(seen.lock().unwrap().len(), 4); // origin plus changed proxy
}

#[tokio::test]
async fn no_proxy_uses_direct_policy_and_route_cache_is_separate() {
    let server = Server::start(|_| ok("direct")).await;
    let proxy = Server::start(|_| ok("proxy")).await;
    let mut ctx = server.context();
    ctx.web_settings.http_proxy = Some(format!("http://{}", proxy.address));
    let seen = permissions(&mut ctx, Decision::AllowSession);
    assert!(fetch(&ctx, &server.url("/"))
        .await
        .unwrap()
        .contains("proxy"));
    ctx.web_settings.no_proxy = Some(".invalid".into());
    assert!(fetch(&ctx, &server.url("/"))
        .await
        .unwrap()
        .contains("direct"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(server.count(), 1);
    assert_eq!(proxy.count(), 1);
}

#[tokio::test]
async fn generic_allow_rules_and_permission_hooks_cannot_grant_private_network_trust() {
    let server = Server::start(|_| ok("private")).await;
    let mut ctx = context();
    ctx.permission_rules.write().unwrap().allow.push(rule("**"));
    ctx.hooks = vec![serde_json::from_value(serde_json::json!({
        "id":"allow-hook", "event":"permission_request", "matcher":"WebFetch",
        "command":"printf '{\"decision\":\"allow\"}'", "timeout_secs":2, "enabled":true
    }))
    .unwrap()];
    let seen = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, &format!("http://{}/", server.address))
        .await
        .is_err());
    assert_eq!(seen.lock().unwrap()[0].kind, Kind::NetworkOrigin);
    assert_eq!(server.count(), 0);
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    ctx.permission_rules.write().unwrap().deny.push(rule("**"));
    assert!(fetch(&ctx, &format!("http://{}/", server.address))
        .await
        .unwrap_err()
        .contains("拒绝"));
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn cancellation_ends_network_and_permission_waits_without_grants() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let ctx = context();
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    let request = fetch(&ctx, &url);
    tokio::pin!(request);
    let (stream, _) = tokio::select! { result = &mut request => panic!("unexpected {result:?}"), conn = listener.accept() => conn.unwrap() };
    ctx.cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .unwrap()
        .unwrap_err()
        .contains("取消"));
    drop(stream);
    let mut ctx = context();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started_tx));
    let held = Arc::new(Mutex::new(None));
    let target = held.clone();
    ctx.request_permission = Some(Arc::new(move |_, tx| {
        *target.lock().unwrap() = Some(tx);
        if let Some(tx) = started.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }));
    let pending = fetch(&ctx, &url);
    tokio::pin!(pending);
    tokio::select! { _ = &mut pending => panic!("permission must wait"), _ = started_rx => {} }
    ctx.cancel.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap_err()
        .contains("取消"));
    assert!(held
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(Decision::AllowSession)
        .is_err());
}

#[tokio::test(start_paused = true)]
async fn network_budget_excludes_human_wait_and_bounds_io() {
    let mut ctx = context();
    ctx.request_permission = Some(Arc::new(|_, tx| {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tx.send(Decision::AllowOnce);
        });
    }));
    let settings = FetchSettings::load(&ctx).await.unwrap();
    let mut budget = NetworkBudget::new(Duration::from_secs(30));
    authorize_hop(
        &ctx,
        &settings,
        &parse_url("http://127.0.0.1:1/").unwrap(),
        &mut budget,
    )
    .await
    .unwrap();
    assert!(budget
        .run(&ctx, async {
            tokio::time::sleep(Duration::from_secs(29)).await;
            Ok(())
        })
        .await
        .is_ok());
    assert!(budget
        .run(&ctx, async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(())
        })
        .await
        .unwrap_err()
        .contains("30 秒"));
}

#[tokio::test]
async fn streaming_body_limit_returns_visible_truncation() {
    let server = Server::start(|_| ok(&"x".repeat(1024 * 1024))).await;
    let ctx = server.context();
    let output = fetch(&ctx, &server.url("/")).await.unwrap();
    assert!(output.contains("可能不完整"));
    assert!(output.len() < 100_000);
}

#[tokio::test]
async fn custom_ca_is_used_for_tls_and_ca_contents_change_config_identity() {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    use tokio_rustls::{rustls, TlsAcceptor};
    const CERT: &[u8] = include_bytes!("fixtures/web-test-ca.pem");
    const LEAF: &[u8] = include_bytes!("fixtures/web-test-server.pem");
    const KEY: &[u8] = include_bytes!("fixtures/web-test-key.pem");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from_pem_slice(LEAF).unwrap()],
        PrivateKeyDer::from_pem_slice(KEY).unwrap(),
    )
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut valid = 0;
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            if let Ok(mut tls) = acceptor.accept(stream).await {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(tls.read_u8().await.unwrap());
                }
                tls.write_all(ok("trusted TLS").as_bytes()).await.unwrap();
                valid += 1;
            }
        }
        valid
    });
    let mut ctx = context();
    ctx.web_test_resolver = Some(Arc::new(|_, port| {
        Ok(vec![SocketAddr::new("8.8.8.8".parse().unwrap(), port)])
    }));
    ctx.web_test_connect = Some(Arc::new(move |_, _| Ok(vec![address])));
    let url = format!("https://fixture.invalid:{}/", address.port());
    assert!(fetch(&ctx, &url).await.is_err()); // Untrusted cert must fail.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ca.pem");
    std::fs::write(&path, CERT).unwrap();
    ctx.web_settings.ca_cert_path = Some(path.to_string_lossy().into_owned());
    assert!(fetch(&ctx, &url).await.unwrap().contains("trusted TLS"));
    assert_eq!(server.await.unwrap(), 1);
    let first = FetchSettings::load(&ctx).await.unwrap().identity;
    let mut changed = CERT.to_vec();
    changed.push(b'\n');
    std::fs::write(&path, changed).unwrap();
    assert_ne!(first, FetchSettings::load(&ctx).await.unwrap().identity);
    std::fs::write(&path, "invalid PEM").unwrap();
    assert!(fetch(&ctx, &url).await.unwrap_err().contains("PEM"));
}

#[tokio::test]
async fn stale_main_origin_cannot_consume_a_network_grant() {
    let server = Server::start(|_| ok("must not be requested")).await;
    let mut ctx = context();
    let mailbox = Arc::new(crate::native::steer::SteerMailbox::new(
        "session", "runtime",
    ));
    mailbox.configure(Arc::new(|_| Box::pin(async { Ok(()) })), Arc::new(|_| {}));
    ctx.main_origin = Some(mailbox.begin_turn().await);
    ctx.user_steer = Some(mailbox.clone());
    let turn = mailbox.snapshot().await.turn_id.unwrap();
    ctx.request_permission = Some(Arc::new(move |prompt, tx| {
        assert_eq!(prompt.origin.as_ref().unwrap().instance_id, "runtime");
        let mailbox = mailbox.clone();
        let turn = turn.clone();
        tokio::spawn(async move {
            mailbox
                .accept(
                    &turn,
                    &uuid::Uuid::new_v4().to_string(),
                    "new instruction",
                    &[],
                    vec![],
                )
                .await
                .unwrap();
            let _ = tx.send(Decision::AllowSession);
        });
    }));
    assert!(fetch(&ctx, &format!("http://{}/", server.address))
        .await
        .unwrap_err()
        .contains(crate::native::steer::SUPERSEDED));
    assert_eq!(server.count(), 0);
    // A fresh generation must still prompt: the stale response stored no grant.
    ctx.main_origin = Some(ctx.user_steer.as_ref().unwrap().origin());
    let seen = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, &format!("http://{}/", server.address))
        .await
        .is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deny_added_while_origin_prompt_waits_prevents_connection() {
    let server = Server::start(|_| ok("must not be requested")).await;
    let mut ctx = context();
    let rules = ctx.permission_rules.clone();
    ctx.request_permission = Some(Arc::new(move |_, tx| {
        rules.write().unwrap().deny.push(rule("**"));
        let _ = tx.send(Decision::AllowOnce);
    }));
    assert!(fetch(&ctx, &format!("http://{}/", server.address))
        .await
        .unwrap_err()
        .contains("拒绝"));
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn cancellation_during_body_stream_drops_the_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (started, body_started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\npartial")
            .await
            .unwrap();
        let _ = started.send(());
        let mut byte = [0];
        match tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap()
        {
            Ok(size) => size,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => 0,
            Err(error) => panic!("unexpected socket failure: {error}"),
        }
    });
    let ctx = context();
    ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
    let url = format!("http://{address}/");
    let request = fetch(&ctx, &url);
    tokio::pin!(request);
    tokio::select! { _ = &mut request => panic!("body should be waiting"), _ = body_started => {} }
    ctx.cancel.cancel();
    assert!(request.await.unwrap_err().contains("取消"));
    assert_eq!(server.await.unwrap(), 0);
}

#[tokio::test]
async fn switching_through_a_direct_route_invalidates_previous_proxy_trust() {
    let server = Server::start(|_| ok("direct")).await;
    let proxy = Server::start(|_| ok("proxy")).await;
    let mut ctx = server.context();
    let proxy_settings = NetworkSettings {
        http_proxy: Some(format!("http://{}", proxy.address)),
        ..Default::default()
    };
    ctx.web_settings = proxy_settings.clone();
    let seen = permissions(&mut ctx, Decision::AllowSession);
    fetch(&ctx, &server.url("/")).await.unwrap();
    ctx.web_settings = NetworkSettings::default();
    fetch(&ctx, &server.url("/")).await.unwrap();
    ctx.web_settings = proxy_settings;
    fetch(&ctx, &server.url("/")).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn webfetch_contract_timeout_excludes_permission_prompt_wait() {
    let mut ctx = context();
    ctx.request_permission = Some(Arc::new(|_, tx| {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tx.send(Decision::Deny);
        });
    }));
    let error = fetch(&ctx, "http://127.0.0.1:1/").await.unwrap_err();
    assert!(error.contains("网络权限确认"), "{error}");
}

#[tokio::test]
async fn pending_old_proxy_grant_cannot_reauthorize_after_config_changes() {
    let proxy = Server::start(|_| ok("proxy")).await;
    let mut ctx = context();
    ctx.web_settings.http_proxy = Some(format!("http://{}", proxy.address));
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let seen_tx = Mutex::new(Some(seen_tx));
    let reply = Arc::new(Mutex::new(None));
    let target = reply.clone();
    ctx.request_permission = Some(Arc::new(move |_, tx| {
        *target.lock().unwrap() = Some(tx);
        if let Some(tx) = seen_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }));
    let old = ctx.clone();
    let request = fetch(&old, "http://fixture.invalid/");
    tokio::pin!(request);
    tokio::select! { result = &mut request => panic!("unexpected {result:?}"), _ = seen_rx => {} }
    let original = ctx.web_settings.clone();
    ctx.web_settings = NetworkSettings::default();
    FetchSettings::load(&ctx).await.unwrap();
    ctx.web_settings = original;
    FetchSettings::load(&ctx).await.unwrap();
    reply
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(Decision::AllowSession)
        .unwrap();
    request.await.unwrap(); // The already-prompted request uses its frozen config.
    let denied = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, "http://fixture.invalid/").await.is_err());
    assert_eq!(denied.lock().unwrap().len(), 1);
    assert_eq!(proxy.count(), 1);
}

#[tokio::test]
async fn implicit_environment_proxy_child() {
    let Ok(address) = std::env::var("NOXCODE_TEST_WEB_DIRECT") else {
        return;
    };
    let address: SocketAddr = address.parse().unwrap();
    let mut ctx = context();
    ctx.web_test_resolver = Some(Arc::new(|_, port| {
        Ok(vec![SocketAddr::new("8.8.8.8".parse().unwrap(), port)])
    }));
    ctx.web_test_connect = Some(Arc::new(move |_, _| Ok(vec![address])));
    assert!(fetch(&ctx, "http://fixture.invalid/environment")
        .await
        .unwrap()
        .contains("direct body"));
}

#[tokio::test]
async fn implicit_environment_proxies_never_change_the_selected_direct_route() {
    let direct = Server::start(|_| ok("direct body")).await;
    let trap = Server::start(|_| ok("unexpected proxy")).await;
    let proxy = format!("http://{}", trap.address);
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native::tools::web_tests::implicit_environment_proxy_child",
            "--nocapture",
        ])
        .env("NOXCODE_TEST_WEB_DIRECT", direct.address.to_string());
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(key, &proxy);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(direct.count(), 1);
    assert_eq!(trap.count(), 0);
}

#[tokio::test]
async fn failing_proxy_does_not_fallback_to_a_reachable_direct_target() {
    let direct = Server::start(|_| ok("must not reach direct")).await;
    let unavailable = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = unavailable.local_addr().unwrap();
    drop(unavailable);
    let mut ctx = context();
    ctx.web_settings.http_proxy = Some(format!("http://user:secret@{proxy_address}"));
    permissions(&mut ctx, Decision::AllowSession);
    let error = fetch(&ctx, &format!("http://{}/", direct.address))
        .await
        .unwrap_err();
    assert!(!error.contains("secret"));
    assert!(!error.contains("user:"));
    assert_eq!(direct.count(), 0);
}

#[tokio::test]
async fn https_proxy_tunnel_delegates_destination_dns_only_after_trust() {
    let proxy = Server::start(|_| {
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    })
    .await;
    let mut ctx = context();
    ctx.web_settings.http_proxy = Some(format!("http://{}", proxy.address));
    ctx.web_test_resolver = Some(Arc::new(|_, _| {
        panic!("proxy destination must not resolve locally")
    }));
    let seen = permissions(&mut ctx, Decision::Deny);
    assert!(fetch(&ctx, "https://destination.invalid/").await.is_err());
    assert_eq!(proxy.count(), 0);
    assert_eq!(seen.lock().unwrap()[0].kind, Kind::NetworkProxy);
    permissions(&mut ctx, Decision::AllowOnce);
    assert!(fetch(&ctx, "https://destination.invalid/").await.is_err());
    assert!(
        proxy.requests.lock().unwrap()[0].starts_with("CONNECT destination.invalid:443 HTTP/1.1")
    );
}

#[tokio::test]
async fn https_proxy_transport_requires_trust_and_validates_its_custom_ca() {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    use tokio_rustls::{rustls, TlsAcceptor};

    // Generated solely for loopback tests; the leaf reuses the test-only key.
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![
            CertificateDer::from_pem_slice(include_bytes!("fixtures/web-test-proxy-server.pem"))
                .unwrap(),
        ],
        PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/web-test-key.pem")).unwrap(),
    )
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let captured = connections.clone();
    let proxy = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            captured.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut tls) = acceptor.accept(stream).await {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                    request.push(tls.read_u8().await.unwrap());
                }
                requests.push(String::from_utf8(request).unwrap());
                tls.write_all(ok("HTTPS proxy verified").as_bytes())
                    .await
                    .unwrap();
            }
        }
        requests
    });
    let mut ctx = context();
    ctx.web_settings.http_proxy = Some(format!("https://{address}"));
    ctx.web_test_resolver = Some(Arc::new(|_, _| panic!("proxy resolves destination DNS")));
    let seen = permissions(&mut ctx, Decision::Deny);
    let url = "http://destination.invalid/secure-proxy";
    assert!(fetch(&ctx, url).await.is_err());
    assert_eq!(connections.load(Ordering::SeqCst), 0);
    assert_eq!(seen.lock().unwrap()[0].kind, Kind::NetworkProxy);

    permissions(&mut ctx, Decision::AllowOnce);
    assert!(fetch(&ctx, url).await.is_err());
    let temp = tempfile::tempdir().unwrap();
    let ca = temp.path().join("proxy-ca.pem");
    std::fs::write(&ca, include_bytes!("fixtures/web-test-proxy-ca.pem")).unwrap();
    ctx.web_settings.ca_cert_path = Some(ca.to_string_lossy().into_owned());
    assert!(fetch(&ctx, url)
        .await
        .unwrap()
        .contains("HTTPS proxy verified"));
    let requests = tokio::time::timeout(Duration::from_secs(3), proxy)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET http://destination.invalid/secure-proxy HTTP/1.1"));
}
