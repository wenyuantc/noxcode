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
3. [`.github/workflows/build.yml`](../.github/workflows/build.yml) 在 Windows / Linux / macOS 打安装包
4. 三端完成后创建 GitHub Release，挂载安装包，并用 [`scripts/build-latest-json.mjs`](../scripts/build-latest-json.mjs) 生成 `latest.json`

客户端启动后从 `releases/latest/download/latest.json` 检查更新。开发模式（`tauri dev` / 浏览器）不能检查或安装更新。

## 托盘与窗口

关闭主窗口会写入 `$APPCONFIG/window-state.json` 并隐藏到托盘，不退出进程。托盘左键或菜单「显示窗口」恢复；「退出」走 `app.exit(0)`，`RunEvent::Exit` 里关闭 SshPool 并取消 Agent。macOS 点 Dock 图标触发 `RunEvent::Reopen`，同样恢复主窗口。

命令：`show_main_window`。

## 图标

源图是 [`src-tauri/app-icon.svg`](../src-tauri/app-icon.svg)（深色圆角方块 + path 字形，不用 `<text>`）。重新生成：

```bash
npx tauri icon src-tauri/app-icon.svg
rm -rf src-tauri/icons/android src-tauri/icons/ios
```
