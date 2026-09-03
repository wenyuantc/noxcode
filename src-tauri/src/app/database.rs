use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::{
    migrate::{Migration as SqlxMigration, MigrationType as SqlxMigrationType, Migrator},
    SqlitePool,
};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::app::shared::{
    database_path, now_sqlite, resolve_existing_file_path, resolve_user_file_path, sqlite_pool,
    DatabaseMigrationStatus, DB_AUTO_IMPORT_BACKUP_PREFIX,
};
use crate::db::models::{AppHealthCheck, DatabaseBackupResult, DatabaseRestoreResult};
use crate::git::preflight;

pub(crate) async fn fetch_database_migration_status(
    pool: &SqlitePool,
) -> Result<DatabaseMigrationStatus, String> {
    let (applied_count, current_version, current_description) =
        sqlx::query_as::<_, (i64, Option<i64>, Option<String>)>(
            r#"
            SELECT
                COUNT(*) AS applied_count,
                MAX(version) AS current_version,
                (
                    SELECT description
                    FROM _sqlx_migrations
                    WHERE success = 1
                    ORDER BY version DESC
                    LIMIT 1
                ) AS latest_description
            FROM _sqlx_migrations
            WHERE success = 1
            "#,
        )
        .fetch_one(pool)
        .await
        .map_err(|error| format!("Failed to fetch migration status: {error}"))?;

    Ok(DatabaseMigrationStatus {
        applied_count,
        current_version,
        current_description,
    })
}

fn filesystem_safe_timestamp() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

fn auto_import_backup_sql_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法解析应用配置目录: {error}"))?;

    Ok(dir.join(format!(
        "{}-{}.sql",
        DB_AUTO_IMPORT_BACKUP_PREFIX,
        filesystem_safe_timestamp()
    )))
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(crate) fn ensure_statement_terminated(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.ends_with(';') {
        trimmed.to_string()
    } else {
        format!("{trimmed};")
    }
}

fn append_sql_statement(script: &mut String, sql: &str) {
    let statement = ensure_statement_terminated(sql);
    if !statement.is_empty() {
        script.push_str(&statement);
        script.push_str("\n\n");
    }
}

pub(crate) fn build_current_migrator() -> Migrator {
    let migrations = crate::db::migrations::get_all_migrations()
        .into_iter()
        .filter_map(|migration| match migration.kind {
            tauri_plugin_sql::MigrationKind::Up => Some(SqlxMigration::new(
                migration.version,
                Cow::Borrowed(migration.description),
                SqlxMigrationType::ReversibleUp,
                Cow::Borrowed(migration.sql),
                false,
            )),
            tauri_plugin_sql::MigrationKind::Down => None,
        })
        .collect::<Vec<_>>();

    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
}

async fn fetch_schema_names(pool: &SqlitePool, object_type: &str) -> Result<Vec<String>, String> {
    let query = if object_type == "table" {
        "SELECT name FROM sqlite_master WHERE type = $1 AND name NOT LIKE 'sqlite_%' ORDER BY CASE WHEN name = '_sqlx_migrations' THEN 0 ELSE 1 END, name"
    } else {
        "SELECT name FROM sqlite_master WHERE type = $1 AND name NOT LIKE 'sqlite_%' ORDER BY name"
    };

    sqlx::query_scalar::<_, String>(query)
        .bind(object_type)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("读取数据库对象列表失败（{object_type}）: {error}"))
}

async fn fetch_schema_sql(pool: &SqlitePool, object_type: &str) -> Result<Vec<String>, String> {
    let query = if object_type == "table" {
        "SELECT sql FROM sqlite_master WHERE type = $1 AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY CASE WHEN name = '_sqlx_migrations' THEN 0 ELSE 1 END, name"
    } else {
        "SELECT sql FROM sqlite_master WHERE type = $1 AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name"
    };

    sqlx::query_scalar::<_, String>(query)
        .bind(object_type)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("读取数据库对象定义失败（{object_type}）: {error}"))
}

