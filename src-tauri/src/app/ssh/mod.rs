pub(crate) mod client;
pub(crate) mod config_file;
pub(crate) mod configs;
pub(crate) mod error;
pub(crate) mod exec;
pub(crate) mod known_hosts;
pub(crate) mod pool;
pub(crate) mod shell;

#[cfg(test)]
pub(crate) mod test_server;

#[cfg(test)]
mod integration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::app::secret_store::SecretStore;
use crate::app::shared::{now_sqlite, sqlite_pool};
use crate::db::models::{
    CreateSshConfig, PasswordAuthProbeResult, SshConfig, SshConfigRecord, UpdateSshConfig,
};

use self::client::{AuthMaterial, ConnectParams};
use self::config_file::{import_host, list_hosts, load_default_ssh_config};
use self::configs::{
    create_ssh_config_with, delete_ssh_config_with, fetch_ssh_config_by_id,
    fetch_ssh_config_record_by_id, list_ssh_config_records, ssh_config_target_host_label,
    update_ssh_config_with, write_connection_check_result, write_password_probe_result,
};
use self::exec::{ExecOptions, SshCommandOutput};
use self::known_hosts::{default_known_hosts_path, KnownHostsPolicy};
use self::shell::{build_remote_shell_command, expand_tilde};

pub(crate) use self::config_file::{SshConfigFileHost, SshConfigFileImport};
pub(crate) use self::known_hosts::{HostTrustBroker, HostTrustEvent};
pub(crate) use self::pool::SshPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SshConnectionTestResult {
    pub ssh_config_id: String,
    pub target_host_label: String,
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub uname: Option<String>,
    pub remote_git_version: Option<String>,
    pub checked_at: String,
}

pub(crate) fn resolve_connect_params<R: Runtime>(
    app: &AppHandle<R>,
    record: &SshConfigRecord,
    require_password_probe: bool,
) -> Result<ConnectParams, String> {
    if require_password_probe
        && record.auth_type == "password"
        && !matches!(
            record.password_probe_status.as_deref(),
            Some("passed" | "available")
        )
    {
        return Err("密码认证尚未通过探测，禁止执行远端命令".to_string());
    }

    let secrets = SecretStore::for_app(app)?;
    let auth = match record.auth_type.as_str() {
        "password" => {
            let password = secrets
                .resolve(record.password_ref.as_deref())?
                .ok_or_else(|| "未配置 SSH 密码".to_string())?;
            AuthMaterial::Password(password)
        }
        "key" => {
            let path = record
                .private_key_path
                .as_deref()
                .ok_or_else(|| "密钥认证必须提供 private_key_path".to_string())?;
            AuthMaterial::Key {
                path: expand_tilde(path),
                passphrase: secrets.resolve(record.passphrase_ref.as_deref())?,
            }
        }
        other => return Err(format!("不支持的 SSH 认证类型: {other}")),
    };

    let port = u16::try_from(record.port).unwrap_or(22);
    Ok(ConnectParams {
        ssh_config_id: record.id.clone(),
        name: record.name.clone(),
        host: record.host.clone(),
        port,
        username: record.username.clone(),
        auth,
        policy: KnownHostsPolicy::from_mode(&record.known_hosts_mode),
        known_hosts_path: default_known_hosts_path(),
    })
}

#[tauri::command]
pub(crate) async fn list_ssh_configs<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<SshConfig>, String> {
    let pool = sqlite_pool(&app).await?;
    list_ssh_config_records(&pool).await
}

#[tauri::command]
pub(crate) async fn get_ssh_config<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<SshConfig, String> {
    let pool = sqlite_pool(&app).await?;
    fetch_ssh_config_by_id(&pool, &id).await
}

#[tauri::command]
pub(crate) async fn create_ssh_config<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateSshConfig,
) -> Result<SshConfig, String> {
    let pool = sqlite_pool(&app).await?;
    let secrets = SecretStore::for_app(&app)?;
    create_ssh_config_with(&pool, &secrets, payload).await
}

