use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::{normalize_optional_text, resolve_existing_file_path};

const SETTINGS_FILE_NAME: &str = "network-settings.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkSettings {
    #[serde(default)]
    pub http_proxy: Option<String>,
    #[serde(default)]
    pub no_proxy: Option<String>,
    #[serde(default)]
    pub ca_cert_path: Option<String>,
}

fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SETTINGS_FILE_NAME)
}

pub(crate) fn normalize_network_settings(
    settings: NetworkSettings,
) -> Result<NetworkSettings, String> {
    let http_proxy = normalize_optional_text(settings.http_proxy.as_deref());
    if let Some(proxy) = http_proxy.as_deref() {
        if !(proxy.starts_with("http://") || proxy.starts_with("https://")) {
            return Err("第一版仅支持 http/https 代理".to_string());
        }
    }

    let no_proxy = normalize_optional_text(settings.no_proxy.as_deref());

    let ca_cert_path = match normalize_optional_text(settings.ca_cert_path.as_deref()) {
        Some(path) => {
            let canonical = resolve_existing_file_path(&path)?;
            let pem =
                fs::read(&canonical).map_err(|error| format!("读取自定义证书失败: {error}"))?;
            let certs = reqwest::Certificate::from_pem_bundle(&pem)
                .map_err(|error| format!("自定义证书不是合法 PEM: {error}"))?;
            if certs.is_empty() {
                return Err("自定义证书不是合法 PEM: 文件中没有证书".to_string());
            }
            Some(canonical.to_string_lossy().into_owned())
        }
        None => None,
    };

    Ok(NetworkSettings {
        http_proxy,
        no_proxy,
        ca_cert_path,
    })
}

pub(crate) fn load_network_settings_from(config_dir: &Path) -> Result<NetworkSettings, String> {
    let path = settings_path(config_dir);
    if !path.exists() {
        return Ok(NetworkSettings::default());
    }

    let raw = fs::read_to_string(&path).map_err(|error| format!("读取网络设置失败: {error}"))?;
    let parsed: NetworkSettings =
        serde_json::from_str(&raw).map_err(|error| format!("解析网络设置失败: {error}"))?;
    Ok(parsed)
}

pub(crate) fn save_network_settings_to(
    config_dir: &Path,
    settings: &NetworkSettings,
) -> Result<(), String> {
    let path = settings_path(config_dir);
    fs::create_dir_all(config_dir).map_err(|error| format!("创建网络设置目录失败: {error}"))?;
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("序列化网络设置失败: {error}"))?;

    let tmp_path = config_dir.join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", std::process::id()));
    fs::write(&tmp_path, raw.as_bytes()).map_err(|error| format!("写入网络设置失败: {error}"))?;
    if let Err(error) = fs::rename(&tmp_path, &path) {
        let _ = fs::remove_file(&path);
        fs::rename(&tmp_path, &path).map_err(|rename_error| {
            let _ = fs::remove_file(&tmp_path);
            format!("写入网络设置失败: {error}; 重试: {rename_error}")
        })?;
    }
    Ok(())
}

pub(crate) fn build_http_client(
    timeout: Duration,
    settings: &NetworkSettings,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(timeout);

    if let Some(proxy_url) = settings.http_proxy.as_deref() {
        let mut proxy =
            reqwest::Proxy::all(proxy_url).map_err(|error| format!("HTTP 代理无效: {error}"))?;
        if let Some(no_proxy) = settings.no_proxy.as_deref() {
            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(no_proxy));
        }
        builder = builder.proxy(proxy);
    }

    if let Some(ca_path) = settings.ca_cert_path.as_deref() {
        let pem = fs::read(ca_path).map_err(|error| format!("读取自定义证书失败: {error}"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|error| format!("自定义证书不是合法 PEM: {error}"))?;
        if certs.is_empty() {
            return Err("自定义证书不是合法 PEM: 文件中没有证书".to_string());
        }
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }

    builder
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))
}

pub(crate) fn load_network_settings<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<NetworkSettings, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    load_network_settings_from(&config_dir)
}

pub(crate) fn proxy_env_vars(settings: &NetworkSettings) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    if let Some(proxy) = settings
        .http_proxy
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            vars.push((key.to_string(), proxy.to_string()));
        }
    }
    if let Some(no_proxy) = settings
        .no_proxy
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        for key in ["NO_PROXY", "no_proxy"] {
            vars.push((key.to_string(), no_proxy.to_string()));
        }
    }
    if let Some(ca_cert_path) = settings
        .ca_cert_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        for key in ["SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS"] {
            vars.push((key.to_string(), ca_cert_path.to_string()));
        }
    }
    vars
}

#[tauri::command]
pub async fn get_network_settings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<NetworkSettings, String> {
    load_network_settings(&app)
}

