# Backend Development Guidelines

> Conventions for the Tauri 2 / Rust backend in `src-tauri/`.

---

## Overview

The backend is a single Rust crate (`noxcode_lib`, see `src-tauri/Cargo.toml`) that
owns every side effect of the desktop app: SQLite, system `git`, SSH (`russh`),
HTTP model APIs, local child processes, tray and window lifecycle.

The data-flow rule (from `AGENTS.md` and `docs/architecture.md`) is absolute:

```
React (UI) -> Tauri IPC commands -> Rust service layer -> SQLite
```

The frontend never touches SQL. `src/lib/database.ts` is a hard-fail stub and
`src-tauri/capabilities/default.json` grants no `sql:*` permission. Every
frontend call goes through `src/lib/backend.ts`.

Upstream Chinese docs (source of truth for product behaviour, but some are
stale -- see notes in each guide): `AGENTS.md`, `docs/architecture.md`,
`docs/database.md`, `docs/git.md`, `docs/ssh.md`, `docs/native.md`,
`docs/channels.md`.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module layout under `src-tauri/src`, where new code goes | Filled |
| [Command Guidelines](./command-guidelines.md) | `#[tauri::command]` shape, registration, `backend.ts` contract, events | Filled |
| [Database Guidelines](./database-guidelines.md) | `sqlite_pool`, sqlx runtime queries, migrations `1..N`, tests | Filled |
| [Error Handling](./error-handling.md) | `Result<T, String>`, thiserror enums, Chinese messages, `eprintln!` logging | Filled |
| [Quality Guidelines](./quality-guidelines.md) | clippy/rustfmt/tests, git runner, `process_spawn`, ring backend, forbidden patterns | Filled |

---

## Pre-Development Checklist

Before writing backend code, confirm:

- [ ] Read [Directory Structure](./directory-structure.md) to decide the module (`app/`, `git/`, `native/`, `db/`, `engine/`).
- [ ] New IPC command? Follow [Command Guidelines](./command-guidelines.md): add it to
      `tauri::generate_handler![...]` in `src-tauri/src/lib.rs`, add a wrapper in
      `src/lib/backend.ts`, add the TS type in `src/lib/types.ts`.
- [ ] Schema change? Follow [Database Guidelines](./database-guidelines.md): append the next
      contiguous version in `src-tauri/src/db/migrations.rs` and bump the hard-coded
      assertion in `migration_versions_are_contiguous`. Never edit a shipped migration.
- [ ] Spawning a process? `git` only via `src-tauri/src/git/runner.rs`; everything else via
      `crate::process_spawn` helpers ([Quality Guidelines](./quality-guidelines.md)).
- [ ] New dependency? It must not pull `aws-lc-rs` (`cargo tree -i aws-lc-rs` must report
      "did not match any packages").
- [ ] Errors surfaced to UI are user-facing Chinese strings ([Error Handling](./error-handling.md)).

## Verification Commands

```bash
npm run lint:rust        # cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo tree --manifest-path src-tauri/Cargo.toml -i aws-lc-rs   # must not match
```

Also run `npm run format:check` when `src/lib/backend.ts` / `src/lib/types.ts` change.

---

**Language**: All documentation should be written in **English**.
