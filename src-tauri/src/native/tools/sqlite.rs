use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::TryStreamExt;
use libsqlite3_sys as ffi;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, Connection, Executor, Row, SqliteConnection, Statement, TypeInfo, ValueRef};

use super::cancel::CancelFlag;
use super::dispatch::ToolCtx;

const QUERY_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_QUERY_BYTES: usize = 100_000;
const MAX_RESULT_BYTES: usize = 1_000_000;
const MAX_VALUE_BYTES: c_int = 256_000;

#[derive(Deserialize)]
struct QueryArgs {
    file_path: String,
    query: String,
    #[serde(default)]
    parameters: Vec<Value>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    200
}

impl QueryArgs {
    fn parse(arguments: &str) -> Result<Self, String> {
        let args: Self = serde_json::from_str(arguments)
            .map_err(|error| format!("SQLiteQuery 参数错误: {error}"))?;
        if args.file_path.trim().is_empty() || args.query.trim().is_empty() {
            return Err("file_path 和 query 不能为空".to_string());
        }
        if args.query.len() > MAX_QUERY_BYTES || !(1..=1000).contains(&args.limit) {
            return Err("SQL 最多 100000 字节，limit 必须在 1 到 1000 之间".to_string());
        }
        for value in &args.parameters {
            if value.is_array() || value.is_object() || (value.is_u64() && value.as_i64().is_none())
            {
                return Err(
                    "parameters 仅支持字符串、64 位有符号整数、浮点数、布尔值或 null".to_string(),
                );
            }
        }
        Ok(args)
    }
}

pub async fn query(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    if ctx.ssh.is_some() {
        return Err("SQLiteQuery 仅支持本地数据库".to_string());
    }
    let args = QueryArgs::parse(arguments)?;
    let path = ctx.workspace_for_read().resolve_for_read(&args.file_path)?;
    if !path.is_file() {
        return Err(format!("SQLite 数据库不存在或不是文件: {}", path.display()));
    }
    execute_query(&path, &args, &ctx.cancel, QUERY_TIMEOUT).await
}

async fn execute_query(
    path: &Path,
    args: &QueryArgs,
    cancel: &CancelFlag,
    timeout: Duration,
) -> Result<String, String> {
    if cancel.is_cancelled() {
        return Err("已取消".to_string());
    }
    let deadline = Instant::now() + timeout;
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(2))
        .pragma("query_only", "ON")
        .pragma("trusted_schema", "OFF")
        .pragma("temp_store", "MEMORY");
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|error| format!("无法只读打开 SQLite 数据库: {error}"))?;
    let result = read_rows(&mut connection, args, cancel, deadline).await;
    // Each call has its own connection; user SQL never uses the application's writable pool.
    let _ = connection.close().await;
    if cancel.is_cancelled() {
        return Err("已取消".to_string());
    }
    if Instant::now() >= deadline {
        return Err("SQLite 只读查询超时".to_string());
    }
    result
}

async fn read_rows(
    connection: &mut SqliteConnection,
    args: &QueryArgs,
    cancel: &CancelFlag,
    deadline: Instant,
) -> Result<String, String> {
    {
        let mut handle = connection.lock_handle().await.map_err(query_error)?;
        let cancelled = cancel.clone();
        handle.set_progress_handler(1000, move || {
            !cancelled.is_cancelled() && Instant::now() < deadline
        });
        let raw = handle.as_raw_handle().as_ptr();
        // SQLx's handle lock excludes its worker. The authorizer is static and owns no data.
        unsafe {
            ffi::sqlite3_limit(raw, ffi::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES);
            ffi::sqlite3_limit(raw, ffi::SQLITE_LIMIT_COLUMN, 128);
            if ffi::sqlite3_set_authorizer(raw, Some(read_only_authorizer), ptr::null_mut())
                != ffi::SQLITE_OK
            {
                return Err("无法启用 SQLite 只读查询授权器".to_string());
            }
            validate_single_statement(raw, &args.query)?;
        }
    }
    let statement = connection.prepare(&args.query).await.map_err(query_error)?;
    let columns: Vec<_> = statement
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect();
    let mut query = sqlx::query(&args.query);
    for parameter in &args.parameters {
        query = match parameter {
            Value::Null => query.bind(Option::<String>::None),
            Value::Bool(value) => query.bind(*value),
            Value::Number(value) if value.is_i64() => query.bind(value.as_i64().unwrap()),
            Value::Number(value) => query.bind(value.as_f64().unwrap()),
            Value::String(value) => query.bind(value),
            _ => return Err("不支持的 SQLite 参数类型".to_string()),
        };
    }
    let mut stream = query.fetch(connection);
    let mut rows = Vec::new();
    let mut bytes = serde_json::to_vec(&columns)
        .map_err(|error| error.to_string())?
        .len();
    let mut truncated = false;
    while let Some(row) = stream.try_next().await.map_err(query_error)? {
        if cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        if rows.len() == args.limit {
            truncated = true;
            break;
        }
        let values = row_values(&row)?;
        bytes += serde_json::to_vec(&values)
            .map_err(|error| error.to_string())?
            .len();
        if bytes > MAX_RESULT_BYTES {
            truncated = true;
            break;
        }
        rows.push(values);
    }
    Ok(json!({ "columns": columns, "rows": rows, "row_count": rows.len(), "truncated": truncated }).to_string())
}

