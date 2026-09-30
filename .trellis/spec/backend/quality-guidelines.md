# Quality Guidelines

> Lint, format, tests, and hard constraints for `src-tauri/`.

---

## Required Checks (run after every change)

From `AGENTS.md` ("每次写完代码都要运行检查命令") and `package.json`:

```bash
npm run lint:rust   # cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check      # default rustfmt, no rustfmt.toml
cargo test --manifest-path src-tauri/Cargo.toml
npm run format:check                                        # if backend.ts / types.ts changed
```

- Clippy runs with `--all-targets` (tests included) and `-D warnings`: any warning fails.
- There is no `rustfmt.toml` / `clippy.toml`; defaults apply.
- Existing `allow`s: `#[allow(clippy::too_many_arguments)]` (18 sites), item-level
  `#[allow(dead_code)]` (e.g. `process_spawn::configure_tokio_command` / `tokio_command`,
  `app/ssh/shell.rs`), and file-level `#![allow(dead_code)]` on legacy-port modules
  (`native/mod.rs`, `native/manager.rs`, `native/settings.rs`, `native/skills.rs`,
  `native/agent/mod.rs`, `db/models.rs`, ... 12 files). Do not add new crate-wide allows
  or other lint allows to silence clippy.

## Tests

- Unit tests live in the same file: `#[cfg(test)] mod tests { use super::*; ... }`
  (137 `#[cfg(test)]` sites). Large suites use a sibling file declared as
  `#[cfg(test)] mod tests;` (`git/tests.rs`, `native/session/tests.rs`,
  `native/attachments/tests.rs`, `native/agent/loop/tests.rs`).
- Async tests use `#[tokio::test]` (~440 sites). `db/migrations.rs` uses
  `tauri::async_runtime::block_on` inside `#[test]`.
- DB tests: `db::test_support::setup_migrated_pool()` (in-memory SQLite, all migrations,
  `PRAGMA foreign_keys = ON`). Test the `&SqlitePool` service function, not the command.
- SSH tests use the in-process server `app/ssh/test_server.rs` (`#[cfg(test)]`);
  integration-style tests are in `app/ssh/integration.rs`.
- Git tests (`git/tests.rs`) create real temp repos through `runner::fixture_git`
  (`#[cfg(test)]`), never by spawning `git` directly.
- Test-only hooks go behind `#[cfg(any(test, feature = "media-faults"))]`
  (`native/test_pause.rs`; feature declared in `Cargo.toml`).
- There is no `src-tauri/tests/*.rs` integration crate; `src-tauri/tests/fixtures/` holds data only.

## Hard Constraint 1: Only `git/runner.rs` Spawns `git`

`AGENTS.md`: "全仓库只允许 src-tauri/src/git/runner.rs spawn git". Enforced by
`git::runner::tests::only_runner_spawns_git`, which scans every `.rs` file under `src/`
for `Command::new("git")`, `std_command("git")`, `tokio_command("git")`.

Use the runner API; `IndexMode` is mandatory:

```rust
// src-tauri/src/git/runner.rs
pub(crate) enum IndexMode {
    ReadOnly,                       // adds --no-optional-locks; index-writing subcommands rejected
    UserIndex(UserIndexToken),      // real .git/index; token constructible only inside git/
    Scratch(ScratchIndex),          // GIT_INDEX_FILE temp index for checkpoints
}

pub(crate) async fn git(target: &GitTarget, args: &[&str], mode: &IndexMode)
    -> Result<GitOutput, GitError>
```

- `GitTarget` is `Local(PathBuf)` or `Ssh { pool, params, repo_path }`; the runner handles
  both (remote uses non-login `sh -c`, asserted by `wrap_ssh_script_uses_non_login_shell`).
- Every call gets `GIT_TERMINAL_PROMPT=0` and `LC_ALL=C`; local runs use
  `process_spawn::tokio_command(git_program())` with `kill_on_drop(true)`.
- Index-writing work (`UserIndex`, `Scratch`) goes through `with_repo_lock` (per-repo lock).
- Parse machine output with NUL separators (`status --porcelain=v2 --branch -z`,
  `split_nul_strings`), see `docs/git.md`.

## Hard Constraint 2: System git >= 2.23 Preflight

`git/preflight.rs` defines `MIN_GIT_VERSION = 2.23.0`. `run_startup_check()` is the first
line of `run()` in `lib.rs`; failure shows a Chinese dialog and exits (see
[Error Handling](./error-handling.md)). Remote SSH repos are checked separately
(`GitError::VersionTooOld`). System `git` is the only runtime external dependency --
do not add features that require Node, `ssh`, or other system binaries at runtime.

## Hard Constraint 3: Child Processes Go Through `process_spawn.rs`