async fn build_table_insert_statements(
    pool: &SqlitePool,
    table_name: &str,
) -> Result<Vec<String>, String> {
    let column_query = format!(
        "SELECT name FROM pragma_table_info({}) ORDER BY cid",
        sql_string_literal(table_name)
    );
    let columns = sqlx::query_scalar::<_, String>(&column_query)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("读取表 {table_name} 的列信息失败: {error}"))?;

    if columns.is_empty() {
        return Ok(Vec::new());
    }

    let table_ident = sql_identifier(table_name);
    let column_list = columns
        .iter()
        .map(|column| sql_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_prefix = format!("INSERT INTO {table_ident} ({column_list}) VALUES (");
    let values_expr = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let quoted_column = sql_identifier(column);
            if index == 0 {
                format!("quote({quoted_column})")
            } else {
                format!(" || ',' || quote({quoted_column})")
            }
        })
        .collect::<String>();
    let row_query = format!(
        "SELECT {} || {} || ');' FROM {}",
        sql_string_literal(&insert_prefix),
        values_expr,
        table_ident
    );

    sqlx::query_scalar::<_, String>(&row_query)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("导出表 {table_name} 的数据失败: {error}"))
}

async fn build_sql_backup_script(pool: SqlitePool) -> Result<String, String> {
    let migration_status = fetch_database_migration_status(&pool).await.ok();
    let mut script = String::new();

    writeln!(&mut script, "-- noxcode SQL backup").ok();
    writeln!(&mut script, "-- created_at: {}", now_sqlite()).ok();
    if let Some(version) = migration_status
        .as_ref()
        .and_then(|status| status.current_version)
    {
        writeln!(&mut script, "-- database_version: {version}").ok();
    }
    script.push('\n');

    for sql in fetch_schema_sql(&pool, "table").await? {
        append_sql_statement(&mut script, &sql);
    }

    for table_name in fetch_schema_names(&pool, "table").await? {
        let row_statements = build_table_insert_statements(&pool, &table_name).await?;
        if !row_statements.is_empty() {
            for statement in row_statements {
                script.push_str(&statement);
                script.push('\n');
            }
            script.push('\n');
        }
    }

    for sql in fetch_schema_sql(&pool, "index").await? {
        append_sql_statement(&mut script, &sql);
    }

    for sql in fetch_schema_sql(&pool, "view").await? {
        append_sql_statement(&mut script, &sql);
    }

    for sql in fetch_schema_sql(&pool, "trigger").await? {
        append_sql_statement(&mut script, &sql);
    }

    Ok(script)
}

fn write_sql_backup_file(path: &Path, script: &str) -> Result<(), String> {
    if script.trim().is_empty() {
        return Err("SQL 备份内容为空，已中止写入".to_string());
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("无法解析目标目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建目录 {}: {error}", parent.display()))?;
    fs::write(path, script)
        .map_err(|error| format!("写入 SQL 备份文件失败 {}: {error}", path.display()))
}

pub(crate) fn sanitize_sql_backup_script(script: &str) -> String {
    let script = script.trim_start_matches('\u{feff}');
    let mut normalized = String::new();

    for line in script.lines() {
        let trimmed = line.trim();
        let upper = trimmed.trim_end_matches(';').trim().to_ascii_uppercase();
        let skip = matches!(
            upper.as_str(),
            "BEGIN TRANSACTION"
                | "BEGIN IMMEDIATE"
                | "BEGIN EXCLUSIVE"
                | "COMMIT"
                | "ROLLBACK"
                | "PRAGMA FOREIGN_KEYS=OFF"
                | "PRAGMA FOREIGN_KEYS = OFF"
                | "PRAGMA FOREIGN_KEYS=ON"
                | "PRAGMA FOREIGN_KEYS = ON"
        );

        if !skip {
            normalized.push_str(line);
            normalized.push('\n');
        }
    }

    normalized
}

async fn ensure_integrity_on_pool(pool: SqlitePool) -> Result<(), String> {
    let integrity_result = sqlx::query_scalar::<_, String>("PRAGMA integrity_check(1)")
        .fetch_all(&pool)
        .await
        .map_err(|error| format!("数据库完整性校验失败: {error}"))?;

    if integrity_result.is_empty()
        || integrity_result
            .iter()
            .any(|item| !item.eq_ignore_ascii_case("ok"))
    {
        return Err(format!(
            "数据库完整性校验未通过: {}",
            integrity_result.join("; ")
        ));
    }

    let foreign_key_violations =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&pool)
            .await
            .map_err(|error| format!("数据库外键校验失败: {error}"))?;

    if foreign_key_violations > 0 {
        return Err(format!(
            "数据库外键校验未通过，发现 {foreign_key_violations} 条约束问题"
        ));
    }

    Ok(())
}

