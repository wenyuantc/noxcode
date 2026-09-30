# Database Guidelines

> SQLite access, migrations, and DB tests in `src-tauri/`.

---

## Stack

- `tauri-plugin-sql` (feature `sqlite`) owns the connection pool and runs migrations.
  `src-tauri/tauri.conf.json` preloads `"sqlite:noxcode.db"`; `src-tauri/src/lib.rs`
  registers the migrations:

  ```rust
  tauri_plugin_sql::Builder::default()
      .add_migrations(app::shared::DB_URL, db::migrations::get_all_migrations())
      .build(),
  ```
- Rust code queries with `sqlx = "=0.8.6"` **runtime** APIs (`sqlx::query`,
  `sqlx::query_as::<_, T>`, `sqlx::raw_sql`). There are no `query!` / `query_as!`
  compile-time macros in the crate.
- File location: `$APPCONFIG/noxcode.db` (`app::shared::database_path`). Applied versions are
  tracked by sqlx in `_sqlx_migrations`.

## Hard Rule: No Frontend SQL

- `src-tauri/capabilities/default.json` must never gain any `sql:*` permission -- not even
  `sql:default`, because it includes `allow-select` (`AGENTS.md`, `docs/architecture.md`).
  Current permissions: `core:default`, `opener:default`, `dialog:default`,
  `notification:default`, `updater:default`.
- `src/lib/database.ts` is a hard-fail stub (`getDb` / `select` / `execute` throw
  "前端禁止直接访问 SQL..."). Do not "fix" it. Add a Tauri command instead.

## Getting the Pool

Runtime code always goes through `app::shared::sqlite_pool`, which reads the plugin's
`DbInstances`:

```rust
// src-tauri/src/app/shared.rs
pub(crate) async fn sqlite_pool<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, String> {
    let instances = app.state::<DbInstances>();
    let instances = instances.0.read().await;
    let db = instances
        .get(DB_URL)
        .ok_or_else(|| format!("Database {DB_URL} is not loaded"))?;
    let DbPool::Sqlite(pool) = db;
    Ok(pool.clone())
}
```

Commands call it once and pass `&SqlitePool` down (26 files use `sqlite_pool(&app)`).

## Query Conventions

