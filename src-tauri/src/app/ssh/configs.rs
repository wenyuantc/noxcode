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

    let connection_changed = host != current.host
        || port != current.port
        || username != current.username
        || auth_type != current.auth_type
        || private_key_path != current.private_key_path
        || password_ref != current.password_ref
        || passphrase_ref != current.passphrase_ref
        || known_hosts_mode != current.known_hosts_mode
        || algorithms_json != current.algorithms_json;
    let password_probe_needs_reset = connection_changed || auth_type != "password";

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
            password_probe_checked_at = CASE WHEN $11 THEN NULL ELSE password_probe_checked_at END,
            password_probe_status = CASE WHEN $11 THEN NULL ELSE password_probe_status END,
            password_probe_message = CASE WHEN $11 THEN NULL ELSE password_probe_message END,
            last_checked_at = CASE WHEN $12 THEN NULL ELSE last_checked_at END,
            last_check_status = CASE WHEN $12 THEN NULL ELSE last_check_status END,
            last_check_message = CASE WHEN $12 THEN NULL ELSE last_check_message END,
            algorithms_json = $13,
            updated_at = $14
        WHERE id = $1
            AND host IS $15 AND port IS $16 AND username IS $17 AND auth_type IS $18
            AND private_key_path IS $19 AND password_ref IS $20 AND passphrase_ref IS $21
            AND known_hosts_mode IS $22 AND algorithms_json IS $23
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
    .bind(password_probe_needs_reset)
    .bind(connection_changed)
    .bind(&algorithms_json)
    .bind(now_sqlite())
    .bind(&current.host)
    .bind(current.port)
    .bind(&current.username)
    .bind(&current.auth_type)
    .bind(&current.private_key_path)
    .bind(&current.password_ref)
    .bind(&current.passphrase_ref)
    .bind(&current.known_hosts_mode)
    .bind(&current.algorithms_json)
    .execute(pool)
    .await
    .map_err(|error| format!("Failed to update ssh config: {error}"))
    .and_then(|result| {
        if result.rows_affected() == 0 {
            Err("SSH 配置已变更或删除，请重新加载后保存".to_string())
        } else {
            Ok(())
        }
    });

    if let Err(error) = update_result {
        if created_password_ref.is_some() {
            let _ = secrets.delete(password_ref.as_deref());
        }
        if created_passphrase_ref.is_some() {
            let _ = secrets.delete(passphrase_ref.as_deref());
        }
        return Err(error);
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
    record: &SshConfigRecord,
    status: &str,
    message: &str,
    checked_at: &str,
) -> Result<(), String> {
    write_verification_result(pool, record, status, message, checked_at, false).await
}

pub(crate) async fn write_connection_check_result(
    pool: &SqlitePool,
    record: &SshConfigRecord,
    status: &str,
    message: &str,
    checked_at: &str,
) -> Result<(), String> {
    write_verification_result(pool, record, status, message, checked_at, true).await
}

