use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};

use super::algorithms::{legacy_preset, SshAlgorithms};
use super::client::{AuthMaterial, ConnectParams};
use super::error::SshError;
use super::exec::ExecOptions;
use super::known_hosts::{HostTrustBroker, HostTrustEvent, KnownHostsPolicy};
use super::pool::SshPool;
use super::test_server::{TestServerOpts, TestSshServer};

fn temp_known_hosts() -> PathBuf {
    let dir = tempfile::tempdir().expect("known_hosts dir");
    let path = dir.path().join("known_hosts");
    std::mem::forget(dir);
    path
}

fn test_pool() -> SshPool {
    SshPool::new(
        Arc::new(HostTrustBroker::new(Duration::from_secs(2))),
        Duration::from_secs(600),
    )
}

fn password_params(port: u16, known_hosts: PathBuf, policy: KnownHostsPolicy) -> ConnectParams {
    ConnectParams {
        ssh_config_id: "test-cfg".to_string(),
        name: "test".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        username: "tester".to_string(),
        auth: AuthMaterial::Password("secret".to_string()),
        policy,
        known_hosts_path: known_hosts,
        algorithms: None,
    }
}

#[tokio::test]
async fn password_auth_success_and_failure() {
    let server = TestSshServer::start(TestServerOpts::default()).await;
    let pool = test_pool();
    let known_hosts = temp_known_hosts();
    let ok = password_params(server.port, known_hosts.clone(), KnownHostsPolicy::Off);
    let output = pool
        .exec(&ok, "echo hello", ExecOptions::default())
        .await
        .expect("password auth");
    assert_eq!(output.stdout_lossy(), "hello");
    assert_eq!(output.exit_code, Some(0));

    let mut bad = ok.clone();
    bad.ssh_config_id = "test-cfg-bad".to_string();
    bad.auth = AuthMaterial::Password("wrong".to_string());
    let error = pool
        .exec(&bad, "echo hello", ExecOptions::default())
        .await
        .expect_err("bad password");
    assert!(matches!(error, SshError::AuthFailed { .. }));
}

#[tokio::test]
async fn publickey_auth_plain_and_encrypted() {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("client key");
    let dir = tempfile::tempdir().expect("key dir");
    let plain_path = dir.path().join("id_ed25519");
    key.write_openssh_file(&plain_path, LineEnding::LF)
        .expect("write key");

    let server = TestSshServer::start(TestServerOpts {
        authorized_public_key: Some(key.public_key().clone()),
        ..TestServerOpts::default()
    })
    .await;
    let pool = test_pool();
    let known_hosts = temp_known_hosts();
    let params = ConnectParams {
        ssh_config_id: "key-cfg".to_string(),
        name: "key".to_string(),
        host: "127.0.0.1".to_string(),
        port: server.port,
        username: "tester".to_string(),
        auth: AuthMaterial::Key {
            path: plain_path,
            passphrase: None,
        },
        policy: KnownHostsPolicy::Off,
        known_hosts_path: known_hosts,
        algorithms: None,
    };
    let output = pool
        .exec(&params, "echo pubkey", ExecOptions::default())
        .await
        .expect("pubkey");
    assert_eq!(output.stdout_lossy(), "pubkey");

    let encrypted = key.encrypt(&mut rand::rng(), "phrase").expect("encrypt");
    let enc_path = dir.path().join("id_ed25519_enc");
    encrypted
        .write_openssh_file(&enc_path, LineEnding::LF)
        .expect("write enc key");
    let mut enc_params = params.clone();
    enc_params.ssh_config_id = "key-enc".to_string();
    enc_params.auth = AuthMaterial::Key {
        path: enc_path,
        passphrase: Some("phrase".to_string()),
    };
    let output = pool
        .exec(&enc_params, "echo enc", ExecOptions::default())
        .await
        .expect("encrypted pubkey");
    assert_eq!(output.stdout_lossy(), "enc");
}