fn row_values(row: &SqliteRow) -> Result<Vec<Value>, String> {
    (0..row.len())
        .map(|index| {
            let raw = row.try_get_raw(index).map_err(query_error)?;
            if raw.is_null() {
                return Ok(Value::Null);
            }
            Ok(match raw.type_info().name() {
                "INTEGER" => json!(row.try_get::<i64, _>(index).map_err(query_error)?),
                "REAL" => json!(row.try_get::<f64, _>(index).map_err(query_error)?),
                "BLOB" => {
                    let value: Vec<u8> = row.try_get(index).map_err(query_error)?;
                    json!({ "base64": BASE64.encode(&value), "bytes": value.len() })
                }
                _ => json!(row.try_get::<String, _>(index).map_err(query_error)?),
            })
        })
        .collect()
}

fn query_error(error: sqlx::Error) -> String {
    format!("SQLite 只读查询失败: {error}")
}

unsafe extern "C" fn read_only_authorizer(
    _: *mut c_void,
    action: c_int,
    first: *const c_char,
    second: *const c_char,
    _: *const c_char,
    _: *const c_char,
) -> c_int {
    match action {
        ffi::SQLITE_SELECT | ffi::SQLITE_READ | ffi::SQLITE_RECURSIVE => ffi::SQLITE_OK,
        ffi::SQLITE_FUNCTION if !second.is_null() => {
            // SQLite supplies valid strings for the duration of this callback.
            let function = unsafe { CStr::from_ptr(second) }.to_bytes();
            if [b"load_extension".as_slice(), b"readfile", b"writefile"]
                .iter()
                .any(|name| function.eq_ignore_ascii_case(name))
            {
                ffi::SQLITE_DENY
            } else {
                ffi::SQLITE_OK
            }
        }
        ffi::SQLITE_PRAGMA if !first.is_null() => {
            let pragma = unsafe { CStr::from_ptr(first) }.to_bytes();
            if [
                b"table_info".as_slice(),
                b"table_xinfo",
                b"table_list",
                b"index_list",
                b"index_info",
                b"index_xinfo",
                b"foreign_key_list",
                b"database_list",
                b"compile_options",
            ]
            .iter()
            .any(|name| pragma.eq_ignore_ascii_case(name))
            {
                ffi::SQLITE_OK
            } else {
                ffi::SQLITE_DENY
            }
        }
        _ => ffi::SQLITE_DENY,
    }
}