async fn write_verification_result(
    pool: &SqlitePool,
    record: &SshConfigRecord,
    status: &str,
    message: &str,
    checked_at: &str,
    connection_check: bool,
) -> Result<(), String> {
    // 连接测试已完成密码认证和远端 exec，原子更新两组状态。
    // 只允许本次测试使用的配置写回，不能把旧凭据的成功结果授予新配置。
    let result = sqlx::query(
        r#"
        UPDATE ssh_configs
        SET last_checked_at = CASE WHEN $6 THEN $2 ELSE last_checked_at END,
            last_check_status = CASE WHEN $6 THEN $3 ELSE last_check_status END,
            last_check_message = CASE WHEN $6 THEN $4 ELSE last_check_message END,
            password_probe_checked_at = CASE WHEN auth_type = 'password' THEN $2 ELSE NULL END,
            password_probe_status = CASE WHEN auth_type = 'password' THEN $3 ELSE NULL END,
            password_probe_message = CASE WHEN auth_type = 'password' THEN $4 ELSE NULL END,
            updated_at = $5
        WHERE id = $1
            AND host IS $7 AND port IS $8 AND username IS $9 AND auth_type IS $10
            AND private_key_path IS $11 AND password_ref IS $12 AND passphrase_ref IS $13
            AND known_hosts_mode IS $14 AND algorithms_json IS $15
        "#,
    )
    .bind(&record.id)
    .bind(checked_at)
    .bind(status)
    .bind(message)
    .bind(now_sqlite())
    .bind(connection_check)
    .bind(&record.host)
    .bind(record.port)
    .bind(&record.username)
    .bind(&record.auth_type)
    .bind(&record.private_key_path)
    .bind(&record.password_ref)
    .bind(&record.passphrase_ref)
    .bind(&record.known_hosts_mode)
    .bind(&record.algorithms_json)
    .execute(pool)
    .await
    .map_err(|error| format!("更新 SSH 验证结果失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err("SSH 配置已变更或删除，请重新测试连接".to_string());
    }
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

    #[tokio::test]
    async fn connection_check_atomically_updates_password_verification_only_for_password() {
        let pool = setup_migrated_pool().await;
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretStore::in_memory(dir.path().to_path_buf());
        for auth_type in ["password", "key"] {
            let created = create_ssh_config_with(&pool, &secrets, sample_create(auth_type))
                .await
                .unwrap();
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .unwrap();
            for status in ["passed", "failed"] {
                write_connection_check_result(
                    &pool,
                    &record,
                    status,
                    "result",
                    "2026-09-07 12:00:00",
                )
                .await
                .unwrap();
                let config = fetch_ssh_config_by_id(&pool, &created.id).await.unwrap();
                assert_eq!(config.last_check_status.as_deref(), Some(status));
                if auth_type == "password" {
                    assert_eq!(config.password_probe_status, config.last_check_status);
                    assert_eq!(config.password_probe_checked_at, config.last_checked_at);
                    assert_eq!(config.password_probe_message, config.last_check_message);
                    assert_eq!(config.password_execution_allowed, status == "passed");
                } else {
                    assert!(config.password_probe_status.is_none());
                    assert!(config.password_probe_checked_at.is_none());
                    assert!(config.password_probe_message.is_none());
                }
            }
        }
    }

    #[tokio::test]
    async fn unchanged_full_form_and_name_only_save_preserve_verification() {
        let pool = setup_migrated_pool().await;
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretStore::in_memory(dir.path().to_path_buf());
        let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
            .await
            .unwrap();
        let record = fetch_ssh_config_record_by_id(&pool, &created.id)
            .await
            .unwrap();
        write_connection_check_result(&pool, &record, "passed", "ok", "2026-09-07 12:00:00")
            .await
            .unwrap();
        for updates in [
            serde_json::json!({"name": "renamed"}),
            serde_json::json!({
                "name": "renamed", "host": " example.test ", "port": 22,
                "username": " deploy ", "auth_type": "password", "private_key_path": null,
                "known_hosts_mode": "accept-new", "algorithms": null
            }),
        ] {
            let config = update_ssh_config_with(
                &pool,
                &secrets,
                &created.id,
                serde_json::from_value(updates).unwrap(),
            )
            .await
            .unwrap();
            assert!(config.password_execution_allowed);
            assert_eq!(config.last_check_status.as_deref(), Some("passed"));
            assert_eq!(
                config.password_probe_checked_at.as_deref(),
                Some("2026-09-07 12:00:00")
            );
        }
        // 仅改名称不改变认证配置，测试开始时取得的快照仍可写回。
        write_password_probe_result(&pool, &record, "passed", "probe", "2026-09-07 12:00:01")
            .await
            .unwrap();
        let config = fetch_ssh_config_by_id(&pool, &created.id).await.unwrap();
        assert_eq!(config.password_probe_message.as_deref(), Some("probe"));
        assert_eq!(config.last_check_message.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn connection_changes_reset_verification_and_reject_stale_test_results() {
        let pool = setup_migrated_pool().await;
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretStore::in_memory(dir.path().to_path_buf());
        for updates in [
            serde_json::json!({"host": "other.test"}),
            serde_json::json!({"port": 2222}),
            serde_json::json!({"username": "other"}),
            serde_json::json!({"password": "new-password"}),
            serde_json::json!({"password": null}),
            serde_json::json!({"auth_type": "key", "private_key_path": "~/.ssh/id_ed25519"}),
            serde_json::json!({"known_hosts_mode": "strict"}),
            serde_json::json!({"algorithms": {"cipher": ["aes256-ctr"]}}),
        ] {
            let created = create_ssh_config_with(&pool, &secrets, sample_create("password"))
                .await
                .unwrap();
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .unwrap();
            write_connection_check_result(&pool, &record, "passed", "ok", "2026-09-07 12:00:00")
                .await
                .unwrap();
            let updated = update_ssh_config_with(
                &pool,
                &secrets,
                &created.id,
                serde_json::from_value(updates.clone()).unwrap(),
            )
            .await
            .unwrap();
            assert!(updated.last_check_status.is_none(), "{updates}");
            assert!(updated.last_checked_at.is_none(), "{updates}");
            assert!(updated.last_check_message.is_none(), "{updates}");
            assert!(updated.password_probe_status.is_none(), "{updates}");
            assert!(updated.password_probe_checked_at.is_none(), "{updates}");
            assert!(updated.password_probe_message.is_none(), "{updates}");
            assert!(!updated.password_execution_allowed, "{updates}");
            for status in ["passed", "failed"] {
                assert!(
                    write_connection_check_result(
                        &pool,
                        &record,
                        status,
                        "stale",
                        "2026-09-07 12:00:01"
                    )
                    .await
                    .unwrap_err()
                    .contains("已变更"),
                    "{updates}"
                );
                assert!(
                    write_password_probe_result(
                        &pool,
                        &record,
                        status,
                        "stale",
                        "2026-09-07 12:00:01"
                    )
                    .await
                    .unwrap_err()
                    .contains("已变更"),
                    "{updates}"
                );
            }
            let unchanged = fetch_ssh_config_by_id(&pool, &created.id).await.unwrap();
            assert!(unchanged.password_probe_status.is_none());
            assert!(unchanged.last_check_status.is_none());
        }
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
            let record = fetch_ssh_config_record_by_id(&pool, &created.id)
                .await
                .unwrap();
            write_password_probe_result(&pool, &record, "passed", "ok", "2026-01-01 00:00:00")
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