- Placeholders are `$1, $2, ...` (~240 occurrences of `= $1`; `?` is not used in app SQL —
  it only appears in the agent's `SQLiteQuery` tool for user databases, `native/tools/sqlite.rs`).
- Rows are structs deriving `sqlx::FromRow` + serde, read with `query_as::<_, T>`.
  `SELECT *` into a full row struct is common (`app/workspaces.rs`, `app/activity_logs.rs`).
- Every DB error is mapped to a Chinese (occasionally English) message with context:
  `.map_err(|error| format!("创建工作区失败: {error}"))?`.
- "Not found" is an explicit error, not `Option` leaking to the UI:

  ```rust
  // src-tauri/src/app/workspaces.rs
  sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces WHERE id = $1 LIMIT 1")
      .bind(id)
      .fetch_optional(pool)
      .await
      .map_err(|error| format!("Failed to load workspace: {error}"))?
      .ok_or_else(|| format!("工作区不存在: {id}"))
  ```
- IDs: `new_id()` (UUID v4 string). Timestamps: `now_sqlite()` (`%Y-%m-%d %H:%M:%S`, UTC),
  written explicitly on insert/update even though tables also have `DEFAULT (datetime('now'))`.
- Create/update functions re-read and return the stored row (`create_workspace_with` ends
  with `fetch_workspace_by_id(pool, &id).await`).

## Transactions

Use `pool.begin()` and write helpers generic over an executor so they work inside
and outside a transaction:

```rust
// src-tauri/src/native/session/events.rs
let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
if receipt.status == crate::native::steer::SteerStatus::Accepted {
    plans::invalidate_persisted(&mut *transaction, &receipt.session_record_id, true).await?;
}
insert_session_event(&mut *transaction, &receipt.session_record_id, "native_steer", Some(&message)).await?;
transaction.commit().await.map_err(|error| error.to_string())

pub(super) async fn insert_session_event<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
```

Other transactional writers: `native/history.rs` (`.begin()`), `native/attachments/service.rs`.

## Migrations

All migrations live in one `Vec` in `src-tauri/src/db/migrations.rs`:

```rust
Migration {
    version: 16,
    description: "goal completion evidence is bound to a history branch",
    sql: "ALTER TABLE native_goals ADD COLUMN bound_branch_id TEXT;",
    kind: tauri_plugin_sql::MigrationKind::Up,
},
```

Rules (from `docs/database.md`, verified against code):

- Versions are contiguous `1..N`, append-only. **Current latest in code: 19**
  (always re-check the last `version:` in `migrations.rs`; docs lag behind).
- Never edit the SQL of an already-shipped migration, never insert a version in the middle,
  no down migrations (`MigrationKind::Up` only). Rollback = restore a pre-upgrade backup.
- Multi-statement migrations use a raw string `r#"...";"#` with `CREATE TABLE`,
  `CREATE INDEX`, triggers; single-column additions use `ALTER TABLE ... ADD COLUMN`.
- Newer "log/ledger" tables deliberately have **no foreign keys** so history survives
  deletes (`native_api_call_logs`, `native_history_*`, `native_tool_runs`, `activity_logs`;
  see the "表关系" section in `docs/database.md`).

### Adding a Migration (N = current latest + 1)

1. Append `Migration { version: N, description, sql, kind: MigrationKind::Up }` at the end.
2. Update the hard-coded assertion in the contiguity test -- it pins the latest version:

   ```rust
   #[test]
   fn migration_versions_are_contiguous() {
       for (index, migration) in get_all_migrations().iter().enumerate() {
           assert_eq!(migration.version, index as i64 + 1);
       }
       assert_eq!(latest_migration_version(), 19);
   ```
3. If you add a table, update the expected list in
   `latest_schema_has_history_tables_without_profiles` (same file), which asserts the full
   sorted table set.
4. If existing rows must survive, add a data-preservation test that runs the first
   `N-1` migrations, inserts a row, then applies migration `N` -- pattern:
   `approved_plan_migration_preserves_existing_sessions` (runs `.take(12)` then `migrations[12]`).
5. Add/extend the row struct in `db/models.rs` and the TS type in `src/lib/types.ts`.
6. Update `AGENTS.md` / `docs/database.md` version notes (they enumerate every version).

## Test Support

`src-tauri/src/db/test_support.rs` (compiled only under `#[cfg(test)]`):

```rust
pub(crate) async fn setup_migrated_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.expect("create sqlite memory pool");
    for migration in get_all_migrations() {
        sqlx::raw_sql(migration.sql).execute(&pool).await
            .unwrap_or_else(|error| panic!("run migration {}: {}", migration.version, error));
    }
    sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await.expect("enable foreign keys");
    pool
}
```

Service tests call the `_with` / `_with_pool` functions directly with this pool, e.g.
`app/activity_logs.rs::tests::inserts_filters_and_limits_activity_logs`.
`db/migrations.rs` has its own identical `setup_test_pool()` and uses
`tauri::async_runtime::block_on` inside `#[test]`; elsewhere `#[tokio::test]` is the norm.

## Backup / Restore

`app/database.rs` implements `health_check`, `backup_database` (SQL script incl.
`_sqlx_migrations`), `restore_database` (validate -> write
`noxcode.pre-import-backup-*.sql` -> import -> re-run migrations -> integrity check).
New tables are included automatically; secrets in keyring and `$APPCONFIG/*.json`
settings are not part of the backup.

## Common Mistakes

- Adding a migration but forgetting to bump `assert_eq!(latest_migration_version(), N)`
  -> `migration_versions_are_contiguous` fails.
- Adding a table without updating the table list test.
- Using `?` placeholders or `query!` macros (inconsistent with the codebase; macros would
  also require a build-time `DATABASE_URL`).
- Returning raw `sqlx::Error` to a command -- commands return `String`; map with context.

## Doc Discrepancies

- `docs/architecture.md` says `db/` is "version 9 (12 张业务表)"; code is version 19.
- `docs/database.md` sample debug log shows `current_version=7`; it is only an example.
- `docs/database.md` heading "十二张既有业务表" refers to the pre-v14 tables; total is 26.