/// Use SQLite's parser so comments, quoted semicolons and WITH cannot hide another statement.
unsafe fn validate_single_statement(db: *mut ffi::sqlite3, query: &str) -> Result<(), String> {
    let query = CString::new(query).map_err(|_| "SQL 不能包含 NUL 字符".to_string())?;
    let mut cursor = query.as_ptr();
    let mut count = 0;
    loop {
        let mut statement = ptr::null_mut();
        let mut tail = ptr::null();
        // The connection is exclusively locked and the SQL buffer outlives all prepare calls.
        let code = unsafe { ffi::sqlite3_prepare_v2(db, cursor, -1, &mut statement, &mut tail) };
        let error = if code != ffi::SQLITE_OK {
            Some(
                unsafe { CStr::from_ptr(ffi::sqlite3_errmsg(db)) }
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        };
        if !statement.is_null() {
            count += 1;
            let read_only = unsafe { ffi::sqlite3_stmt_readonly(statement) } != 0;
            unsafe {
                ffi::sqlite3_finalize(statement);
            }
            if !read_only {
                return Err("SQLiteQuery 只允许只读查询".to_string());
            }
        }
        if let Some(error) = error {
            return Err(format!("SQLite 只读查询失败: {error}"));
        }
        if count > 1 {
            return Err("SQLiteQuery 每次只允许一条 SQL 语句".to_string());
        }
        if tail.is_null() || tail == cursor || unsafe { *tail } == 0 {
            break;
        }
        cursor = tail;
    }
    if count == 0 {
        return Err("query 中没有 SQL 语句".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::dispatch::execute_tool;
    use super::super::local::LocalWorkspace;
    use super::super::permission::NativePermissionDecision;
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    async fn database(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("logs.sqlite");
        let mut connection = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::raw_sql("CREATE TABLE logs(id INTEGER PRIMARY KEY, message TEXT); INSERT INTO logs VALUES(1,'first'),(2,'second'),(3,'third');").execute(&mut connection).await.unwrap();
        connection.close().await.unwrap();
        path
    }

    fn arguments(path: &Path, query: &str) -> String {
        json!({"file_path": path, "query": query}).to_string()
    }

    #[tokio::test]
    async fn plan_mode_queries_data_parameters_and_schema_without_changing_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = database(dir.path()).await;
        let original = std::fs::read(&path).unwrap();
        let ctx = ToolCtx::new(LocalWorkspace::new(dir.path().to_path_buf()));
        ctx.set_read_only(true);
        let output = execute_tool(&ctx,"SQLiteQuery",&json!({"file_path":path,"query":"SELECT id, message FROM logs WHERE id > ? ORDER BY id", "parameters":[1],"limit":1}).to_string()).await.unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["columns"], json!(["id", "message"]));
        assert_eq!(result["rows"], json!([[2, "second"]]));
        assert_eq!(result["truncated"], true);
        let schema = execute_tool(
            &ctx,
            "SQLiteQuery",
            &arguments(
                &path,
                "SELECT name, sql FROM sqlite_schema WHERE type = 'table'",
            ),
        )
        .await
        .unwrap();
        assert!(schema.contains("CREATE TABLE logs"));
        let schema = execute_tool(
            &ctx,
            "SQLiteQuery",
            &arguments(&path, "PRAGMA table_info('logs')"),
        )
        .await
        .unwrap();
        assert!(schema.contains("message"));
        let schema = execute_tool(
            &ctx,
            "SQLiteQuery",
            &arguments(&path, "SELECT name FROM pragma_table_info('logs')"),
        )
        .await
        .unwrap();
        assert!(schema.contains("message"));
        let data = execute_tool(&ctx,"SQLiteQuery",&arguments(&path,"WITH q AS (SELECT '; literal' AS text) SELECT text, NULL, 1.5, x'00ff', 1, 2 AS text FROM q; -- trailing comment")).await.unwrap();
        let data: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(
            data["rows"],
            json!([["; literal",null,1.5,{"base64":"AP8=","bytes":2},1,2]])
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(execute_tool(
            &ctx,
            "Bash",
            r#"{"command":"sqlite3 -readonly logs.sqlite 'SELECT 1'"}"#
        )
        .await
        .unwrap_err()
        .contains("只读规划模式"));
    }

    #[tokio::test]
    async fn reads_committed_wal_rows_while_the_application_connection_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("live.sqlite");
        let mut writer = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
        sqlx::raw_sql("CREATE TABLE logs(message TEXT); INSERT INTO logs VALUES('live API log');")
            .execute(&mut writer)
            .await
            .unwrap();
        let ctx = ToolCtx::new(LocalWorkspace::new(dir.path().to_path_buf()));
        ctx.set_read_only(true);
        let result = execute_tool(
            &ctx,
            "SQLiteQuery",
            &arguments(&path, "SELECT message FROM logs"),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap()["rows"],
            json!([["live API log"]])
        );
        sqlx::query("INSERT INTO logs VALUES ('next log')")
            .execute(&mut writer)
            .await
            .unwrap();
        writer.close().await.unwrap();
    }

    #[tokio::test]
    async fn writes_attachments_extensions_and_additional_statements_are_denied_even_in_yolo() {
        let dir = tempfile::tempdir().unwrap();
        let path = database(dir.path()).await;
        let original = std::fs::read(&path).unwrap();
        let ctx = ToolCtx::new(LocalWorkspace::new(dir.path().to_path_buf()));
        ctx.set_read_only(true);
        ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        for sql in [
            "INSERT INTO logs VALUES(4,'forbidden')",
            "UPDATE logs SET message='forbidden'",
            "DELETE FROM logs",
            "CREATE TABLE forbidden(x)",
            "DROP TABLE logs",
            "PRAGMA query_only=OFF",
            "PRAGMA writable_schema=ON",
            "PRAGMA user_version=9",
            "PRAGMA journal_mode=WAL",
            "ATTACH DATABASE ':memory:' AS other",
            "SELECT load_extension('/missing')",
            "SELECT 1; SELECT 2",
            "SELECT 1; DELETE FROM logs",
            "WITH q AS (SELECT 1) DELETE FROM logs RETURNING id",
            "BEGIN",
            ".shell touch forbidden",
            "VACUUM INTO 'forbidden.sqlite'",
        ] {
            let result = execute_tool(&ctx, "SQLiteQuery", &arguments(&path, sql)).await;
            assert!(result.is_err(), "{sql}: {result:?}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn external_database_reuses_read_permissions_and_once_does_not_persist() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = database(outside.path()).await;
        let mut ctx = ToolCtx::new(LocalWorkspace::new(root.path().to_path_buf()));
        ctx.set_read_only(true);
        let calls = Arc::new(AtomicUsize::new(0));
        let asked = calls.clone();
        ctx.request_permission = Some(Arc::new(move |prompt, reply| {
            let access = prompt.file_access.unwrap();
            assert_eq!(
                access.paths[0].capability,
                super::super::contract::PermissionCapability::Read
            );
            assert!(access.paths[0].outside_workspace);
            let decision = if asked.fetch_add(1, Ordering::SeqCst) == 0 {
                NativePermissionDecision::AllowOnce
            } else {
                NativePermissionDecision::Deny
            };
            reply.send(decision).unwrap();
        }));
        let args = arguments(&path, "SELECT count(*) FROM logs");
        let result = execute_tool(&ctx, "SQLiteQuery", &args).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap()["rows"],
            json!([[3]])
        );
        assert!(execute_tool(&ctx, "SQLiteQuery", &args).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(ctx.permission_rules_snapshot().is_empty());
        ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        execute_tool(&ctx, "SQLiteQuery", &args).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancellation_and_deadline_interrupt_expensive_queries() {
        let dir = tempfile::tempdir().unwrap();
        let path = database(dir.path()).await;
        let args = QueryArgs::parse(&arguments(&path,"WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n")).unwrap();
        let started = Instant::now();
        let error = execute_query(&path, &args, &CancelFlag::new(), Duration::from_millis(30))
            .await
            .unwrap_err();
        assert!(error.contains("超时"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
        let cancel = CancelFlag::new();
        let (result, ()) = tokio::join!(
            execute_query(&path, &args, &cancel, Duration::from_secs(10)),
            async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                cancel.cancel();
            }
        );
        assert_eq!(result.unwrap_err(), "已取消");
    }

    #[tokio::test]
    async fn missing_databases_are_not_created_and_large_values_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(LocalWorkspace::new(dir.path().to_path_buf()));
        let missing = dir.path().join("missing.sqlite");
        assert!(
            execute_tool(&ctx, "SQLiteQuery", &arguments(&missing, "SELECT 1"))
                .await
                .is_err()
        );
        assert!(!missing.exists());
        let path = database(dir.path()).await;
        assert!(execute_tool(
            &ctx,
            "SQLiteQuery",
            &arguments(&path, "SELECT randomblob(10000000)")
        )
        .await
        .is_err());
        let output = execute_tool(&ctx,"SQLiteQuery",&arguments(&path,"WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100) SELECT hex(zeroblob(20000)) FROM n")).await.unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["truncated"], true);
        assert!(output.len() < MAX_RESULT_BYTES + 10_000);
    }
}
