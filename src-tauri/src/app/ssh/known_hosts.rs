use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use russh::keys::ssh_key::{Algorithm, HashAlg, PublicKey};
use russh::keys::{Error as KeysError, PublicKeyOrCertificate};
use russh::Preferred;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::error::SshError;
use super::shell::user_home_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnownHostsPolicy {
    AcceptNew,
    Strict,
    Ask,
    Off,
}

impl KnownHostsPolicy {
    pub(crate) fn from_mode(mode: &str) -> Self {
        match mode.trim() {
            "strict" => Self::Strict,
            "ask" => Self::Ask,
            "off" => Self::Off,
            _ => Self::AcceptNew,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SshHostTrustPrompt {
    pub prompt_id: String,
    pub ssh_config_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub known_hosts_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SshHostKeyChanged {
    pub ssh_config_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint_sha256: String,
    pub known_hosts_path: String,
    pub line: usize,
}

pub(crate) enum HostTrustEvent {
    Request(SshHostTrustPrompt),
    KeyChanged(SshHostKeyChanged),
}

pub(crate) struct HostVerifyContext {
    pub ssh_config_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub policy: KnownHostsPolicy,
    pub known_hosts_path: PathBuf,
}

type HostTrustEmitter = Box<dyn Fn(HostTrustEvent) + Send + Sync>;

pub(crate) struct HostTrustBroker {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    emitter: RwLock<Option<HostTrustEmitter>>,
    timeout: Duration,
}

impl HostTrustBroker {
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            emitter: RwLock::new(None),
            timeout,
        }
    }

    pub(crate) fn set_emitter<F>(&self, emitter: F)
    where
        F: Fn(HostTrustEvent) + Send + Sync + 'static,
    {
        *self.emitter.write().expect("host trust emitter lock") = Some(Box::new(emitter));
    }

    fn emit(&self, event: HostTrustEvent) {
        if let Some(emitter) = self
            .emitter
            .read()
            .expect("host trust emitter lock")
            .as_ref()
        {
            emitter(event);
        }
    }

    pub(crate) async fn ask(&self, prompt: SshHostTrustPrompt) -> Result<bool, SshError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| SshError::Config("主机确认队列锁定失败".to_string()))?;
            pending.insert(prompt.prompt_id.clone(), tx);
        }
        self.emit(HostTrustEvent::Request(prompt));
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(accepted)) => Ok(accepted),
            Ok(Err(_)) => Err(SshError::TrustPromptRejected),
            Err(_) => Err(SshError::TrustPromptTimeout),
        }
    }

    pub(crate) fn resolve(&self, prompt_id: &str, accept: bool) -> Result<(), String> {
        let sender = self
            .pending
            .lock()
            .map_err(|_| "主机确认队列锁定失败".to_string())?
            .remove(prompt_id);
        match sender {
            Some(sender) => {
                let _ = sender.send(accept);
                Ok(())
            }
            None => Err(format!("找不到待确认的主机指纹请求: {prompt_id}")),
        }
    }

    pub(crate) fn notify_key_changed(&self, info: SshHostKeyChanged) {
        self.emit(HostTrustEvent::KeyChanged(info));
    }
}

pub(crate) fn default_known_hosts_path() -> PathBuf {
    user_home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh")
        .join("known_hosts")
}

pub(crate) fn fingerprint_sha256(key: &PublicKey) -> String {
    format!("{}", key.fingerprint(HashAlg::Sha256))
}

pub(crate) fn key_type_label(key: &PublicKey) -> String {
    key.algorithm().to_string()
}

pub(crate) fn preferred_key_algorithms(
    host: &str,
    port: u16,
    path: &Path,
) -> Option<Vec<Algorithm>> {
    let recorded = russh::keys::known_hosts::known_host_keys_path(host, port, path).ok()?;
    if recorded.is_empty() {
        return None;
    }

    let mut ordered = Vec::new();
    for (_, key) in recorded {
        let algorithm = key.algorithm();
        if !ordered.contains(&algorithm) {
            ordered.push(algorithm);
        }
    }
    for algorithm in Preferred::DEFAULT.key.iter() {
        if !ordered.contains(algorithm) {
            ordered.push(algorithm.clone());
        }
    }
    Some(ordered)
}