async fn validate_sql_backup_script(
    script: String,
    latest_registered_version: i64,
) -> Result<(String, DatabaseMigrationStatus), String> {
    let sanitized = sanitize_sql_backup_script(&script);
    if sanitized.trim().is_empty() {
        return Err("SQL 备份文件为空或不包含可执行语句".to_string());
    }

    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .map_err(|error| format!("无法创建临时校验数据库: {error}"))?;

    sqlx::raw_sql(&sanitized)
        .execute(&pool)
        .await
        .map_err(|error| format!("SQL 备份文件执行失败: {error}"))?;

    ensure_integrity_on_pool(pool.clone()).await?;

    let migration_status = fetch_database_migration_status(&pool).await?;
    let source_version = migration_status
        .current_version
        .ok_or_else(|| "SQL 备份不包含已应用迁移记录，无法导入".to_string())?;

    if source_version > latest_registered_version {
        pool.close().await;
        return Err(format!(
            "SQL 备份版本 v{source_version} 高于当前应用支持的最新版本 v{latest_registered_version}，请先升级应用后再导入"
        ));
    }

    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| format!("无法获取临时校验数据库连接: {error}"))?;
    let migrator = build_current_migrator();
    migrator
        .run_direct(&mut *connection)
        .await
        .map_err(|error| format!("SQL 备份与当前应用迁移不兼容: {error}"))?;

    ensure_integrity_on_pool(pool.clone()).await?;

    let final_status = fetch_database_migration_status(&pool).await?;
    pool.close().await;

    Ok((sanitized, final_status))
}

async fn build_clear_database_script(pool: SqlitePool) -> Result<String, String> {
    let mut script = String::new();

    for trigger in fetch_schema_names(&pool, "trigger").await? {
        writeln!(
            &mut script,
            "DROP TRIGGER IF EXISTS {};",
            sql_identifier(&trigger)
        )
        .ok();
    }

    for view in fetch_schema_names(&pool, "view").await? {
        writeln!(
            &mut script,
            "DROP VIEW IF EXISTS {};",
            sql_identifier(&view)
        )
        .ok();
    }

    for table in fetch_schema_names(&pool, "table").await? {
        writeln!(
            &mut script,
            "DROP TABLE IF EXISTS {};",
            sql_identifier(&table)
        )
        .ok();
    }

    Ok(script)
}

async fn replace_database_from_sql(pool: SqlitePool, sanitized_sql: String) -> Result<(), String> {
    let clear_script = build_clear_database_script(pool.clone()).await?;
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| format!("无法获取数据库连接: {error}"))?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await
        .map_err(|error| format!("无法关闭外键检查: {error}"))?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|error| format!("无法开始 SQL 导入事务: {error}"))?;

    if !clear_script.trim().is_empty() {
        if let Err(error) = sqlx::raw_sql(&clear_script).execute(&mut *connection).await {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await;
            return Err(format!("清空当前数据库失败: {error}"));
        }
    }

    if let Err(error) = sqlx::raw_sql(&sanitized_sql)
        .execute(&mut *connection)
        .await
    {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        let _ = sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await;
        return Err(format!("执行 SQL 导入失败: {error}"));
    }

    if let Err(error) = sqlx::query("COMMIT").execute(&mut *connection).await {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
        let _ = sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await;
        return Err(format!("提交 SQL 导入事务失败: {error}"));
    }

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *connection)
        .await
        .map_err(|error| format!("无法恢复外键检查: {error}"))?;

    Ok(())
}

async fn run_current_migrations(pool: SqlitePool) -> Result<(), String> {
    let mut connection = pool
        .acquire()
        .await
        .map_err(|error| format!("无法获取迁移数据库连接: {error}"))?;
    let migrator = build_current_migrator();
    migrator
        .run_direct(&mut *connection)
        .await
        .map_err(|error| format!("补齐数据库迁移失败: {error}"))
}

pub(crate) async fn log_database_startup_status<R: Runtime>(app: &AppHandle<R>) {
    let path = database_path(app)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    let latest_registered_version = crate::db::migrations::latest_migration_version();

    match sqlite_pool(app).await {
        Ok(pool) => {
            let migration_summary = fetch_database_migration_status(&pool).await;

            match migration_summary {
                Ok(DatabaseMigrationStatus {
                    applied_count,
                    current_version,
                    current_description,
                }) => {
                    let current_version = current_version.unwrap_or_default();
                    let pending_migrations =
                        latest_registered_version.saturating_sub(current_version);

                    println!("[db] SQLite 已加载: path={path}");
                    println!(
                        "[db] 迁移检查完成: applied_count={applied_count}, current_version={current_version}, latest_registered_version={latest_registered_version}, pending_migrations={pending_migrations}, latest_description={}",
                        current_description.as_deref().unwrap_or("none")
                    );
                }
                Err(error) => {
                    eprintln!("[db] SQLite 已加载，但读取迁移状态失败: path={path}, error={error}");
                }
            }
        }
        Err(error) => {
            eprintln!("[db] SQLite 未加载: path={path}, error={error}");
        }
    }
}

