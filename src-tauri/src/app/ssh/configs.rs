use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::app::secret_store::SecretStore;
use crate::app::shared::{new_id, normalize_optional_text, now_sqlite};
use crate::db::models::{CreateSshConfig, SshConfig, SshConfigRecord, UpdateSshConfig};

use super::algorithms::{validate, SshAlgorithms};

pub(crate) fn normalize_ssh_auth_type(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("key") => Ok("key".to_string()),
        Some("password") => Ok("password".to_string()),
        Some(other) => Err(format!("不支持的 SSH 认证类型: {other}")),
    }
}

pub(crate) fn normalize_known_hosts_mode(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some("strict") => "strict".to_string(),
        Some("off") => "off".to_string(),
        Some("ask") => "ask".to_string(),
        _ => "accept-new".to_string(),
    }
}

pub(crate) fn ssh_config_target_host_label(record: &SshConfigRecord) -> String {
    format!("{}@{}:{}", record.username, record.host, record.port)
}

fn normalize_ssh_algorithms(algorithms: Option<SshAlgorithms>) -> Result<Option<String>, String> {
    let Some(mut algorithms) = algorithms else {
        return Ok(None);
    };
    for names in [
        &mut algorithms.kex,
        &mut algorithms.host_key,
        &mut algorithms.cipher,
        &mut algorithms.mac,
    ] {
        let mut normalized = Vec::new();
        for name in std::mem::take(names) {
            let name = name.trim();
            if !name.is_empty() && !normalized.iter().any(|item| item == name) {
                normalized.push(name.to_string());
            }
        }
        *names = normalized;
    }
    if algorithms.is_empty() {
        return Ok(None);
    }
    validate(&algorithms)?;
    serde_json::to_string(&algorithms)
        .map(Some)
        .map_err(|error| format!("序列化 SSH 算法配置失败: {error}"))
}

fn record_to_config(record: SshConfigRecord) -> Result<SshConfig, String> {
    let algorithms = match record.algorithms_json.as_deref() {
        Some(json) => {
            let algorithms: SshAlgorithms = serde_json::from_str(json)
                .map_err(|error| format!("解析 SSH 算法配置失败: {error}"))?;
            validate(&algorithms)?;
            Some(algorithms)
        }
        None => None,
    };
    let mut config = SshConfig::from(record);
    config.algorithms = algorithms;
    Ok(config)
}

pub(crate) async fn fetch_ssh_config_record_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<SshConfigRecord, String> {
    sqlx::query_as::<_, SshConfigRecord>("SELECT * FROM ssh_configs WHERE id = $1 LIMIT 1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("Failed to load ssh config: {error}"))?
        .ok_or_else(|| format!("SSH 配置不存在: {id}"))
}

pub(crate) async fn fetch_ssh_config_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<SshConfig, String> {
    fetch_ssh_config_record_by_id(pool, id)
        .await
        .and_then(record_to_config)
}

pub(crate) async fn list_ssh_config_records(pool: &SqlitePool) -> Result<Vec<SshConfig>, String> {
    let records = sqlx::query_as::<_, SshConfigRecord>(
        "SELECT * FROM ssh_configs ORDER BY updated_at DESC, created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Failed to list ssh configs: {error}"))?;

    records.into_iter().map(record_to_config).collect()
}

async fn collect_all_ssh_secret_refs(pool: &SqlitePool) -> Result<HashSet<String>, String> {
    let rows = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT password_ref, passphrase_ref FROM ssh_configs",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Failed to load ssh secret refs: {error}"))?;

    let mut refs = HashSet::new();
    for (password_ref, passphrase_ref) in rows {
        if let Some(password_ref) = password_ref {
            refs.insert(password_ref);
        }
        if let Some(passphrase_ref) = passphrase_ref {
            refs.insert(passphrase_ref);
        }
    }
    Ok(refs)
}

async fn sweep_ssh_secret_store(pool: &SqlitePool, secrets: &SecretStore) -> Result<usize, String> {
    let active_refs = collect_all_ssh_secret_refs(pool).await?;
    secrets.sweep_orphans(&active_refs)
}