`src-tauri/src/process_spawn.rs` (refactored in commit `f8a23ca`) is the constructor for
every non-git child process. API:

```rust
pub fn std_command(program: impl AsRef<OsStr>) -> StdCommand      // new + configure
pub fn tokio_command(program: impl AsRef<OsStr>) -> TokioCommand  // new + configure
pub fn configure_std_command(command: &mut StdCommand)            // for pre-built commands
pub fn configure_tokio_command(command: &mut TokioCommand)        // delegates via as_std_mut()
```

What `configure_std_command` does:

- Windows: `creation_flags(CREATE_NO_WINDOW)` (`0x0800_0000`) so no CMD window flashes.
- Under `#[cfg(test)]` only: `command.env("LC_ALL", "C")` so subprocess fixtures are
  locale-portable (asserted by `std_test_command_has_portable_locale_and_preserves_stderr`
  and its tokio twin).
- Otherwise a no-op; it never changes the parent process environment.

Usage patterns in the code:

- `tokio_command(...)` directly: `native/tools/local.rs`, `native/tools/sandbox.rs`,
  `native/tools/shell_snapshot.rs`, `native/commands.rs`, `native/tools/lsp.rs`.
- Build with `Command::new(path)` then call `configure_tokio_command(&mut command)` before
  spawn: `native/mcp_servers.rs` (`command_path::resolve_program` + `apply_augmented_path`),
  `native/tools/mcp.rs`.
- Long-running local children set `kill_on_drop(true)` (`git/runner.rs`,
  `native/mcp_servers.rs`).

Known existing deviations (runtime code calling `std::process::Command::new` without
`process_spawn`): `native/tools/local.rs` (`rg` search), `native/components.rs`
(`node_on_path`), `native/tools/sandbox.rs` (`probe_bwrap`, Linux-only). Do not copy these;
new code must use the helpers.

## Hard Constraint 4: `russh` Stays on `ring` (No `aws-lc-rs`)

`src-tauri/Cargo.toml`:

```toml
russh = { version = "0.63", default-features = false, features = ["ring", "flate2", "rsa"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
# dev-dependency: keep the same ring TLS backend as production
tokio-rustls = { version = "0.26", default-features = false, features = ["ring", "tls12"] }
```

`cargo tree --manifest-path src-tauri/Cargo.toml -i aws-lc-rs` must print
`did not match any packages` (verified on 2026-09-30). When adding or upgrading a crate
that touches TLS/crypto, set `default-features = false` and pick `ring` / `rustls-tls`,
then re-run the check. SSH is pure Rust -- never shell out to system `ssh` or use `ssh2`.

## Other Established Rules

- Version pins are deliberate (MSRV / external-dependency reasons, commented in
  `Cargo.toml`): `sqlx = "=0.8.6"`, `libsqlite3-sys = "=0.30.1"`, `xcap = "=0.4.1"`,
  `enigo = "=0.3.0"` (no `xdo`), `image = "=0.25.6"`. Do not bump casually.
- Remote shell commands must escape with `app::ssh::shell::shell_escape_single_quoted`
  (`docs/ssh.md`); SSH exec returns `SshCommandOutput`, not `std::process::Output`.
- Secrets: SSH passwords/passphrases go to keyring via `app/secret_store.rs`
  (service `noxcode-ssh`); `ai_channels.api_key` is stored in SQLite by design.
- App settings documents live as JSON in `$APPCONFIG` (`network-settings.json`,
  `native-settings.json`, `mcp-servers.json`, ...), not in SQLite.
- Window close hides to tray (`window_event.rs`); exit cleanup is in
  `app::lifecycle::handle_exit_requested`.

## Forbidden Patterns

| Pattern | Why | Instead |
|---------|-----|---------|
| `Command::new("git")` / `tokio_command("git")` outside `git/runner.rs` | breaks `only_runner_spawns_git`, bypasses index guard/lock | `runner::git(target, args, &IndexMode::...)` |
| Bare `std::process::Command::new(x)` in new runtime code | CMD window flashes on Windows | `process_spawn::{std_command, tokio_command}` |
| Any `sql:*` permission in `capabilities/default.json` | frontend could read/write SQLite | a Tauri command |
| Editing/inserting old migrations | breaks upgraded databases | append version N+1 |
| Crates enabling `aws-lc-rs` | build constraint | `ring` features |
| Mutating the real index from agent/tool code | pollutes the user's staging area | `IndexMode::Scratch` |

## Common Mistakes

- Registering a command but forgetting the `backend.ts` wrapper (or vice versa).
- New migration without bumping the `assert_eq!(latest_migration_version(), N)` test.
- Test that spawns a subprocess and depends on the developer's locale -- use
  `process_spawn` helpers, which force `LC_ALL=C` under test.
- Forgetting `--all-targets`: test code must be clippy-clean too.