#[tokio::test]
async fn exec_stdout_stderr_exit_stdin_and_timeout() {
    let server = TestSshServer::start(TestServerOpts::default()).await;
    let pool = test_pool();
    let params = password_params(server.port, temp_known_hosts(), KnownHostsPolicy::Off);

    let stdout = pool
        .exec(&params, "echo hi", ExecOptions::default())
        .await
        .expect("stdout");
    assert_eq!(stdout.stdout_lossy(), "hi");
    assert_eq!(stdout.exit_code, Some(0));

    let stderr = pool
        .exec(&params, "stderr boom", ExecOptions::default())
        .await
        .expect("stderr");
    assert_eq!(stderr.stderr_lossy(), "boom");
    assert_eq!(stderr.exit_code, Some(1));

    let exit = pool
        .exec(&params, "exit 7", ExecOptions::default())
        .await
        .expect("exit");
    assert_eq!(exit.exit_code, Some(7));

    let cat = pool
        .exec(
            &params,
            "cat",
            ExecOptions {
                stdin: Some(b"ping".to_vec()),
                timeout: Some(Duration::from_secs(5)),
            },
        )
        .await
        .expect("cat");
    assert_eq!(cat.stdout_lossy(), "ping");

    let timeout = pool
        .exec(
            &params,
            "hang",
            ExecOptions {
                stdin: None,
                timeout: Some(Duration::from_millis(250)),
            },
        )
        .await
        .expect_err("hang");
    assert!(matches!(timeout, SshError::CommandTimeout));
}