#[tauri::command]
pub async fn health_check<R: Runtime>(app: AppHandle<R>) -> Result<AppHealthCheck, String> {
    let latest_registered_version = crate::db::migrations::latest_migration_version();
    let pool = sqlite_pool(&app).await.ok();
    let database_loaded = pool.is_some();
    let migration_status = if let Some(pool) = pool.as_ref() {
        match fetch_database_migration_status(pool).await {
            Ok(status) => Some(status),
            Err(error) => {
                eprintln!("[db] health_check 读取迁移状态失败: {error}");
                None
            }
        }
    } else {
        None
    };

    let git_result = tokio::task::spawn_blocking(preflight::check_local_git)
        .await
        .map_err(|error| format!("探测系统 git 失败: {error}"))?;
    let (git_available, git_version) = match git_result {
        Ok(version) => (true, Some(version.to_string())),
        Err(error) => (false, Some(error.to_string())),
    };

    Ok(AppHealthCheck {
        database_loaded,
        database_path: database_path(&app).map(|path| path.to_string_lossy().to_string()),
        database_current_version: migration_status
            .as_ref()
            .and_then(|status| status.current_version),
        database_current_description: migration_status
            .as_ref()
            .and_then(|status| status.current_description.clone()),
        database_latest_version: latest_registered_version,
        git_available,
        git_version,
        checked_at: now_sqlite(),
    })
}

#[tauri::command]
pub async fn backup_database<R: Runtime>(
    app: AppHandle<R>,
    destination_path: String,
) -> Result<DatabaseBackupResult, String> {
    let pool = sqlite_pool(&app).await?;
    let live_path = database_path(&app).ok_or_else(|| "无法解析数据库路径".to_string())?;
    let destination = resolve_user_file_path(&destination_path)?;

    if destination == live_path {
        return Err("备份目标不能与当前数据库文件相同".to_string());
    }

    let parent = destination
        .parent()
        .ok_or_else(|| format!("无法解析备份目录: {}", destination.display()))?;
    if !parent.exists() {
        return Err(format!("备份目录不存在: {}", parent.display()));
    }
    if destination.exists() && !destination.is_file() {
        return Err(format!("备份目标不是文件: {}", destination.display()));
    }
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| format!("无法覆盖已有备份文件: {error}"))?;
    }

    let backup_script = build_sql_backup_script(pool.clone())
        .await
        .map_err(|error| format!("生成 SQL 备份失败: {error}"))?;
    write_sql_backup_file(&destination, &backup_script)
        .map_err(|error| format!("写入 SQL 备份失败: {error}"))?;

    let migration_status = fetch_database_migration_status(&pool).await.ok();
    let created_at = now_sqlite();

    Ok(DatabaseBackupResult {
        source_path: live_path.to_string_lossy().to_string(),
        destination_path: destination.to_string_lossy().to_string(),
        database_version: migration_status.and_then(|status| status.current_version),
        created_at: created_at.clone(),
        message: format!("SQL 备份已导出到 {}", destination.display()),
    })
}

#[tauri::command]
pub fn restore_database<R: Runtime>(
    app: AppHandle<R>,
    source_path: String,
) -> Result<DatabaseRestoreResult, String> {
    tauri::async_runtime::block_on(async move {
        let source = resolve_existing_file_path(&source_path)?;
        let source_sql = fs::read_to_string(&source)
            .map_err(|error| format!("读取 SQL 备份文件失败 {}: {error}", source.display()))?;
        let latest_registered_version = crate::db::migrations::latest_migration_version();
        let (sanitized_sql, migration_status) =
            validate_sql_backup_script(source_sql, latest_registered_version).await?;
        let source_version = migration_status
            .current_version
            .ok_or_else(|| "SQL 备份不包含已应用迁移记录，无法导入".to_string())?;
        let pool = sqlite_pool(&app).await?;
        let current_backup_script = build_sql_backup_script(pool.clone())
            .await
            .map_err(|error| format!("生成导入前自动备份失败: {error}"))?;
        let backup_path = auto_import_backup_sql_path(&app)?;
        write_sql_backup_file(&backup_path, &current_backup_script)
            .map_err(|error| format!("写入导入前自动备份失败: {error}"))?;

        if let Err(error) = replace_database_from_sql(pool.clone(), sanitized_sql.clone()).await {
            return Err(format!("导入 SQL 失败，原数据库未改动。错误：{error}"));
        }

        if let Err(error) = run_current_migrations(pool.clone()).await {
            let restore_error = match replace_database_from_sql(
                pool.clone(),
                current_backup_script.clone(),
            )
            .await
            {
                Ok(()) => run_current_migrations(pool.clone()).await,
                Err(restore_error) => Err(restore_error),
            };

            return match restore_error {
            Ok(()) => Err(format!(
                "SQL 导入后补齐迁移失败，已恢复导入前数据库。错误：{error}"
            )),
            Err(recovery_error) => Err(format!(
                "SQL 导入后补齐迁移失败，且恢复导入前数据库失败：{recovery_error}。原始错误：{error}。自动备份位于 {}",
                backup_path.display()
            )),
        };
        }

        ensure_integrity_on_pool(pool.clone()).await?;
        let final_status = fetch_database_migration_status(&pool).await?;

        let restored_at = now_sqlite();
        Ok(DatabaseRestoreResult {
            source_path: source.to_string_lossy().to_string(),
            backup_path: backup_path.to_string_lossy().to_string(),
            database_version: final_status.current_version.or(Some(source_version)),
            restored_at,
            message: format!(
                "SQL 导入完成，当前数据库已更新到 v{}",
                final_status.current_version.unwrap_or(source_version)
            ),
        })
    })
}