pub(crate) async fn create_ssh_config_with(
    pool: &SqlitePool,
    secrets: &SecretStore,
    payload: CreateSshConfig,
) -> Result<SshConfig, String> {
    let id = new_id();
    let name = payload.name.trim().to_string();
    let host = payload.host.trim().to_string();
    let username = payload.username.trim().to_string();
    let auth_type = normalize_ssh_auth_type(Some(&payload.auth_type))?;
    let private_key_path = normalize_optional_text(payload.private_key_path.as_deref());
    let known_hosts_mode = normalize_known_hosts_mode(payload.known_hosts_mode.as_deref());
    let algorithms_json = normalize_ssh_algorithms(payload.algorithms)?;
    let port = payload.port.unwrap_or(22).clamp(1, 65535);

    if name.is_empty() || host.is_empty() || username.is_empty() {
        return Err("SSH 配置名称、主机和用户名不能为空".to_string());
    }
    if auth_type == "key" && private_key_path.is_none() {
        return Err("密钥认证必须提供 private_key_path".to_string());
    }

    let password_ref = secrets.store(payload.password.as_deref(), None)?;
    let passphrase_ref = secrets.store(payload.passphrase.as_deref(), None)?;

    let insert_result = sqlx::query(
        r#"
        INSERT INTO ssh_configs (
            id,
            name,
            host,
            port,
            username,
            auth_type,
            private_key_path,
            password_ref,
            passphrase_ref,
            known_hosts_mode,
            algorithms_json,
            created_at,
            updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
    )
    .bind(&id)
    .bind(&name)
    .bind(&host)
    .bind(port)
    .bind(&username)
    .bind(&auth_type)
    .bind(&private_key_path)
    .bind(&password_ref)
    .bind(&passphrase_ref)
    .bind(&known_hosts_mode)
    .bind(&algorithms_json)
    .bind(now_sqlite())
    .bind(now_sqlite())
    .execute(pool)
    .await;

    if let Err(error) = insert_result {
        let _ = secrets.delete(password_ref.as_deref());
        let _ = secrets.delete(passphrase_ref.as_deref());
        return Err(format!("Failed to create ssh config: {error}"));
    }

    let _ = sweep_ssh_secret_store(pool, secrets).await;
    fetch_ssh_config_by_id(pool, &id).await
}

pub(crate) async fn update_ssh_config_with(
    pool: &SqlitePool,
    secrets: &SecretStore,
    id: &str,
    updates: UpdateSshConfig,
) -> Result<SshConfig, String> {
    let current = fetch_ssh_config_record_by_id(pool, id).await?;

    let host_changed = updates.host.is_some();
    let port_changed = updates.port.is_some();
    let username_changed = updates.username.is_some();

    let name = updates
        .name
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.name.clone());
    let host = updates
        .host
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.host.clone());
    let username = updates
        .username
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| current.username.clone());
    let auth_type =
        normalize_ssh_auth_type(updates.auth_type.as_deref().or(Some(&current.auth_type)))?;
    let private_key_path = match updates.private_key_path {
        Some(Some(value)) => normalize_optional_text(Some(&value)),
        Some(None) => None,
        None => current.private_key_path.clone(),
    };
    let known_hosts_mode = updates
        .known_hosts_mode
        .map(|value| normalize_known_hosts_mode(Some(&value)))
        .unwrap_or_else(|| current.known_hosts_mode.clone());
    let algorithms_json = match updates.algorithms {
        Some(algorithms) => normalize_ssh_algorithms(algorithms)?,
        None => current.algorithms_json.clone(),
    };
    let port = updates.port.unwrap_or(current.port).clamp(1, 65535);

    if name.is_empty() || host.is_empty() || username.is_empty() {
        return Err("SSH 配置名称、主机和用户名不能为空".to_string());
    }
    if auth_type == "key" && private_key_path.is_none() {
        return Err("密钥认证必须提供 private_key_path".to_string());
    }

    let mut created_password_ref: Option<String> = None;
    let mut created_passphrase_ref: Option<String> = None;

    let password_ref = if auth_type == "password" {
        match updates.password {
            Some(Some(ref value)) => {
                let next = secrets.store(Some(value), None)?;
                created_password_ref = next.clone();
                next
            }
            Some(None) => None,
            None => current.password_ref.clone(),
        }
    } else {
        None
    };
    let passphrase_ref = if auth_type == "key" {
        match updates.passphrase {
            Some(Some(ref value)) => {
                let next = secrets.store(Some(value), None)?;
                created_passphrase_ref = next.clone();
                next
            }
            Some(None) => None,
            None => current.passphrase_ref.clone(),
        }
    } else {
        None
    };

    let password_probe_needs_reset = auth_type != "password"
        || current.auth_type != "password"
        || updates.password.is_some()
        || host_changed
        || port_changed
        || username_changed;
    let password_probe_status = if auth_type == "password" && !password_probe_needs_reset {
        current.password_probe_status.clone()
    } else {
        None
    };
    let password_probe_checked_at = if auth_type == "password" && !password_probe_needs_reset {
        current.password_probe_checked_at.clone()
    } else {
        None
    };
    let password_probe_message = if auth_type == "password" && !password_probe_needs_reset {
        current.password_probe_message.clone()
    } else {
        None
    };

    let update_result = sqlx::query(
        r#"
        UPDATE ssh_configs
        SET name = $2,
            host = $3,
            port = $4,
            username = $5,
            auth_type = $6,
            private_key_path = $7,
            password_ref = $8,
            passphrase_ref = $9,
            known_hosts_mode = $10,
            password_probe_checked_at = $11,
            password_probe_status = $12,
            password_probe_message = $13,
            algorithms_json = $14,
            updated_at = $15
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(&name)
    .bind(&host)
    .bind(port)
    .bind(&username)
    .bind(&auth_type)
    .bind(&private_key_path)
    .bind(&password_ref)
    .bind(&passphrase_ref)
    .bind(&known_hosts_mode)
    .bind(&password_probe_checked_at)
    .bind(&password_probe_status)
    .bind(&password_probe_message)
    .bind(&algorithms_json)
    .bind(now_sqlite())
    .execute(pool)
    .await;

    if let Err(error) = update_result {
        if created_password_ref.is_some() {
            let _ = secrets.delete(password_ref.as_deref());
        }
        if created_passphrase_ref.is_some() {
            let _ = secrets.delete(passphrase_ref.as_deref());
        }
        return Err(format!("Failed to update ssh config: {error}"));
    }

    if current.password_ref != password_ref {
        secrets.delete(current.password_ref.as_deref())?;
    }
    if current.passphrase_ref != passphrase_ref {
        secrets.delete(current.passphrase_ref.as_deref())?;
    }

    sweep_ssh_secret_store(pool, secrets).await?;
    fetch_ssh_config_by_id(pool, id).await
}

