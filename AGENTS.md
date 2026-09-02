# 仓库指南

## 项目结构与模块组织
该应用由 Vite/React 前端和 Tauri/Rust 桌面壳组成。前端源码位于 `src/`：页面级路由在 `src/pages`，可复用 UI 在 `src/components`，共享辅助函数在 `src/lib`，Zustand 状态在 `src/stores`。原生桌面端代码位于 `src-tauri/src`。架构见 `docs/architecture.md`，数据层见 `docs/database.md`，SSH 见 `docs/ssh.md`，Git 见 `docs/git.md`，渠道见 `docs/channels.md`，完整构建计划见 `plan.md`。

## 数据流铁律
```
React (UI) → Tauri IPC commands → Rust service layer → SQLite
```
前端永不直接读写 SQLite。`src/lib/database.ts` 是 hard-fail stub（`select` / `execute` / `getDb` 直接抛错）。`src-tauri/capabilities/default.json` 不授予任何 `sql:*` 权限（包括 `sql:default`，因为它含 `allow-select`）。所有读写必须走 `src/lib/backend.ts` 封装的 Tauri command。

## 开发约束
- 数据库迁移版本必须连续（`1..N`），由 `migration_versions_are_contiguous` 单测强制。baseline 共 9 张表（含 `git_checkpoints`）。
- 全仓库只允许 `src-tauri/src/git/runner.rs` spawn `git`。业务代码一律通过该 runner。
- `russh` 必须保持 ring 后端。`cargo tree -i aws-lc-rs` 必须无匹配。
- 运行时外部依赖只有系统 `git` ≥ 2.23；启动预检失败则弹中文错误并退出。

## 构建、测试与开发命令
- `npm run dev`：启动仅用于浏览器开发的 Vite 前端。
- `npm run build`：执行 TypeScript 编译并生成前端产物。
- `npm run lint` / `npm run format:check`：前端 ESLint 与 Prettier 检查。
- `npm run lint:rust`：`cargo clippy --all-targets -- -D warnings`。
- `npm run tauri:dev`：启动带 Rust 后端且前端支持热更新的桌面应用。
- `cargo test --manifest-path src-tauri/Cargo.toml`：运行 Tauri 层的 Rust 测试。

## 编码风格与命名约定
TypeScript 使用 2 空格缩进，Rust 使用默认 `rustfmt` 格式化。React 组件、页面和对话框文件使用 PascalCase 命名；store、工具函数和模块辅助文件使用 camelCase 命名。

## 每次写完代码都要运行检查命令

- `clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
- `npm run format:check`
