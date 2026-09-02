use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{AuthResult, Handle, Handler};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{client, Preferred};

use super::error::SshError;
use super::known_hosts::{
    preferred_key_algorithms, verify_server_key, HostTrustBroker, HostVerifyContext,
    KnownHostsPolicy,
};

#[derive(Clone)]
pub(crate) enum AuthMaterial {
    Password(String),
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

#[derive(Clone)]
pub(crate) struct ConnectParams {
    pub ssh_config_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMaterial,
    pub policy: KnownHostsPolicy,
    pub known_hosts_path: PathBuf,
}

impl ConnectParams {
    pub(crate) fn fingerprint(&self) -> String {
        let auth = match &self.auth {
            AuthMaterial::Password(_) => "password".to_string(),
            AuthMaterial::Key { path, passphrase } => {
                format!("key:{}:passphrase={}", path.display(), passphrase.is_some())
            }
        };
        format!(
            "{}|{}|{}|{}|{:?}|{}|{auth}",
            self.ssh_config_id,
            self.host,
            self.port,
            self.username,
            self.policy,
            self.known_hosts_path.display()
        )
    }

    fn verify_context(&self) -> HostVerifyContext {
        HostVerifyContext {
            ssh_config_id: self.ssh_config_id.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            policy: self.policy,
            known_hosts_path: self.known_hosts_path.clone(),
        }
    }
}

pub(crate) struct ClientHandler {
    verify: HostVerifyContext,
    trust: Arc<HostTrustBroker>,
}

impl Handler for ClientHandler {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        verify_server_key(&self.verify, &self.trust, server_public_key).await
    }
}

pub(crate) async fn connect_and_authenticate(
    params: &ConnectParams,
    trust: &Arc<HostTrustBroker>,
) -> Result<Handle<ClientHandler>, SshError> {
    let mut preferred = Preferred::DEFAULT.clone();
    if let Some(algorithms) =
        preferred_key_algorithms(&params.host, params.port, &params.known_hosts_path)
    {
        preferred.key = Cow::Owned(algorithms);
    }

    let config = client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 5,
        preferred,
        ..Default::default()
    };

    let handler = ClientHandler {
        verify: params.verify_context(),
        trust: trust.clone(),
    };

    let mut handle = match tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(
            Arc::new(config),
            (params.host.as_str(), params.port),
            handler,
        ),
    )
    .await
    {
        Ok(Ok(handle)) => handle,
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(SshError::ConnectTimeout),
    };

    let result = match &params.auth {
        AuthMaterial::Password(password) => {
            handle
                .authenticate_password(&params.username, password)
                .await?
        }
        AuthMaterial::Key { path, passphrase } => {
            let path = path.clone();
            let passphrase = passphrase.clone();
            let key = tokio::task::spawn_blocking(move || {
                load_secret_key(&path, passphrase.as_deref())
                    .map_err(|error| SshError::KeyLoad(error.to_string()))
            })
            .await
            .map_err(|error| SshError::KeyLoad(error.to_string()))??;
            let hash_alg = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(
                    &params.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await?
        }
    };

    match result {
        AuthResult::Success => Ok(handle),
        AuthResult::Failure {
            remaining_methods, ..
        } => Err(SshError::AuthFailed {
            remaining_methods: format!("{remaining_methods:?}"),
        }),
    }
}
