# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Frontend dev server only (no Rust backend)
npm run dev

# Full Tauri dev environment (frontend + backend)
npm run tauri:dev

# Build (TypeScript check + Vite bundle)
npm run build

# Lint / format
npm run lint              # ESLint (src + vite.config + vitest.config)
npm run lint:fix          # ESLint auto-fix
npm run format            # Prettier write
npm run format:check      # Prettier check
npm run lint:rust         # cargo clippy -D warnings

# Frontend tests (Vitest, node env, pure functions only)
npm test                  # watch mode
npm run test:ci           # single run — required locally before commit

# Rust backend tests
cargo test --manifest-path src-tauri/Cargo.toml
```

Lint gate: ESLint + Prettier + `npm run test:ci` + `cargo clippy --all-targets -- -D warnings`.

## Architecture

**noxcode** is a Tauri v2 desktop app: an in-process native Agent with AI channels, SSH workspaces, and Git checkpoints. Architecture: `docs/architecture.md`. Database schema: `docs/database.md`. SSH: `docs/ssh.md`. Channels: `docs/channels.md`. Full build plan: `plan.md`.

The data flow is strictly:

```
React (UI) → Tauri IPC commands → Rust service layer → SQLite
```

**All business writes go through Rust Tauri commands.** The frontend never writes directly to the database. Zustand stores only cache frontend state fetched from Rust.

**Frontend never touches SQLite.** All reads and writes go through Tauri commands via `src/lib/backend.ts`. `src/lib/database.ts` is a hard-fail stub (`select` / `execute` / `getDb` throw). `src-tauri/capabilities/default.json` does not grant any `sql:*` permissions (not even `sql:default`, which would include `allow-select`).

### Constraints

- SQLite lives at `$APPCONFIG/noxcode.db`. The baseline schema has 9 tables (including `git_checkpoints`). Migration versions in `db/migrations.rs` must stay contiguous `1..N`; this is enforced by `migration_versions_are_contiguous`.
- The only runtime external dependency is system `git` ≥ 2.23. Startup preflight fails hard below that.
- The entire repo may spawn `git` only from `src-tauri/src/git/runner.rs`. Other modules must go through that runner.
- `russh` must keep the `ring` backend (`default-features = false`, features `ring,flate2,rsa`). `cargo tree -i aws-lc-rs` must report no matches.

### Quality gate (must be green before commit)

```bash
npm run lint
npm run format:check
npm run test:ci
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```
