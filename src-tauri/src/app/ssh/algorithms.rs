use std::borrow::Cow;
use std::str::FromStr;

use russh::keys::ssh_key::Algorithm;
use russh::Preferred;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshAlgorithms {
    #[serde(default)]
    pub kex: Vec<String>,
    #[serde(default)]
    pub host_key: Vec<String>,
    #[serde(default)]
    pub cipher: Vec<String>,
    #[serde(default)]
    pub mac: Vec<String>,
}

impl SshAlgorithms {
    pub fn is_empty(&self) -> bool {
        self.kex.is_empty()
            && self.host_key.is_empty()
            && self.cipher.is_empty()
            && self.mac.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshSupportedAlgorithms {
    pub supported: SshAlgorithms,
    pub defaults: SshAlgorithms,
    pub legacy_preset: SshAlgorithms,
}

pub fn validate(algorithms: &SshAlgorithms) -> Result<(), String> {
    let invalid_kex = invalid_names(&algorithms.kex, |name| {
        russh::kex::Name::try_from(name).is_ok()
    });
    let invalid_host_key = invalid_names(&algorithms.host_key, |name| {
        Algorithm::from_str(name).is_ok()
    });
    let invalid_cipher = invalid_names(&algorithms.cipher, |name| {
        russh::cipher::Name::try_from(name).is_ok()
    });
    let invalid_mac = invalid_names(&algorithms.mac, |name| {
        russh::mac::Name::try_from(name).is_ok()
    });
    let mut errors = Vec::new();
    if !invalid_kex.is_empty() {
        errors.push(format!("未知 SSH KEX 算法：{}", invalid_kex.join("、")));
    }
    if !invalid_host_key.is_empty() {
        errors.push(format!(
            "未知 SSH Host Key 算法：{}",
            invalid_host_key.join("、")
        ));
    }
    if !invalid_cipher.is_empty() {
        errors.push(format!(
            "未知 SSH Cipher 算法：{}",
            invalid_cipher.join("、")
        ));
    }
    if !invalid_mac.is_empty() {
        errors.push(format!("未知 SSH MAC 算法：{}", invalid_mac.join("、")));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

fn invalid_names<F>(names: &[String], is_valid: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    names
        .iter()
        .map(|name| name.trim())
        .filter(|name| name.is_empty() || !is_valid(name))
        .map(ToOwned::to_owned)
        .collect()
}

pub fn apply(algorithms: &SshAlgorithms, preferred: &mut Preferred) {
    if !algorithms.kex.is_empty() {
        preferred.kex = Cow::Owned(
            algorithms
                .kex
                .iter()
                .filter_map(|name| russh::kex::Name::try_from(name.as_str()).ok())
                .collect(),
        );
    }
    if !algorithms.host_key.is_empty() {
        preferred.key = Cow::Owned(
            algorithms
                .host_key
                .iter()
                .filter_map(|name| Algorithm::from_str(name).ok())
                .collect(),
        );
    }
    if !algorithms.cipher.is_empty() {
        preferred.cipher = Cow::Owned(
            algorithms
                .cipher
                .iter()
                .filter_map(|name| russh::cipher::Name::try_from(name.as_str()).ok())
                .collect(),
        );
    }
    if !algorithms.mac.is_empty() {
        preferred.mac = Cow::Owned(
            algorithms
                .mac
                .iter()
                .filter_map(|name| russh::mac::Name::try_from(name.as_str()).ok())
                .collect(),
        );
    }
}

fn default_algorithms() -> SshAlgorithms {
    SshAlgorithms {
        kex: Preferred::DEFAULT
            .kex
            .iter()
            .filter(|name| russh::kex::Name::try_from(name.as_ref()).is_ok())
            .map(|name| name.as_ref().to_string())
            .collect(),
        host_key: Preferred::DEFAULT
            .key
            .iter()
            .map(ToString::to_string)
            .collect(),
        cipher: Preferred::DEFAULT
            .cipher
            .iter()
            .map(|name| name.as_ref().to_string())
            .collect(),
        mac: Preferred::DEFAULT
            .mac
            .iter()
            .map(|name| name.as_ref().to_string())
            .collect(),
    }
}

fn append_unique(target: &mut Vec<String>, values: &[&str]) {
    for value in values {
        if !target.iter().any(|item| item == value) {
            target.push((*value).to_string());
        }
    }
}

pub fn legacy_preset() -> SshAlgorithms {
    let mut algorithms = default_algorithms();
    append_unique(
        &mut algorithms.kex,
        &[
            "diffie-hellman-group14-sha1",
            "diffie-hellman-group-exchange-sha1",
            "diffie-hellman-group1-sha1",
            "ecdh-sha2-nistp256",
            "ecdh-sha2-nistp384",
            "ecdh-sha2-nistp521",
        ],
    );
    append_unique(&mut algorithms.host_key, &["ssh-rsa"]);
    append_unique(
        &mut algorithms.cipher,
        &[
            "aes128-cbc",
            "aes192-cbc",
            "aes256-cbc",
            "aes128-gcm@openssh.com",
        ],
    );
    append_unique(
        &mut algorithms.mac,
        &["hmac-sha1", "hmac-sha1-etm@openssh.com"],
    );
    algorithms
}

pub fn supported() -> SshSupportedAlgorithms {
    let host_key = [
        "ssh-ed25519",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "rsa-sha2-512",
        "rsa-sha2-256",
        "ssh-rsa",
    ]
    .into_iter()
    .filter(|name| Algorithm::from_str(name).is_ok())
    .map(ToOwned::to_owned)
    .collect();
    SshSupportedAlgorithms {
        supported: SshAlgorithms {
            kex: russh::kex::ALL_KEX_ALGORITHMS
                .iter()
                .map(|name| name.as_ref().to_string())
                .collect(),
            host_key,
            cipher: russh::cipher::ALL_CIPHERS
                .iter()
                .map(|name| name.as_ref().to_string())
                .collect(),
            mac: russh::mac::ALL_MAC_ALGORITHMS
                .iter()
                .map(|name| name.as_ref().to_string())
                .collect(),
        },
        defaults: default_algorithms(),
        legacy_preset: legacy_preset(),
    }
}

#[tauri::command]
pub(crate) fn list_ssh_supported_algorithms() -> SshSupportedAlgorithms {
    supported()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_applies_selected_algorithms() {
        let algorithms = SshAlgorithms {
            kex: vec!["curve25519-sha256".to_string()],
            host_key: vec!["ssh-ed25519".to_string()],
            cipher: vec!["aes256-ctr".to_string()],
            mac: vec!["hmac-sha2-256".to_string()],
        };
        validate(&algorithms).expect("valid algorithms");
        let mut preferred = Preferred::DEFAULT.clone();
        apply(&algorithms, &mut preferred);
        assert_eq!(preferred.kex[0].as_ref(), "curve25519-sha256");
        assert_eq!(preferred.key[0].to_string(), "ssh-ed25519");
        assert_eq!(preferred.cipher[0].as_ref(), "aes256-ctr");
        assert_eq!(preferred.mac[0].as_ref(), "hmac-sha2-256");
    }

    #[test]
    fn rejects_unknown_names_and_builds_legacy_preset() {
        let error = validate(&SshAlgorithms {
            kex: vec!["unknown-kex".to_string()],
            ..SshAlgorithms::default()
        })
        .expect_err("unknown kex");
        assert!(error.contains("unknown-kex"));

        let preset = legacy_preset();
        validate(&preset).expect("legacy algorithms supported by russh");
        assert!(preset
            .kex
            .iter()
            .any(|name| name == "diffie-hellman-group1-sha1"));
        assert!(preset.cipher.iter().any(|name| name == "aes128-cbc"));
        assert!(preset.mac.iter().any(|name| name == "hmac-sha1"));
    }
}
