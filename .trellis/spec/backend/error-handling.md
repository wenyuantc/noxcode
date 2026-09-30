# Error Handling

> How errors are typed, converted, surfaced to the frontend, and logged.

---

## Summary

| Where | Error type | Evidence |
|-------|-----------|----------|
| Tauri command boundary | `Result<T, String>` | all commands in `app/`, `git/mod.rs`, `native/` |
| Service functions | mostly `Result<T, String>` too | `app/workspaces.rs`, `app/activity_logs.rs`, `app/shared.rs` |
| Rich subsystems | `thiserror` enums + `impl From<E> for String` | `SshError` (`app/ssh/error.rs`), `GitError` (`git/runner.rs`); `GitPreflightError` (`git/preflight.rs`) is thiserror too but startup-only, no `String` conversion |
| Hand-written error structs | `Display` + `std::error::Error` (+ `From<_> for String`) | `ModelError` (`native/model/response.rs`), `MediaError` with stable `code` (`native/media_error.rs`) |

There is **no `anyhow`** in the crate and no `Serialize` error type sent to the
frontend. The frontend always receives a plain string.

## Typed Errors with thiserror

Enums carry user-facing Chinese messages in `#[error(...)]`, wrap sources with
`#[from]`, and convert to `String` so `?` works inside `Result<_, String>` functions:

```rust
// src-tauri/src/app/ssh/error.rs
#[derive(Debug, thiserror::Error)]
pub(crate) enum SshError {
    #[error("SSH 协议错误: {0}")]
    Russh(#[from] russh::Error),
    #[error("连接超时")]
    ConnectTimeout,
    #[error("主机密钥已变更（known_hosts 第 {line} 行，文件 {path}），已拒绝连接以防止中间人攻击")]
    HostKeyChanged { line: usize, path: String },
    // ...
}

impl From<SshError> for String {
    fn from(value: SshError) -> Self {
        value.to_string()
    }
}
```

`GitError` in `git/runner.rs` follows the same shape (`Bug`, `NotARepo`,
`CommandFailed { args, exit_code, stderr }`, `Timeout`, `Io(#[from] io::Error)`,
`Blocked`, `VersionTooOld`, ...). Git commands convert at the boundary:

```rust
// src-tauri/src/git/mod.rs
get_status(&target, untracked_mode.as_deref())
    .await
    .map_err(Into::into)
```

When a subsystem needs programmatic matching, use a stable code rather than parsing text:
`MediaError { code: &'static str, message }` ("调用方只比较 `code`"), and
`ModelError { kind: ModelErrorKind, message }`.

## Plain String Errors

For simple services, build the string at the failure site with Chinese context +
the underlying error:

```rust
// src-tauri/src/app/workspaces.rs
return Err("工作区名称不能为空".to_string());
// ...
.map_err(|error| format!("创建工作区失败: {error}"))?;
```

```rust
// src-tauri/src/app/network_settings.rs
.app_config_dir()
.map_err(|error| format!("无法读取应用配置目录: {error}"))?;
```

Conventions:

- The closure variable is named `error` (not `e`).
- Message language is Chinese for anything a user can see. A few internal messages are
  English (`"Database {DB_URL} is not loaded"`, `"Failed to load workspace: {error}"`);
  prefer Chinese for new code.
- Validation errors name the offending field/value: `"不支持的工作区类型: {other}"`,
  `"本地工作区必须提供 repo_path"`.
- Business-rule refusals are errors, not silent no-ops:
  `"该工作区有运行中的会话，无法删除"` in `delete_workspace`.

## Surfacing to the Frontend

`invoke` rejects with the `String`. The UI converts with `errorMessage` from
`src/lib/toast.ts`:

```ts
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
```

So the Rust string is displayed verbatim -- write it for end users.

## Best-Effort Side Effects

Secondary cleanup that must not fail the main operation is logged and swallowed:

```rust
// src-tauri/src/app/workspaces.rs (delete_workspace)
if let Ok(target) = resolve_git_target(&app, &id).await {
    if let Err(error) = clear_workspace_checkpoints(&pool, &target, &id).await {
        eprintln!("[git] 清理工作区 checkpoint 失败: {error}");
    }
}
```

Same pattern in `git::get_git_repo_info` (`"[git] 清扫孤儿 checkpoint ref 失败: {error}"`).
Event emission results are ignored with `let _ = app.emit(...)`.

## Fatal Startup Errors

Only the git preflight is fatal. `git::preflight::run_startup_check()` runs before the
Tauri builder, stores the message in a `OnceLock`, and
`show_fatal_dialog_if_needed(app)` (called on `RunEvent::Ready` in `lib.rs`) shows a
Chinese error dialog titled "noxcode 无法启动" on a separate thread, then
`std::process::exit(1)`. Never block `setup` with a dialog (`docs/architecture.md`).

## Logging

There is **no `log` / `tracing` crate**. Logging is stderr/stdout:

- `eprintln!` (about 60 call sites) with a bracketed module tag:
  `[native]` (most), `[git]`, `[db]`, `[scheduler]`, `[plugins]`, `[mcp-oauth]`, `[ai]`,
  `[MCP]` -- e.g. `eprintln!("[git] 清理工作区 checkpoint 失败: {error}")`.
- `println!` only for debug-build DB startup status in
  `app::database::log_database_startup_status` (`[db] SQLite 已加载: ...`), spawned from
  `lib.rs` under `if cfg!(debug_assertions)`.
- Model/API calls are persisted to `native_api_call_logs` (`native/model/call_log.rs`) and
  user-visible audit events to `activity_logs` (`app/activity_logs.rs::insert_activity_log`)
  -- use these instead of console logs when the UI must show history.

## Forbidden / Avoid

- Adding `anyhow`, `log`, or `tracing` just for one feature -- follow the existing style.
- `unwrap()` / `expect()` on runtime I/O or DB results in command paths -- use `?` with a
  mapped message. Existing runtime uses are limited to `lib.rs`
  (`.expect("error while building tauri application")`), std `Mutex`/`RwLock` poisoning
  (`.lock().expect("ssh pool entries lock")` in `app/ssh/pool.rs`, `native/input_queue.rs`,
  `native/steer.rs`) and invariants that cannot fail (`tray.rs` default icon); everything
  else is in tests.
- Returning `Ok(())` after a failed business rule; return `Err("<中文原因>".to_string())`.
- Leaking secrets into error strings (SSH passwords/passphrases live in keyring via
  `app/secret_store.rs`; never format them into messages).
- Parsing error text on the frontend to branch logic; add a code/kind field instead
  (see `MediaError`, `ModelError`, or `GitPullResult { updated, message }` in `docs/git.md`).