pub(crate) async fn delete_ssh_config_with(
    pool: &SqlitePool,
    secrets: &SecretStore,
    id: &str,
) -> Result<(), String> {
    let current = fetch_ssh_config_record_by_id(pool, id).await?;
    let usage_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workspaces WHERE ssh_config_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(|error| format!("Failed to check ssh config usage: {error}"))?;
    if usage_count > 0 {
        return Err("当前 SSH 配置仍被工作区引用，不能删除".to_string());
    }

    sqlx::query("DELETE FROM ssh_configs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("Failed to delete ssh config: {error}"))?;

    secrets.delete(current.password_ref.as_deref())?;
    secrets.delete(current.passphrase_ref.as_deref())?;
    sweep_ssh_secret_store(pool, secrets).await?;
    Ok(())
}

pub(crate) async fn write_password_probe_result(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    message: &str,
    checked_at: &str,
) -> Result<(), String> {
    sqlx::query(
        r#"
        UPDATE ssh_configs
        SET password_probe_checked_at = $2,
            password_probe_status = $3,
            password_probe_message = $4,
            updated_at = $5
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(checked_at)
    .bind(status)
    .bind(message)
    .bind(now_sqlite())
    .execute(pool)
    .await
    .map_err(|error| format!("Failed to update password probe: {error}"))?;
    Ok(())
}

pub(crate) async fn write_connection_check_result(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    message: &str,
    checked_at: &str,
) -> Result<(), String> {
    sqlx::query(
        r#"
        UPDATE ssh_configs
        SET last_checked_at = $2,
            last_check_status = $3,
            last_check_message = $4,
            updated_at = $5
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(checked_at)
    .bind(status)
    .bind(message)
    .bind(now_sqlite())
    .execute(pool)
    .await
    .map_err(|error| format!("Failed to update connection check: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::secret_store::SecretStore;
    use crate::db::test_support::setup_migrated_pool;

    fn temp_secret_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noxcode-ssh-config-secrets-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create secret dir");
        dir
    }

    fn sample_create(auth_type: &str) -> CreateSshConfig {
        CreateSshConfig {
            name: "demo".to_string(),
            host: "example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            auth_type: auth_type.to_string(),
            private_key_path: if auth_type == "key" {
                Some("~/.ssh/id_ed25519".to_string())
            } else {
                None
            },
            password: if auth_type == "password" {
                Some("s3cret".to_string())
            } else {
                None
            },
            passphrase: None,
            known_hosts_mode: Some("accept-new".to_string()),
            algorithms: None,
        }
    }

    #[test]
    fn normalize_known_hosts_mode_four_values() {
        assert_eq!(normalize_known_hosts_mode(Some("strict")), "strict");
        assert_eq!(normalize_known_hosts_mode(Some("off")), "off");
        assert_eq!(normalize_known_hosts_mode(Some("ask")), "ask");
        assert_eq!(normalize_known_hosts_mode(Some("accept-new")), "accept-new");
        assert_eq!(normalize_known_hosts_mode(Some("weird")), "accept-new");
        assert_eq!(normalize_known_hosts_mode(None), "accept-new");
    }

    #[test]
    fn create_stores_password_ref() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = temp_secret_dir();
            let secrets = SecretStore::in_memory(dir.clone());
            let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
                .await
                .expect("create");
            assert!(created.password_configured);
            assert!(!created.passphrase_configured);
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .expect("record");
            let value = secrets
                .resolve(record.password_ref.as_deref())
                .expect("resolve")
                .expect("password");
            assert_eq!(value, "s3cret");
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn update_replaces_password_and_deletes_old_ref() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = temp_secret_dir();
            let secrets = SecretStore::in_memory(dir.clone());
            let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
                .await
                .expect("create");
            let old_record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .expect("old record");
            let old_ref = old_record.password_ref.clone().expect("old ref");

            let updated = update_ssh_config_with(
                &pool,
                &secrets,
                &created.id,
                UpdateSshConfig {
                    name: None,
                    host: None,
                    port: None,
                    username: None,
                    auth_type: None,
                    private_key_path: None,
                    password: Some(Some("newer".to_string())),
                    passphrase: None,
                    known_hosts_mode: None,
                    algorithms: None,
                },
            )
            .await
            .expect("update");
            assert!(updated.password_configured);

            let new_record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .expect("new record");
            assert_ne!(new_record.password_ref.as_deref(), Some(old_ref.as_str()));
            assert!(secrets
                .resolve(Some(&old_ref))
                .expect("old resolve")
                .is_none());
            assert_eq!(
                secrets
                    .resolve(new_record.password_ref.as_deref())
                    .expect("new resolve")
                    .as_deref(),
                Some("newer")
            );
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn switching_auth_type_clears_other_secret_and_resets_probe() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = temp_secret_dir();
            let secrets = SecretStore::in_memory(dir.clone());
            let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
                .await
                .expect("create");
            write_password_probe_result(&pool, &created.id, "passed", "ok", "2026-01-01 00:00:00")
                .await
                .expect("probe");

            let updated = update_ssh_config_with(
                &pool,
                &secrets,
                &created.id,
                UpdateSshConfig {
                    name: None,
                    host: None,
                    port: None,
                    username: None,
                    auth_type: Some("key".to_string()),
                    private_key_path: Some(Some("~/.ssh/id_ed25519".to_string())),
                    password: None,
                    passphrase: Some(Some("phrase".to_string())),
                    known_hosts_mode: None,
                    algorithms: None,
                },
            )
            .await
            .expect("update");

            assert!(!updated.password_configured);
            assert!(updated.passphrase_configured);
            assert!(updated.password_probe_status.is_none());
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .expect("record");
            assert!(record.password_ref.is_none());
            assert!(record.passphrase_ref.is_some());
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn delete_clears_refs_and_rejects_workspace_usage() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = temp_secret_dir();
            let secrets = SecretStore::in_memory(dir.clone());
            let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
                .await
                .expect("create");
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .expect("record");
            let password_ref = record.password_ref.clone().expect("ref");

            sqlx::query(
                "INSERT INTO workspaces (id, name, workspace_type, ssh_config_id, remote_repo_path) VALUES ('ws-1', 'remote', 'ssh', $1, '/repo')",
            )
            .bind(&created.id)
            .execute(&pool)
            .await
            .expect("insert workspace");

            let error = delete_ssh_config_with(&pool, &secrets, &created.id)
                .await
                .expect_err("must refuse");
            assert!(error.contains("工作区"));

            sqlx::query("DELETE FROM workspaces WHERE id = 'ws-1'")
                .execute(&pool)
                .await
                .expect("delete workspace");

            delete_ssh_config_with(&pool, &secrets, &created.id)
                .await
                .expect("delete config");
            assert!(secrets
                .resolve(Some(&password_ref))
                .expect("resolve")
                .is_none());
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn algorithms_roundtrip_and_can_be_cleared() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let dir = temp_secret_dir();
            let secrets = SecretStore::in_memory(dir.clone());
            let mut payload = sample_create("password");
            let algorithms = SshAlgorithms {
                kex: vec!["curve25519-sha256".to_string()],
                host_key: vec!["ssh-ed25519".to_string()],
                cipher: vec!["aes256-ctr".to_string()],
                mac: vec!["hmac-sha2-256".to_string()],
            };
            payload.algorithms = Some(algorithms.clone());
            let created = create_ssh_config_with(&pool, &secrets, payload)
                .await
                .expect("create");
            assert_eq!(created.algorithms, Some(algorithms));

            let cleared = update_ssh_config_with(
                &pool,
                &secrets,
                &created.id,
                UpdateSshConfig {
                    name: None,
                    host: None,
                    port: None,
                    username: None,
                    auth_type: None,
                    private_key_path: None,
                    password: None,
                    passphrase: None,
                    known_hosts_mode: None,
                    algorithms: Some(None),
                },
            )
            .await
            .expect("clear algorithms");
            assert!(cleared.algorithms.is_none());
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}
