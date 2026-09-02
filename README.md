# noxcode

桌面端 AI coding 工具：内置 Agent、AI 渠道、SSH 工作区、Git checkpoint。基于 Tauri 2 + React 19。

完整构建计划见 [plan.md](plan.md)。

## 要求

- Node.js 22+
- Rust 1.77.2+（本仓库按 rustc 1.94 验证）
- 系统 `git` ≥ 2.11

## 命令

```bash
npm install
npm run tauri:dev
npm run lint
npm run format:check
npm run test:ci
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```