#[tauri::command]
pub(crate) async fn update_ssh_config<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    updates: UpdateSshConfig,
) -> Result<SshConfig, String> {
    let pool = sqlite_pool(&app).await?;
    let secrets = SecretStore::for_app(&app)?;
    let updated = update_ssh_config_with(&pool, &secrets, &id, updates).await?;
    app.state::<SshPool>().inner().invalidate(&id).await;
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn delete_ssh_config<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), String> {
    let pool = sqlite_pool(&app).await?;
    let secrets = SecretStore::for_app(&app)?;
    delete_ssh_config_with(&pool, &secrets, &id).await?;
    app.state::<SshPool>().inner().invalidate(&id).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn probe_ssh_password_auth<R: Runtime>(
    app: AppHandle<R>,
    ssh_config_id: String,
) -> Result<PasswordAuthProbeResult, String> {
    let pool = sqlite_pool(&app).await?;
    let record = fetch_ssh_config_record_by_id(&pool, &ssh_config_id).await?;
    let label = ssh_config_target_host_label(&record);
    let checked_at = now_sqlite();

    if record.auth_type != "password" {
        let message = "当前配置不是密码认证，无需探测".to_string();
        write_password_probe_result(&pool, &ssh_config_id, "failed", &message, &checked_at).await?;
        return Ok(PasswordAuthProbeResult {
            ssh_config_id,
            target_host_label: label,
            supported: false,
            status: "failed".to_string(),
            message,
            checked_at,
        });
    }

    let params = resolve_connect_params(&app, &record, false)?;
    let ssh_pool = app.state::<SshPool>().inner().clone();
    ssh_pool.invalidate(&ssh_config_id).await;
    let result = ssh_pool
        .exec(
            &params,
            "printf 'noxcode-password-probe' >/dev/null",
            ExecOptions::default(),
        )
        .await;
    let (status, message) = match result {
        Ok(output) if output.success() => ("passed".to_string(), "密码认证探测通过".to_string()),
        Ok(output) => (
            "failed".to_string(),
            format!(
                "密码认证探测失败: exit={:?} {}",
                output.exit_code,
                output.stderr_lossy()
            ),
        ),
        Err(error) => ("failed".to_string(), error.to_string()),
    };
    write_password_probe_result(&pool, &ssh_config_id, &status, &message, &checked_at).await?;
    Ok(PasswordAuthProbeResult {
        ssh_config_id,
        target_host_label: label,
        supported: status == "passed",
        status,
        message,
        checked_at,
    })
}

fn parse_connection_test_output(output: &SshCommandOutput) -> (Option<String>, Option<String>) {
    let stdout = output.stdout_lossy();
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let uname = lines.get(1).map(|line| (*line).to_string());
    let remote_git_version = lines.get(3).and_then(|line| {
        if line.contains("not found") {
            None
        } else {
            Some((*line).to_string())
        }
    });
    (uname, remote_git_version)
}

#[tauri::command]
pub(crate) async fn test_ssh_connection<R: Runtime>(
    app: AppHandle<R>,
    ssh_config_id: String,
) -> Result<SshConnectionTestResult, String> {
    let pool = sqlite_pool(&app).await?;
    let record = fetch_ssh_config_record_by_id(&pool, &ssh_config_id).await?;
    let label = ssh_config_target_host_label(&record);
    let params = resolve_connect_params(&app, &record, false)?;
    let ssh_pool = app.state::<SshPool>().inner().clone();
    ssh_pool.invalidate(&ssh_config_id).await;

    let command = build_remote_shell_command(
        "echo ok && uname -a && pwd && (git --version 2>/dev/null || echo 'git: not found')",
    );
    let checked_at = now_sqlite();
    match ssh_pool
        .exec(&params, &command, ExecOptions::default())
        .await
    {
        Ok(output) if output.success() => {
            let (uname, remote_git_version) = parse_connection_test_output(&output);
            write_connection_check_result(&pool, &ssh_config_id, "passed", "连接成功", &checked_at)
                .await?;
            Ok(SshConnectionTestResult {
                ssh_config_id,
                target_host_label: label,
                ok: true,
                status: "passed".to_string(),
                message: "连接成功".to_string(),
                uname,
                remote_git_version,
                checked_at,
            })
        }
        Ok(output) => {
            let message = format!(
                "连接测试失败: exit={:?} {}",
                output.exit_code,
                output.stderr_lossy()
            );
            write_connection_check_result(&pool, &ssh_config_id, "failed", &message, &checked_at)
                .await?;
            Ok(SshConnectionTestResult {
                ssh_config_id,
                target_host_label: label,
                ok: false,
                status: "failed".to_string(),
                message,
                uname: None,
                remote_git_version: None,
                checked_at,
            })
        }
        Err(error) => {
            let message = error.to_string();
            write_connection_check_result(&pool, &ssh_config_id, "failed", &message, &checked_at)
                .await?;
            Ok(SshConnectionTestResult {
                ssh_config_id,
                target_host_label: label,
                ok: false,
                status: "failed".to_string(),
                message,
                uname: None,
                remote_git_version: None,
                checked_at,
            })
        }
    }
}

#[tauri::command]
pub(crate) fn list_ssh_config_file_hosts() -> Result<Vec<SshConfigFileHost>, String> {
    let config = load_default_ssh_config()?;
    Ok(list_hosts(&config))
}

#[tauri::command]
pub(crate) fn import_ssh_config_file_host(alias: String) -> Result<SshConfigFileImport, String> {
    let config = load_default_ssh_config()?;
    Ok(import_host(&config, &alias))
}

#[tauri::command]
pub(crate) fn resolve_ssh_host_trust<R: Runtime>(
    app: AppHandle<R>,
    prompt_id: String,
    accept: bool,
) -> Result<(), String> {
    app.state::<SshPool>()
        .inner()
        .trust()
        .resolve(&prompt_id, accept)
}