#[tauri::command]
pub fn open_database_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let database_file = database_path(&app).ok_or_else(|| "无法解析当前数据库路径".to_string())?;
    let directory = database_file
        .parent()
        .ok_or_else(|| format!("无法解析数据库所在目录: {}", database_file.display()))?;

    if !directory.exists() {
        return Err(format!("数据库目录不存在: {}", directory.display()));
    }

    if !directory.is_dir() {
        return Err(format!("数据库目录不是文件夹: {}", directory.display()));
    }

    app.opener()
        .open_path(directory.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| format!("打开数据库文件夹失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_migrated_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("create sqlite memory pool");
        let migrator = build_current_migrator();
        let mut connection = pool.acquire().await.expect("acquire sqlite connection");
        migrator
            .run_direct(&mut *connection)
            .await
            .expect("run migrations");
        drop(connection);
        pool
    }

    #[test]
    fn sanitizes_sql_control_statements() {
        let source = "\u{feff}BEGIN TRANSACTION;\nCREATE TABLE demo(id INTEGER);\nCOMMIT;\nPRAGMA foreign_keys=OFF;\nINSERT INTO demo VALUES (1);\n";
        let sanitized = sanitize_sql_backup_script(source);

        assert!(sanitized.contains("CREATE TABLE demo(id INTEGER);"));
        assert!(sanitized.contains("INSERT INTO demo VALUES (1);"));
        assert!(!sanitized.contains("BEGIN TRANSACTION"));
        assert!(!sanitized.contains("COMMIT"));
        assert!(!sanitized.contains("PRAGMA foreign_keys=OFF"));
    }

    #[test]
    fn terminates_sql_statement_once() {
        assert_eq!(
            ensure_statement_terminated("CREATE TABLE demo(id INTEGER)"),
            "CREATE TABLE demo(id INTEGER);"
        );
        assert_eq!(
            ensure_statement_terminated("CREATE TABLE demo(id INTEGER);"),
            "CREATE TABLE demo(id INTEGER);"
        );
        assert_eq!(ensure_statement_terminated("   "), "");
    }

    #[test]
    fn migrator_reports_latest_version() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            let status = fetch_database_migration_status(&pool)
                .await
                .expect("fetch migration status");
            assert_eq!(status.current_version, Some(7));
            assert_eq!(status.applied_count, 7);
            assert_eq!(
                status.current_description.as_deref(),
                Some("activity audit logs")
            );
        });
    }

    #[test]
    fn backup_script_round_trips_with_channel_row() {
        tauri::async_runtime::block_on(async {
            let pool = setup_migrated_pool().await;
            sqlx::query(
                "INSERT INTO ai_channels (id, name, protocol, base_url, api_key) VALUES ('ch-1', 'demo', 'openai', 'https://example.com', 'sk-test')",
            )
            .execute(&pool)
            .await
            .expect("insert ai channel");

            let script = build_sql_backup_script(pool.clone())
                .await
                .expect("build backup script");
            assert!(script.contains("-- noxcode SQL backup"));
            assert!(script.contains("INSERT INTO \"ai_channels\""));

            let (_, status) = validate_sql_backup_script(script, 7)
                .await
                .expect("validate backup script");
            assert_eq!(status.current_version, Some(7));
        });
    }
}