#[tauri::command]
pub async fn update_network_settings<R: Runtime>(
    app: AppHandle<R>,
    payload: NetworkSettings,
) -> Result<NetworkSettings, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let normalized = normalize_network_settings(payload)?;
    save_network_settings_to(&config_dir, &normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CA_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDFTCCAf2gAwIBAgIUDaqw4FsPEMZNRHmBVA1WNVTAakkwDQYJKoZIhvcNAQEL
BQAwGjEYMBYGA1UEAwwPbm94Y29kZS10ZXN0LWNhMB4XDTI2MDkwMjA5NTU1MloX
DTM2MDgzMDA5NTU1MlowGjEYMBYGA1UEAwwPbm94Y29kZS10ZXN0LWNhMIIBIjAN
BgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsUp94IaUpvX50CMDgCdcSKaDwZ5c
GkJVVND1nJNN/bYCZYwGWGIA1aPgydS/s9gOVWulfBBdT0RpEbpU56bGbbp3cmG3
8xI60Ccep28L5lEGbOASDjD4YBH1DQ/ThhBuIU7T8NeW8RwjzR/UQxYWvzGZMSZ1
s2GdBlelCbG5/XukCV6/OdFhQ0RvtOhn7t34YSDfcZODl/q/9oaq3Zg5Ll5gqbqw
nyWJdNwf6eBB53eBUScL/qRoWFMEbXUgiaCX/1kXJaMviVlQDcomCwgtik/KVA1N
OopMJcvRC22jluRxgr16EJDj2YtjFi2JkltU7Gpj81GYdNtHUwxd63A08QIDAQAB
o1MwUTAdBgNVHQ4EFgQU8gMlhRi3afSHpV7k2z/x7IY0OGwwHwYDVR0jBBgwFoAU
8gMlhRi3afSHpV7k2z/x7IY0OGwwDwYDVR0TAQH/BAUwAwEB/zANBgkqhkiG9w0B
AQsFAAOCAQEAW7uYhSsuusy0ivthn9F7DvjmCthpGQwz5vTOYTZbXzyxNdejS0uo
tpWt46ok682A0IW7Fc24VzEXXLKIe22HghflTeeZIzeNmqn7FkU7nGEdsRIuiu51
wkVBnusuZ9dPtsQ7yMfqgLVN5inGkLel7KSAd/dweerdw0viRmK/WtMsueX1udXe
8Lamm7cI9km7HLc5Dnku0TYbkn/T909YaQS7/x33IMyxEURMMLImE0Qci0lGdeQR
VpiXFfRk10cn5rvzDr655on4hQsQgb5slhhR4Byx8FmA6gOmSNnfCD6ZF0FahKoj
/dpQrfhPc3iXjKMQ9qZtEirBCdGXX19UcQ==
-----END CERTIFICATE-----
"#;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noxcode-network-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = temp_dir();
        let loaded = load_network_settings_from(&dir).expect("load");
        assert_eq!(loaded, NetworkSettings::default());
        cleanup(&dir);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let settings = NetworkSettings {
            http_proxy: Some("http://127.0.0.1:8080".to_string()),
            no_proxy: Some("localhost,127.0.0.1".to_string()),
            ca_cert_path: None,
        };
        save_network_settings_to(&dir, &settings).expect("save");
        let loaded = load_network_settings_from(&dir).expect("load");
        assert_eq!(loaded, settings);
        cleanup(&dir);
    }

    #[test]
    fn rejects_socks_proxy() {
        let err = normalize_network_settings(NetworkSettings {
            http_proxy: Some("socks5://127.0.0.1:1080".to_string()),
            no_proxy: None,
            ca_cert_path: None,
        })
        .expect_err("socks should fail");
        assert!(err.contains("http/https"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_missing_ca_file() {
        let err = normalize_network_settings(NetworkSettings {
            http_proxy: None,
            no_proxy: None,
            ca_cert_path: Some("/tmp/noxcode-missing-ca.pem".to_string()),
        })
        .expect_err("missing ca should fail");
        assert!(
            err.contains("不存在") || err.contains("不可访问"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_non_pem_ca_file() {
        let dir = temp_dir();
        let path = dir.join("not-a-cert.txt");
        fs::write(&path, "not a certificate").expect("write");
        let err = normalize_network_settings(NetworkSettings {
            http_proxy: None,
            no_proxy: None,
            ca_cert_path: Some(path.to_string_lossy().into_owned()),
        })
        .expect_err("invalid pem should fail");
        assert!(err.contains("PEM"), "unexpected error: {err}");
        cleanup(&dir);
    }

    #[test]
    fn builds_client_with_proxy_and_no_proxy() {
        let dir = temp_dir();
        let ca_path = dir.join("ca.pem");
        fs::write(&ca_path, TEST_CA_PEM).expect("write ca");
        let settings = normalize_network_settings(NetworkSettings {
            http_proxy: Some("http://127.0.0.1:8080".to_string()),
            no_proxy: Some("localhost,127.0.0.1".to_string()),
            ca_cert_path: Some(ca_path.to_string_lossy().into_owned()),
        })
        .expect("normalize");
        build_http_client(Duration::from_secs(5), &settings).expect("build client");
        cleanup(&dir);
    }

    #[test]
    fn trims_empty_fields_to_none() {
        let settings = normalize_network_settings(NetworkSettings {
            http_proxy: Some("   ".to_string()),
            no_proxy: Some("".to_string()),
            ca_cert_path: Some("  ".to_string()),
        })
        .expect("normalize");
        assert_eq!(settings, NetworkSettings::default());
    }

    #[test]
    fn builds_proxy_environment_for_child_processes() {
        let vars = proxy_env_vars(&NetworkSettings {
            http_proxy: Some("http://127.0.0.1:8080".to_string()),
            no_proxy: Some("localhost".to_string()),
            ca_cert_path: Some("/tmp/ca.pem".to_string()),
        });
        assert_eq!(
            vars,
            vec![
                (
                    "HTTP_PROXY".to_string(),
                    "http://127.0.0.1:8080".to_string()
                ),
                (
                    "HTTPS_PROXY".to_string(),
                    "http://127.0.0.1:8080".to_string()
                ),
                (
                    "http_proxy".to_string(),
                    "http://127.0.0.1:8080".to_string()
                ),
                (
                    "https_proxy".to_string(),
                    "http://127.0.0.1:8080".to_string()
                ),
                ("NO_PROXY".to_string(), "localhost".to_string()),
                ("no_proxy".to_string(), "localhost".to_string()),
                ("SSL_CERT_FILE".to_string(), "/tmp/ca.pem".to_string()),
                ("NODE_EXTRA_CA_CERTS".to_string(), "/tmp/ca.pem".to_string()),
            ]
        );
    }
}