#[tokio::test]
async fn pool_reuses_connection_and_reconnects() {
    let server = TestSshServer::start(TestServerOpts::default()).await;
    let pool = test_pool();
    let params = password_params(server.port, temp_known_hosts(), KnownHostsPolicy::Off);

    for _ in 0..3 {
        pool.exec(&params, "echo reuse", ExecOptions::default())
            .await
            .expect("reuse");
    }
    assert_eq!(pool.connect_count(), 1);
    assert_eq!(
        server.connections.load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let futs: Vec<_> = (0..8)
        .map(|_| pool.exec(&params, "echo concurrent", ExecOptions::default()))
        .collect();
    for result in futures_util::future::join_all(futs).await {
        result.expect("concurrent");
    }
    assert_eq!(pool.connect_count(), 1);
    assert_eq!(
        server.connections.load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let _ = pool
        .exec(&params, "disconnect", ExecOptions::default())
        .await;
    pool.exec(&params, "echo again", ExecOptions::default())
        .await
        .expect("reconnect after disconnect");
    assert_eq!(pool.connect_count(), 2);
    assert_eq!(
        server.connections.load(std::sync::atomic::Ordering::SeqCst),
        2
    );

    pool.invalidate("test-cfg").await;
    pool.exec(&params, "echo after-invalidate", ExecOptions::default())
        .await
        .expect("after invalidate");
    assert_eq!(pool.connect_count(), 3);

    let mut other = params.clone();
    other.username = "tester".to_string();
    other.auth = AuthMaterial::Password("secret".to_string());
    other.ssh_config_id = "test-cfg".to_string();
    other.known_hosts_path = temp_known_hosts();
    pool.exec(&other, "echo fingerprint", ExecOptions::default())
        .await
        .expect("fingerprint change");
    assert_eq!(pool.connect_count(), 4);
}

#[tokio::test]
async fn known_hosts_end_to_end() {
    let server = TestSshServer::start(TestServerOpts::default()).await;
    let known_hosts = temp_known_hosts();
    let pool = test_pool();

    let strict = password_params(server.port, known_hosts.clone(), KnownHostsPolicy::Strict);
    let error = pool
        .exec(&strict, "echo no", ExecOptions::default())
        .await
        .expect_err("strict empty");
    assert!(matches!(error, SshError::HostKeyUnknownRejected { .. }));

    let accept = password_params(
        server.port,
        known_hosts.clone(),
        KnownHostsPolicy::AcceptNew,
    );
    pool.exec(&accept, "echo yes", ExecOptions::default())
        .await
        .expect("accept-new");
    assert!(std::fs::read_to_string(&known_hosts)
        .expect("read")
        .contains("127.0.0.1"));

    pool.invalidate("test-cfg").await;
    pool.exec(&strict, "echo again", ExecOptions::default())
        .await
        .expect("strict after learn");

    let fake = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .expect("fake")
        .public_key()
        .clone();
    let recorded = format!(
        "[127.0.0.1]:{} {}\n",
        server.port,
        fake.to_openssh().expect("openssh")
    );
    std::fs::write(&known_hosts, recorded).expect("write fake host key");
    pool.invalidate("test-cfg").await;
    let error = pool
        .exec(&strict, "echo changed", ExecOptions::default())
        .await
        .expect_err("key changed");
    match error {
        SshError::HostKeyChanged { line, .. } => assert_eq!(line, 1),
        other => panic!("expected HostKeyChanged, got {other:?}"),
    }

    let ask_path = temp_known_hosts();
    let ask_trust = Arc::new(HostTrustBroker::new(Duration::from_secs(2)));
    let ask_for_emit = ask_trust.clone();
    ask_trust.set_emitter(move |event| {
        if let HostTrustEvent::Request(prompt) = event {
            let _ = ask_for_emit.resolve(&prompt.prompt_id, true);
        }
    });
    let ask_pool = SshPool::new(ask_trust, Duration::from_secs(600));
    let ask = password_params(server.port, ask_path, KnownHostsPolicy::Ask);
    ask_pool
        .exec(&ask, "echo asked", ExecOptions::default())
        .await
        .expect("ask accept");

    let reject_path = temp_known_hosts();
    let reject_trust = Arc::new(HostTrustBroker::new(Duration::from_secs(2)));
    let reject_for_emit = reject_trust.clone();
    reject_trust.set_emitter(move |event| {
        if let HostTrustEvent::Request(prompt) = event {
            let _ = reject_for_emit.resolve(&prompt.prompt_id, false);
        }
    });
    let reject_pool = SshPool::new(reject_trust, Duration::from_secs(600));
    let reject = password_params(server.port, reject_path, KnownHostsPolicy::Ask);
    let error = reject_pool
        .exec(&reject, "echo no", ExecOptions::default())
        .await
        .expect_err("ask reject");
    assert!(matches!(error, SshError::TrustPromptRejected));
}

#[tokio::test]
async fn configured_algorithms_connect_with_legacy_preset_and_fail_without_overlap() {
    let server = TestSshServer::start(TestServerOpts::default()).await;
    let pool = test_pool();

    let mut legacy = password_params(server.port, temp_known_hosts(), KnownHostsPolicy::Off);
    legacy.algorithms = Some(Box::new(legacy_preset()));
    pool.exec(&legacy, "echo legacy", ExecOptions::default())
        .await
        .expect("legacy preset should retain modern defaults");

    let mut incompatible = password_params(server.port, temp_known_hosts(), KnownHostsPolicy::Off);
    incompatible.ssh_config_id = "test-cfg-no-overlap".to_string();
    incompatible.algorithms = Some(Box::new(SshAlgorithms {
        kex: vec!["diffie-hellman-group1-sha1".to_string()],
        ..SshAlgorithms::default()
    }));
    let error = pool
        .exec(&incompatible, "echo no", ExecOptions::default())
        .await
        .expect_err("server defaults do not offer group1-sha1");
    assert!(
        error.to_string().contains("key exchange")
            || error.to_string().contains("Kex")
            || error.to_string().contains("NoCommon"),
        "unexpected negotiation error: {error}"
    );
}

#[tokio::test]
#[ignore]
async fn real_server_smoke() {
    let host = std::env::var("NOXCODE_SSH_TEST_HOST").expect("NOXCODE_SSH_TEST_HOST");
    let port = std::env::var("NOXCODE_SSH_TEST_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(22);
    let username = std::env::var("NOXCODE_SSH_TEST_USER").expect("NOXCODE_SSH_TEST_USER");
    let auth = if let Ok(path) = std::env::var("NOXCODE_SSH_TEST_KEY_PATH") {
        AuthMaterial::Key {
            path: PathBuf::from(path),
            passphrase: None,
        }
    } else {
        AuthMaterial::Password(
            std::env::var("NOXCODE_SSH_TEST_PASSWORD").expect("NOXCODE_SSH_TEST_PASSWORD"),
        )
    };
    let pool = test_pool();
    let params = ConnectParams {
        ssh_config_id: "real".to_string(),
        name: "real".to_string(),
        host,
        port,
        username,
        auth,
        policy: KnownHostsPolicy::Off,
        known_hosts_path: temp_known_hosts(),
        algorithms: None,
    };
    let output = pool
        .exec(&params, "uname -a", ExecOptions::default())
        .await
        .expect("real smoke");
    assert!(output.success(), "uname failed: {}", output.stderr_lossy());
    assert!(!output.stdout_lossy().trim().is_empty());
}
