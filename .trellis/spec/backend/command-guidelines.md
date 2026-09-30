# Command Guidelines

> How Tauri IPC commands are defined, registered, named, and mirrored in `src/lib/backend.ts`.

---

## The Contract in One Picture

```
src/lib/types.ts      TS interface (snake_case fields)  <->  db/models.rs struct (serde, no rename)
src/lib/backend.ts    invoke("snake_command", { camelArg })
                                   |
src-tauri/src/lib.rs  tauri::generate_handler![module::snake_command, ...]
                                   |
#[tauri::command] async fn snake_command<R: Runtime>(app: AppHandle<R>, snake_arg: T) -> Result<U, String>
                                   |
service fn  xxx_with(&pool, ...) / xxx_with_pool(&pool, ...)  ->  sqlx  ->  SQLite
```

`src/lib/backend.ts` is the only place the frontend calls `invoke` / `listen`
(`docs/architecture.md`: "前端 invoke 的唯一出口是 src/lib/backend.ts").

## Defining a Command

Real conventions (167 `#[tauri::command]` functions in `src-tauri/src`):

1. **Generic over runtime**: `pub async fn name<R: Runtime>(app: AppHandle<R>, ...)`.
   Commands get the DB via `sqlite_pool(&app).await?`, not via a `State<SqlitePool>`.
2. **Return `Result<T, String>`**. Nearly every command does; the only infallible ones are
   pure lookups such as `list_ssh_supported_algorithms` (`app/ssh/algorithms.rs`) and
   `list_model_catalog` (`native/model_catalog.rs`).
3. **Thin command, testable service**. The command resolves `AppHandle` resources and
   delegates to a function that takes `&SqlitePool` (suffix `_with` or `_with_pool`),
   which is what unit tests call.
4. **Visibility**: `pub` (`app/workspaces.rs`, `app/activity_logs.rs`) or `pub(crate)`
   (`git/mod.rs`, `app/ssh/mod.rs`) -- both work because registration is in-crate.
5. **Managed state** is accessed either as a parameter
   `state: State<'_, Arc<Mutex<NativeAgentManager>>>` (40 uses) or inline via
   `app.state::<SshPool>().inner().clone()` (`app/ssh/mod.rs`, `app/workspaces.rs`).
6. **Sync commands** are allowed for trivial work (`tray::show_main_window`,
   `list_ssh_supported_algorithms`); anything doing I/O is `async`.

Example -- thin command over a pool-taking service (`src-tauri/src/app/workspaces.rs`):

```rust
#[tauri::command]
pub async fn create_workspace<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateWorkspace,
) -> Result<Workspace, String> {
    let pool = sqlite_pool(&app).await?;
    create_workspace_with(&pool, payload).await
}
```

Example -- command guarded by live state (`src-tauri/src/app/workspaces.rs`):

```rust
#[tauri::command]
pub async fn delete_workspace<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    id: String,
) -> Result<(), String> {
    if state.lock().await.has_workspace_processes(&id) {
        return Err("该工作区有运行中的会话，无法删除".to_string());
    }
```

## Naming

- Command names are `snake_case` `verb_noun`: `list_*`, `get_*`, `create_*`, `update_*`,
  `delete_*`, plus domain verbs (`stage_git_paths`, `resolve_ssh_host_trust`,
  `ensure_scratch_workspace`, `check_workspace_health`).
- Domain is in the name so names stay unique crate-wide: `*_ssh_config`, `*_git_*`,
  `*_native_*`, `*_ai_channel`, `*_workspace`, `*_agent_session`.
- CRUD argument names are consistent: `payload` for create (and whole-document updates like
  `update_network_settings`), `id` + `updates` for partial update, `id` for get/delete.

## Registration

Every command must be listed in the single `tauri::generate_handler![...]` in
`src-tauri/src/lib.rs`, using its module path:

```rust
.invoke_handler(tauri::generate_handler![
    app::lifecycle::restart_app,
    app::activity_logs::list_activity_logs,
    app::database::health_check,
    // ...
    app::workspaces::create_workspace,
    tray::show_main_window,
])
```

Keep new entries grouped next to their module's existing entries. A command that is
defined but not registered compiles fine and fails only at runtime in the UI.
If the command lives in a private sub-module, re-export the `__cmd__*` and
`__tauri_command_name_*` items too (see `native/session/mod.rs` and
[Directory Structure](./directory-structure.md)).

No capability entry is needed for app commands; `capabilities/default.json` only lists
plugin permissions (`core:default`, `opener:default`, `dialog:default`,
`notification:default`, `updater:default`).

## Serde / Argument Conventions

| Layer | Casing | Evidence |
|-------|--------|----------|
| Top-level command args in `invoke(...)` | camelCase (Tauri maps to the snake_case Rust param) | `invoke("get_git_status", { workspaceId, untrackedMode, sessionId })` -> `workspace_id`, `untracked_mode`, `session_id` in `git/mod.rs` |
| Struct fields (DTOs, rows) | snake_case, **no** `rename_all` | `Workspace` in `db/models.rs` <-> `interface Workspace { workspace_type; repo_path; ... }` in `src/lib/types.ts` |
| Enums | `#[serde(rename_all = "snake_case")]` | `GitFileDiffScope` in `git/diff.rs` <-> `"worktree" \| "staged" \| { range: {...} }` |

