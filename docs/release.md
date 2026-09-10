# 打包与发版

P6 落地 updater、托盘、窗口状态、图标、版本同步脚本与 CI。私钥只存在本机与 GitHub secret，不进仓库。

## 打包命令

```bash
npm run tauri:dmg:no-sign   # macOS .app + .dmg，跳过 Apple 代码签名与 updater 签名
npm run tauri:dmg           # macOS，走系统签名（需要本机证书）
npm run tauri:windows       # NSIS + MSI
npm run tauri:linux         # AppImage + deb + rpm
```

`tauri.conf.json` 的 `bundle.createUpdaterArtifacts` 为 `true`。本地 `tauri:windows` / `tauri:linux` 会尝试生成 updater 产物；没有 `TAURI_SIGNING_PRIVATE_KEY` 会失败。`tauri:dmg:no-sign` 带 `--no-sign`，会跳过 Apple 代码签名和 updater 签名。

本机 `npm run tauri:dmg:no-sign`（2026-09-03，aarch64）产物：release 二进制约 28MB，`.app` 约 28MB，`.dmg` / `.app.tar.gz` 约 11MB。updater 会再拉一份 reqwest 0.13，与业务 0.12 共存；当前体积可接受。

## 签名密钥

```bash
npx tauri signer generate -w ~/.tauri/noxcode-updater.key --ci
```

| 文件 | 用途 |
| --- | --- |
| `~/.tauri/noxcode-updater.key` | 私钥。只放本机，写入 GitHub secret `TAURI_SIGNING_PRIVATE_KEY` |
| `~/.tauri/noxcode-updater.key.pub` | 公钥。内容写进 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey` |

当前 endpoint：`https://github.com/wenyuantc/noxcode/releases/latest/download/latest.json`。

**不能复用 codex-ai 的密钥或 endpoint。** 私钥丢失后无法再签更新包，必须重新生成密钥对并发布一个强制重装的版本。

GitHub secrets：

| Secret | 说明 |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | tag 发版必需。私钥文件内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 可选。本仓库密钥按 `--ci` 生成，无密码 |

配置示例：

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/noxcode-updater.key
```

`workflow_dispatch` 且未配置私钥时仍只打安装包，不生成 updater 产物。

## Tag 发版

1. `npm run bump-version -- <x.y.z>` 同步 `package.json` / `package-lock.json` / `src-tauri/Cargo.toml` / `src-tauri/Cargo.lock` / `src-tauri/tauri.conf.json`
2. 提交并打 tag：`git tag v0.1.0 && git push origin v0.1.0`
3. [`.github/workflows/build.yml`](../.github/workflows/build.yml) 在 Windows / Linux / macOS Apple Silicon（`aarch64-apple-darwin`）/ macOS Intel（`x86_64-apple-darwin`）打安装包
4. 构建完成后创建 GitHub Release，挂载安装包，并用 [`scripts/build-latest-json.mjs`](../scripts/build-latest-json.mjs) 生成 `latest.json`（`darwin-aarch64` 与 `darwin-x86_64` 分开）

客户端启动后从 `releases/latest/download/latest.json` 检查更新；Apple Silicon 与 Intel 各走对应产物。开发模式（`tauri dev` / 浏览器）不能检查或安装更新。

## 托盘与窗口

关闭主窗口、托盘「退出」、`Cmd+Q` 都会写入 `$APPCONFIG/window-state.json`（物理像素 + 内存快照，含窗口位置）。启动时 `RunEvent::Ready` 恢复尺寸与位置（坐标校验失败时居中兜底），显示后再落一次几何。关闭主窗口后隐藏到托盘，不退出进程。托盘左键或菜单「显示窗口」恢复；macOS 点 Dock 图标触发 `RunEvent::Reopen`，同样恢复主窗口。

退出由 `app::lifecycle` 协调：`Running → Draining → Exiting`，首次请求决定退出或重启，重复请求不重复清理。更新重启通过 `restart_app` 命令进入后台清理，最后调用 `request_restart()`；普通退出在 `ExitRequested` 暂缓，清理后重新发出退出。`Exit` 不再等待业务资源，前端不授予 process restart 权限。

更新重启总清理预算 5 秒：窗口保存、会话持久化与 MCP 收尾最多 3 秒，SSH 与数据库各最多 1 秒，提前完成即继续。普通退出总预算 30 秒，保留空闲会话的记忆收尾机会；快速重启跳过记忆提取。退出期间停止自动化、新会话、追加输入与新 SSH 连接，所有会话同时收到结束信号，超时取消并中止剩余任务，不无限等待 join。SQL 插件在清理开始时注销，其已创建的连接池保留到最后限时关闭，避免插件的 `Exit` 钩子重复阻塞。预算不包含操作系统启动新进程的耗时；超时前未持久化的内存内容可能无法完整保存。

诊断日志位于 `$APPLOG/lifecycle-<pid>.jsonl`，包含版本、进程、启动与窗口就绪、清理阶段、耗时及超时信息。日志独立于数据库，后台写入，每个文件最多约 1 MiB 并保留一份轮转文件，日志写入失败不阻塞退出。

命令：`show_main_window`、`restart_app`。

## 图标

源图是 [`src-tauri/app-icon.svg`](../src-tauri/app-icon.svg)（深色圆角方块 + path 字形，不用 `<text>`）。重新生成：

```bash
npx tauri icon src-tauri/app-icon.svg
rm -rf src-tauri/icons/android src-tauri/icons/ios
```