pub(crate) async fn verify_server_key(
    ctx: &HostVerifyContext,
    trust: &HostTrustBroker,
    key: &PublicKeyOrCertificate,
) -> Result<bool, SshError> {
    let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
        return Err(SshError::HostCertificateUnsupported);
    };

    if ctx.policy == KnownHostsPolicy::Off {
        return Ok(true);
    }

    let fingerprint = fingerprint_sha256(key);
    let key_type = key_type_label(key);
    let path_display = ctx.known_hosts_path.display().to_string();

    match russh::keys::check_known_hosts_path(&ctx.host, ctx.port, key, &ctx.known_hosts_path) {
        Ok(true) => Ok(true),
        Ok(false) => match ctx.policy {
            KnownHostsPolicy::AcceptNew => {
                russh::keys::known_hosts::learn_known_hosts_path(
                    &ctx.host,
                    ctx.port,
                    key,
                    &ctx.known_hosts_path,
                )?;
                Ok(true)
            }
            KnownHostsPolicy::Strict => Err(SshError::HostKeyUnknownRejected {
                host: ctx.host.clone(),
                port: ctx.port,
                fingerprint,
            }),
            KnownHostsPolicy::Ask => {
                let prompt = SshHostTrustPrompt {
                    prompt_id: Uuid::new_v4().to_string(),
                    ssh_config_id: ctx.ssh_config_id.clone(),
                    name: ctx.name.clone(),
                    host: ctx.host.clone(),
                    port: ctx.port,
                    key_type,
                    fingerprint_sha256: fingerprint,
                    known_hosts_path: path_display,
                };
                let accepted = trust.ask(prompt).await?;
                if !accepted {
                    return Err(SshError::TrustPromptRejected);
                }
                russh::keys::known_hosts::learn_known_hosts_path(
                    &ctx.host,
                    ctx.port,
                    key,
                    &ctx.known_hosts_path,
                )?;
                Ok(true)
            }
            KnownHostsPolicy::Off => Ok(true),
        },
        Err(KeysError::KeyChanged { line }) => {
            trust.notify_key_changed(SshHostKeyChanged {
                ssh_config_id: ctx.ssh_config_id.clone(),
                name: ctx.name.clone(),
                host: ctx.host.clone(),
                port: ctx.port,
                key_type,
                fingerprint_sha256: fingerprint,
                known_hosts_path: path_display.clone(),
                line,
            });
            Err(SshError::HostKeyChanged {
                line,
                path: path_display,
            })
        }
        Err(error) => Err(SshError::Keys(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::PrivateKey;
    use std::fs;
    use std::sync::Arc;

    fn temp_known_hosts() -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("known_hosts");
        std::mem::forget(dir);
        path
    }

    fn random_key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .expect("random key")
            .public_key()
            .clone()
    }

    fn ctx(path: PathBuf, policy: KnownHostsPolicy) -> HostVerifyContext {
        HostVerifyContext {
            ssh_config_id: "cfg-1".to_string(),
            name: "demo".to_string(),
            host: "example.test".to_string(),
            port: 22,
            policy,
            known_hosts_path: path,
        }
    }

    #[test]
    fn check_known_hosts_three_states() {
        let path = temp_known_hosts();
        let key_a = random_key();
        let key_b = random_key();

        assert!(
            !russh::keys::check_known_hosts_path("example.test", 22, &key_a, &path).expect("check")
        );
        let recorded = format!("example.test {}\n", key_a.to_openssh().expect("openssh"));
        fs::write(&path, recorded).expect("write known host");
        assert!(
            russh::keys::check_known_hosts_path("example.test", 22, &key_a, &path).expect("check")
        );
        let error = russh::keys::check_known_hosts_path("example.test", 22, &key_b, &path)
            .expect_err("changed");
        match error {
            KeysError::KeyChanged { line } => assert_eq!(line, 1),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn preferred_key_algorithms_puts_recorded_first() {
        let path = temp_known_hosts();
        let key = PrivateKey::random(
            &mut rand::rng(),
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
        );
        // RSA generation may fail if feature/size is awkward; fall back to Ed25519.
        let key = match key {
            Ok(key) => key.public_key().clone(),
            Err(_) => random_key(),
        };
        russh::keys::known_hosts::learn_known_hosts_path("example.test", 22, &key, &path)
            .expect("learn");
        let ordered = preferred_key_algorithms("example.test", 22, &path).expect("ordered");
        assert_eq!(ordered.first(), Some(&key.algorithm()));
    }

    #[tokio::test]
    async fn policy_accept_new_writes_file() {
        let path = temp_known_hosts();
        let key = random_key();
        let trust = HostTrustBroker::new(Duration::from_secs(1));
        let accepted = verify_server_key(
            &ctx(path.clone(), KnownHostsPolicy::AcceptNew),
            &trust,
            &PublicKeyOrCertificate::from(key.clone()),
        )
        .await
        .expect("accept");
        assert!(accepted);
        assert!(
            russh::keys::check_known_hosts_path("example.test", 22, &key, &path).expect("check")
        );
    }

    #[tokio::test]
    async fn policy_strict_rejects_unknown() {
        let path = temp_known_hosts();
        let key = random_key();
        let trust = HostTrustBroker::new(Duration::from_secs(1));
        let error = verify_server_key(
            &ctx(path, KnownHostsPolicy::Strict),
            &trust,
            &PublicKeyOrCertificate::from(key),
        )
        .await
        .expect_err("strict");
        assert!(matches!(error, SshError::HostKeyUnknownRejected { .. }));
    }

    #[tokio::test]
    async fn policy_off_skips_check() {
        let path = temp_known_hosts();
        let key = random_key();
        let trust = HostTrustBroker::new(Duration::from_secs(1));
        let accepted = verify_server_key(
            &ctx(path.clone(), KnownHostsPolicy::Off),
            &trust,
            &PublicKeyOrCertificate::from(key),
        )
        .await
        .expect("off");
        assert!(accepted);
        assert!(
            !path.exists() || fs::read_to_string(&path).expect("read").is_empty(),
            "off 策略不应写入 known_hosts"
        );
    }

    #[tokio::test]
    async fn policy_ask_accept_reject_timeout() {
        let path = temp_known_hosts();
        let key = random_key();
        let trust = Arc::new(HostTrustBroker::new(Duration::from_millis(80)));
        let trust_for_emit = trust.clone();
        trust.set_emitter(move |event| {
            if let HostTrustEvent::Request(prompt) = event {
                let _ = trust_for_emit.resolve(&prompt.prompt_id, true);
            }
        });
        let accepted = verify_server_key(
            &ctx(path.clone(), KnownHostsPolicy::Ask),
            &trust,
            &PublicKeyOrCertificate::from(key.clone()),
        )
        .await
        .expect("ask accept");
        assert!(accepted);
        assert!(
            russh::keys::check_known_hosts_path("example.test", 22, &key, &path).expect("check")
        );

        let reject_path = temp_known_hosts();
        let reject_key = random_key();
        let reject_trust = Arc::new(HostTrustBroker::new(Duration::from_millis(80)));
        let reject_for_emit = reject_trust.clone();
        reject_trust.set_emitter(move |event| {
            if let HostTrustEvent::Request(prompt) = event {
                let _ = reject_for_emit.resolve(&prompt.prompt_id, false);
            }
        });
        let error = verify_server_key(
            &ctx(reject_path, KnownHostsPolicy::Ask),
            &reject_trust,
            &PublicKeyOrCertificate::from(reject_key),
        )
        .await
        .expect_err("ask reject");
        assert!(matches!(error, SshError::TrustPromptRejected));

        let timeout_path = temp_known_hosts();
        let timeout_key = random_key();
        let timeout_trust = HostTrustBroker::new(Duration::from_millis(30));
        let error = verify_server_key(
            &ctx(timeout_path, KnownHostsPolicy::Ask),
            &timeout_trust,
            &PublicKeyOrCertificate::from(timeout_key),
        )
        .await
        .expect_err("ask timeout");
        assert!(matches!(error, SshError::TrustPromptTimeout));
    }
}