Exception: `native/mcp_oauth.rs` structs use `#[serde(rename_all = "camelCase")]`, and
`McpOAuthStatus` in `src/lib/types.ts` is camelCase to match. Do not copy this for new
DTOs; new structs follow the snake_case default.

Other rules:

- Optional inputs are `Option<T>` in Rust. On the TS side pass `undefined` or `null`;
  many wrappers normalise with `sessionId: sessionId ?? null` (`src/lib/backend.ts`).
- Normalise/validate strings in the service, not the frontend: trim + reject empty
  (`normalize_name` in `app/workspaces.rs`, shared `normalize_optional_text` in `app/shared.rs`).
- Defaults and clamps live in Rust: `limit.unwrap_or(50)` then `limit.clamp(1, 200)` in
  `app/activity_logs.rs`.
- Timestamps are `String` in `%Y-%m-%d %H:%M:%S` UTC (`now_sqlite()`); IDs are UUID v4
  strings (`new_id()`).

## Frontend Wrapper (`src/lib/backend.ts`)

Each command gets exactly one exported camelCase function that returns the typed
`invoke` promise. No logic, no try/catch -- callers handle errors.

```ts
export function createWorkspace(payload: CreateWorkspaceInput): Promise<Workspace> {
  return invoke("create_workspace", { payload });
}

export function updateWorkspace(id: string, updates: UpdateWorkspaceInput): Promise<Workspace> {
  return invoke("update_workspace", { id, updates });
}
```

Types come from `import type { ... } from "./types"` at the top of `backend.ts`.

## Events (Rust -> Frontend)

Push updates use `app.emit("<kebab-name>", payload)` with a `Serialize` payload; the
frontend subscribes through an `on*` wrapper in `backend.ts` that returns `UnlistenFn`.

```rust
// src-tauri/src/lib.rs (setup)
HostTrustEvent::Request(prompt) => {
    let _ = handle.emit("ssh-host-trust-request", &prompt);
}
```

```ts
// src/lib/backend.ts
export function onSshHostTrustRequest(
  callback: (prompt: SshHostTrustPrompt) => void,
): Promise<UnlistenFn> {
  return listen<SshHostTrustPrompt>("ssh-host-trust-request", (event) => {
    callback(event.payload);
  });
}
```

Event names are kebab-case with a domain prefix (`ssh-*`, `native-*`). Emit results are
ignored with `let _ =` (see `native/session/events.rs` `native-text-delta`).

## End-to-End Example: `listActivityLogs`

1. **TS type** -- `src/lib/types.ts`:
   `export interface ActivityLog { id; kind; workspace_id: string | null; session_id; summary; payload_json; created_at }`
2. **TS wrapper** -- `src/lib/backend.ts`:
   ```ts
   export function listActivityLogs(workspaceId?: string, limit?: number): Promise<ActivityLog[]> {
     return invoke("list_activity_logs", { workspaceId, limit });
   }
   ```
3. **Registration** -- `src-tauri/src/lib.rs`: `app::activity_logs::list_activity_logs,`
4. **Command** -- `src-tauri/src/app/activity_logs.rs`:
   ```rust
   #[tauri::command]
   pub async fn list_activity_logs<R: Runtime>(
       app: AppHandle<R>,
       workspace_id: Option<String>,
       limit: Option<i64>,
   ) -> Result<Vec<ActivityLog>, String> {
       let pool = sqlite_pool(&app).await?;
       list_activity_logs_with_pool(&pool, workspace_id.as_deref(), limit.unwrap_or(50)).await
   }
   ```
5. **Service + DB** -- same file: `list_activity_logs_with_pool(&SqlitePool, Option<&str>, i64)`
   clamps the limit, runs `sqlx::query_as::<_, ActivityLog>("SELECT * FROM activity_logs ...")`
   and maps errors with `.map_err(|error| format!("读取活动日志失败: {error}"))`.
   `ActivityLog` derives `Serialize, Deserialize, FromRow`, so the same struct is the row
   and the IPC DTO.
6. **Test** -- `#[tokio::test] inserts_filters_and_limits_activity_logs` calls the
   `_with_pool` function against `db::test_support::setup_migrated_pool()`.

## Checklist for a New Command

- [ ] `#[tauri::command]` fn, `AppHandle<R>` + snake_case args, returns `Result<T, String>`
- [ ] Logic in a `&SqlitePool`-taking function with a unit test
- [ ] Added to `generate_handler![...]` in `src-tauri/src/lib.rs`
- [ ] Wrapper in `src/lib/backend.ts` (camelCase arg keys) + type in `src/lib/types.ts`
- [ ] If the docs table of commands matters to you, update `docs/architecture.md`
  (it is already incomplete -- e.g. `restart_app`, `list_remote_directories`,
  `get_git_file_preview`, `pull_git_branch` are registered but not listed)
