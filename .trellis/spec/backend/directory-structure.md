# Directory Structure

> How the Rust backend in `src-tauri/src` is organized and where new code goes.

---

## Crate Layout

`src-tauri/Cargo.toml` builds one library crate `noxcode_lib`; the binary is a stub:

```rust
// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    noxcode_lib::run()
}
```

```
src-tauri/
├── Cargo.toml / Cargo.lock / build.rs / tauri.conf.json
├── capabilities/default.json   # window permissions -- NO sql:* entries
├── resources/skills/           # bundled resources
├── tests/fixtures/replay/      # fixture data only (no integration test crates)
└── src/
    ├── main.rs            # calls noxcode_lib::run()
    ├── lib.rs             # plugins, setup(), managed state, invoke_handler, RunEvent loop
    ├── process_spawn.rs   # the ONLY constructor for non-git child processes
    ├── tray.rs            # tray + app menu, show_main_window command
    ├── window_event.rs    # main window close -> hide
    ├── app/               # app-shell services: DB admin, SSH, workspaces, sessions, settings
    ├── db/                # migrations, row models / IPC DTOs, test pool
    ├── engine/            # ExecutionContext (local | ssh), usage accounting
    ├── git/               # all git logic; runner.rs is the only git spawner
    └── native/            # in-process Native Agent runtime (model, tools, session, MCP...)
```

## Module Responsibilities

| Path | Owns | Examples |
|------|------|----------|
| `lib.rs` | Module declarations (`mod app; mod db; ...`), plugin registration, `app.manage(...)` of shared state, the single `tauri::generate_handler![...]` list | `SshPool`, `Arc<Mutex<NativeAgentManager>>`, `Lifecycle` are managed here |
| `app/shared.rs` | DB handle + common helpers: `DB_URL`, `sqlite_pool`, `now_sqlite`, `new_id`, `normalize_optional_text`, `database_path` | used by nearly every service |
| `app/*.rs` | One file per settings/CRUD domain: `workspaces.rs`, `sessions.rs`, `activity_logs.rs`, `database.rs`, `quick_prompts.rs`, `network_settings.rs`, `ai_settings.rs`, `secret_store.rs`, `lifecycle.rs`, `notifications.rs` | |
| `app/ssh/` | russh client, `SshPool`, known_hosts, `~/.ssh/config` import, `SshError` (`error.rs`), test server (`#[cfg(test)]`) | `app/ssh/mod.rs` holds the SSH commands |
| `db/migrations.rs` | `get_all_migrations()` + `latest_migration_version()` + schema tests | |
| `db/models.rs` | `#[derive(Serialize, Deserialize, FromRow)]` rows and IPC DTOs (`Workspace`, `CreateWorkspace`, `UpdateWorkspace`, `SshConfig`...) | |
| `db/test_support.rs` | `#[cfg(test)]` `setup_migrated_pool()` (in-memory SQLite, all migrations, FKs on) | |
| `engine/` | `context.rs` (`ExecutionContext`), `usage.rs` (`UsageDelta`) | |
| `git/` | `runner.rs` (`GitTarget`, `IndexMode`, `ScratchIndex`, guard, per-repo lock), `preflight.rs`, feature files (`status.rs`, `diff.rs`, `checkpoint.rs`, `merge.rs`, `worktree.rs`...), `tests.rs` | Git commands live in `git/mod.rs` |
| `native/` | Agent runtime: `model/` (HTTP clients), `tools/` (Bash, MCP, LSP, sandbox, web...), `agent/` (loop, compaction), `session/` (session commands, split into sub-files), `channels.rs`, `settings.rs`, `skills.rs`, `scheduler.rs`, `history.rs`, `attachments/`... | |

Module visibility convention: sub-modules are declared `pub(crate) mod`
(`src-tauri/src/app/mod.rs`, `src-tauri/src/native/mod.rs`); `db/mod.rs` uses `pub mod`.
Internal helpers use `pub(crate)` / `pub(super)`.

## Where New Code Goes

| You are adding... | Put it in |
|-------------------|-----------|
| CRUD for a new app-shell entity (not agent-specific) | new `app/<entity>.rs` + `pub(crate) mod <entity>;` in `app/mod.rs` |
| A persisted setting stored as JSON under `$APPCONFIG` | `app/<name>_settings.rs`, following `app/network_settings.rs` (`load_*_from(dir)` / `save_*_to(dir)` + thin commands) |
| Anything that runs `git` | a feature file under `git/`, calling `runner::git(...)`; command in `git/mod.rs` |
| Agent / model / tool / MCP behaviour | the matching file under `native/` |
| Session commands | a sub-file of `native/session/` plus re-export in `native/session/mod.rs` (see below) |
| A new table / column | append a migration in `db/migrations.rs`; row/DTO structs in `db/models.rs` (or next to the service for small local types, e.g. `ActivityLog` in `app/activity_logs.rs`) |
| Cross-module test helpers | `#[cfg(test)]` module, e.g. `db/test_support.rs`, `app/ssh/test_server.rs` |

## Splitting a Large Module

Large feature modules are split into a directory with private sub-modules and
re-exports from `mod.rs`. `native/session/mod.rs` shows the pattern -- note that
Tauri's hidden `__cmd__*` / `__tauri_command_name_*` items must be re-exported
together with the command, otherwise `generate_handler!` cannot resolve
`native::session::<command>`:

```rust
// src-tauri/src/native/session/mod.rs
mod startup;
mod permissions;
pub use startup::start_native_session;
pub use startup::{__cmd__start_native_session, __tauri_command_name_start_native_session};
pub use permissions::{
    apply_native_file_rollback, fork_native_session, preview_native_file_rollback,
    resolve_native_tool_permission, NativeFileRollbackInput, NativeFileRollbackPreviewInput,
};
```

Test-only files use `#[cfg(test)] mod tests;` pointing at a sibling `tests.rs`
(`git/mod.rs`, `native/session/mod.rs`, `native/attachments/mod.rs`,
`native/agent/loop/mod.rs`).

## Anti-patterns

- Do not add business logic to `lib.rs`; it only wires plugins, state and the handler list.
- Do not create a second DB access path (e.g. a new `SqlitePool::connect`) in runtime code;
  always use `app::shared::sqlite_pool(&app)`. Only tests connect `sqlite::memory:` directly.
- Do not put git spawning outside `git/runner.rs` (enforced by the `only_runner_spawns_git` test).

## Doc Discrepancy

`docs/architecture.md` "当前落地 vs 目标分层" still says `db/` is at version 9 with 12
tables. Code (`db/migrations.rs`) is at version 19, and `docs/database.md` says 26
tables. Trust the code.
