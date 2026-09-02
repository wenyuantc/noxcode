use std::path::PathBuf;

#[cfg(test)]
use chrono::NaiveDateTime;
use chrono::Utc;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_sql::{DbInstances, DbPool};
use uuid::Uuid;

pub const DB_URL: &str = "sqlite:noxcode.db";
pub(crate) const DB_FILE_NAME: &str = "noxcode.db";
pub(crate) const DB_AUTO_IMPORT_BACKUP_PREFIX: &str = "noxcode.pre-import-backup";
pub(crate) const SQLITE_DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
#[allow(dead_code)]
pub(crate) const EXECUTION_TARGET_LOCAL: &str = "local";
#[allow(dead_code)]
pub(crate) const EXECUTION_TARGET_SSH: &str = "ssh";
#[allow(dead_code)]
pub(crate) const WORKSPACE_TYPE_LOCAL: &str = "local";
#[allow(dead_code)]
pub(crate) const WORKSPACE_TYPE_SSH: &str = "ssh";

pub(crate) struct DatabaseMigrationStatus {
    pub(crate) applied_count: i64,
    pub(crate) current_version: Option<i64>,
    pub(crate) current_description: Option<String>,
}

pub(crate) fn now_sqlite() -> String {
    Utc::now().format(SQLITE_DATETIME_FORMAT).to_string()
}

pub(crate) fn database_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(DB_FILE_NAME))
}

pub(crate) async fn sqlite_pool<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, String> {
    let instances = app.state::<DbInstances>();
    let instances = instances.0.read().await;
    let db = instances
        .get(DB_URL)
        .ok_or_else(|| format!("Database {DB_URL} is not loaded"))?;

    let DbPool::Sqlite(pool) = db;
    Ok(pool.clone())
}

pub(crate) fn resolve_user_file_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("路径不能为空".to_string());
    }

    let raw_path = PathBuf::from(trimmed);
    if raw_path.is_absolute() {
        Ok(raw_path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(raw_path))
            .map_err(|error| format!("无法解析当前工作目录: {error}"))
    }
}

pub(crate) fn resolve_existing_file_path(path: &str) -> Result<PathBuf, String> {
    let resolved = resolve_user_file_path(path)?;
    let canonical = resolved
        .canonicalize()
        .map_err(|error| format!("文件不存在或不可访问: {error}"))?;

    if !canonical.is_file() {
        return Err(format!("路径 {} 不是文件", canonical.display()));
    }

    Ok(canonical)
}

#[allow(dead_code)]
pub(crate) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[allow(dead_code)]
pub(crate) fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
fn parse_sqlite_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, SQLITE_DATETIME_FORMAT).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_optional_text_trims_and_drops_empty() {
        assert_eq!(normalize_optional_text(None), None);
        assert_eq!(normalize_optional_text(Some("")), None);
        assert_eq!(normalize_optional_text(Some("   ")), None);
        assert_eq!(
            normalize_optional_text(Some("  hello  ")),
            Some("hello".to_string())
        );
    }

    #[test]
    fn now_sqlite_round_trips_through_parser() {
        let value = now_sqlite();
        assert!(
            parse_sqlite_datetime(&value).is_some(),
            "unparsable sqlite datetime: {value}"
        );
    }
}
