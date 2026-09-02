use std::fs::File;
use std::io::{BufRead, BufReader, Cursor};

use serde::{Deserialize, Serialize};
use ssh2_config::{ParseRule, SshConfig};

use super::shell::{current_username, user_home_dir};

const PARSE_RULES: ParseRule =
    ParseRule::ALLOW_UNKNOWN_FIELDS.union(ParseRule::ALLOW_UNSUPPORTED_FIELDS);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SshConfigFileHost {
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub has_proxy_jump: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SshConfigFileImport {
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_path: Option<String>,
    pub proxy_jump: Option<String>,
    pub proxy_jump_unsupported: bool,
    pub warnings: Vec<String>,
}

#[allow(dead_code)]
pub(crate) fn parse_ssh_config_text(text: &str) -> Result<SshConfig, String> {
    let mut cursor = Cursor::new(text.as_bytes());
    SshConfig::default()
        .parse(&mut cursor, PARSE_RULES)
        .map_err(|error| format!("解析 SSH config 失败: {error}"))
}

pub(crate) fn load_default_ssh_config() -> Result<SshConfig, String> {
    let Some(home) = user_home_dir() else {
        return Ok(SshConfig::default());
    };
    let path = home.join(".ssh").join("config");
    if !path.exists() {
        return Ok(SshConfig::default());
    }
    let file = File::open(&path).map_err(|error| format!("读取 ~/.ssh/config 失败: {error}"))?;
    let mut reader = BufReader::new(file);
    parse_ssh_config_reader(&mut reader)
}

fn parse_ssh_config_reader(reader: &mut impl BufRead) -> Result<SshConfig, String> {
    SshConfig::default()
        .parse(reader, PARSE_RULES)
        .map_err(|error| format!("解析 SSH config 失败: {error}"))
}

fn is_concrete_alias(pattern: &str) -> bool {
    !pattern.contains('*') && !pattern.contains('?')
}

pub(crate) fn list_hosts(config: &SshConfig) -> Vec<SshConfigFileHost> {
    let mut hosts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for host in config.get_hosts() {
        for clause in &host.pattern {
            if clause.negated || !is_concrete_alias(&clause.pattern) {
                continue;
            }
            if !seen.insert(clause.pattern.clone()) {
                continue;
            }
            let imported = import_host(config, &clause.pattern);
            hosts.push(SshConfigFileHost {
                alias: imported.alias,
                host: imported.host,
                port: imported.port,
                username: Some(imported.username).filter(|value| !value.is_empty()),
                has_proxy_jump: imported.proxy_jump.is_some(),
            });
        }
    }
    hosts
}

fn default_identity_file() -> Option<String> {
    let home = user_home_dir()?;
    for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
        let path = home.join(".ssh").join(name);
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

pub(crate) fn import_host(config: &SshConfig, alias: &str) -> SshConfigFileImport {
    let params = config.query(alias);
    let host = params
        .host_name
        .clone()
        .unwrap_or_else(|| alias.to_string());
    let port = params.port.unwrap_or(22);
    let username = params
        .user
        .clone()
        .or_else(current_username)
        .unwrap_or_default();
    let private_key_path = params
        .identity_file
        .as_ref()
        .and_then(|files| files.first())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(default_identity_file);
    let proxy_jump = params
        .proxy_jump
        .as_ref()
        .filter(|jumps| !jumps.is_empty())
        .map(|jumps| jumps.join(","));
    let mut warnings = Vec::new();
    if params.unsupported_fields.contains_key("ProxyCommand") {
        warnings.push("ProxyCommand 暂不支持，导入后不会生效".to_string());
    }
    if proxy_jump.is_some() {
        warnings.push("ProxyJump 暂不支持，导入后不会生效".to_string());
    }

    SshConfigFileImport {
        alias: alias.to_string(),
        host,
        port,
        username,
        private_key_path,
        proxy_jump_unsupported: proxy_jump.is_some(),
        proxy_jump,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Host alpha
    HostName 10.0.0.1
    User alice
    IdentityFile /tmp/id_alpha

Host beta
    HostName beta.example.test

Host jumpbox
    HostName jump.example.test
    ProxyJump bastion

Host "*.wildcard"
    HostName ignored.example.test

Host *
    User defaultuser
    Port 2222
    IdentityFile /tmp/id_default
"#;

    #[test]
    fn list_hosts_skips_wildcards_and_merges_defaults() {
        let config = parse_ssh_config_text(SAMPLE).expect("parse");
        let hosts = list_hosts(&config);
        let aliases: Vec<&str> = hosts.iter().map(|host| host.alias.as_str()).collect();
        assert_eq!(aliases, vec!["alpha", "beta", "jumpbox"]);
        assert!(!aliases.iter().any(|alias| alias.contains('*')));

        let alpha = hosts.iter().find(|host| host.alias == "alpha").unwrap();
        assert_eq!(alpha.host, "10.0.0.1");
        assert_eq!(alpha.port, 2222);
        assert_eq!(alpha.username.as_deref(), Some("alice"));
        assert!(!alpha.has_proxy_jump);

        let beta = hosts.iter().find(|host| host.alias == "beta").unwrap();
        assert_eq!(beta.username.as_deref(), Some("defaultuser"));
        assert_eq!(beta.port, 2222);
    }

    #[test]
    fn import_host_marks_proxy_jump_and_expands_identity() {
        let config = parse_ssh_config_text(SAMPLE).expect("parse");
        let jump = import_host(&config, "jumpbox");
        assert_eq!(jump.host, "jump.example.test");
        assert_eq!(jump.proxy_jump.as_deref(), Some("bastion"));
        assert!(jump.proxy_jump_unsupported);
        assert!(jump
            .warnings
            .iter()
            .any(|warning| warning.contains("ProxyJump")));
        assert_eq!(jump.private_key_path.as_deref(), Some("/tmp/id_default"));

        let alpha = import_host(&config, "alpha");
        assert_eq!(alpha.private_key_path.as_deref(), Some("/tmp/id_alpha"));
        assert!(!alpha.proxy_jump_unsupported);
    }
}
