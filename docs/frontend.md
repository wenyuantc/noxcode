# 前端

P5 落地单主界面 + 全屏设置。布局学 ZCode：左侧两级树、输入框内控件、命令面板、会话页右侧 Git 抽屉。组件底座是 `@base-ui/react` + shadcn token，不引入 `cmdk` / Monaco。

数据流不变：`React → src/lib/backend.ts → Tauri command → Rust → SQLite`。Zustand 只缓存从 Rust 取回的状态。

## 路由

| 路径 | 页面 |
| --- | --- |
| `/` | `WorkspacePage`：`AppShell`（空态或会话流） |
| `/settings` | 重定向到 `/settings/general` |
| `/settings/:section` | 全屏设置，左导航三组 |
| `/api-logs` | API 调用记录 |

## 目录

```
src/main.tsx             i18n init → applyTheme → App
src/App.tsx              四条路由 + 权限 / 计划提问 / SSH 信任对话框
src/lib/backend.ts       唯一 IPC 出口
src/lib/apiLogs.ts       API 调用记录格式化 / 分页
src/lib/sessionLines.ts  行前缀解析、工具/结果配对、回合折叠
src/lib/gitHelpers.ts    暂存分组、diff 行着色
src/locales/{zh-CN,en}   九个命名空间
src/stores/              ui / workspace / channel / session / settings
src/hooks/               useNativeEvents · useSshTrustEvents · useAppHotkeys
src/components/ui/       17 个 shadcn 底座
src/components/layout/   AppShell · Sidebar*
src/components/session/  Composer · EventStream · pickers · 权限对话框
src/components/settings/ SettingsLayout + 分节
src/components/apiLogs/  调用详情弹层
src/components/git/      GitPanel · DiffView · CheckpointTimeline
```

## Store

| Store | 持久化键 | 职责 |
| --- | --- | --- |
| `uiStore` | `noxcode:sidebar-width`（200–480）、`noxcode:sidebar-collapsed`、`noxcode:composer-plan-mode`、`theme` / `theme-mode` | 侧栏、命令面板、Git 抽屉、Composer 草稿、计划模式、主题 |
| `workspaceStore` | `noxcode:active-workspace` | 工作区列表、会话树、健康检查 |
| `channelStore` | `noxcode:active-model` | 渠道列表、当前渠道/模型 |
| `sessionStore` | — | live 会话、事件行、turn-state、权限/提问 |
| `settingsStore` | — | native / network / 快捷提示 |

## 会话接线

`useNativeEvents` 在 `App` 挂一次，把 native 事件写入 `sessionStore`：

`native-session` / `native-stdout` / `native-text-delta` / `native-context-usage` / `native-turn-state` / `native-exit` / `native-permission-request` / `native-plan-question`。

Composer：无 live 会话走 `startNativeSession`（`ai_channel_id` + 模型）；有 live 走 `sendNativeInput`；停止走 `stopNativeSession`。权限菜单四项：变更前确认 / 自动编辑 / 计划模式 / 完全访问。前两项与完全访问写入 `permission_mode`；计划模式存在 `uiStore.composerPlanMode`，新会话传 `plan_mode`。模型控件是渠道→模型级联菜单，底部「管理模型」进 `/settings/channels`。历史会话点「继续对话」先 `prepareAgentSessionResume`，可续则 `resumeNativeSession`。`native-turn-state` 的 `waiting_input` / `working` 驱动发送按钮，不靠猜前缀。

`@` 调 `list_git_files` 插入 `@path`。`/` 列出全局技能，插入「使用技能：name」。

事件流：`[USER_INPUT]` 用户气泡；`[读取]` / `[写入]` / `[编辑]` / `[命令]` / `[工具]` / `[技能]` / `[待办]` / `[子 Agent…]` 为工具项，紧随的 `[工具结果]` 挂到该项；`[思考]` / `[PLAN]` / `[PERMISSION]` / `[MCP]` / `[续聊]` / `[重试]` / `[ERROR]` 为系统行。连续工具折叠成 WorkSummaryBar。`@tanstack/react-virtual` + `measureElement` 动态高度。

## 工作区 / 分支 / 命令面板

工作区选择器五条路径：搜索切换、打开文件夹（`plugin-dialog` `open({directory:true})` 后 `createWorkspace` local）、远程连接（选或建 SSH 配置 + 远端路径）、不在项目中工作（`ensureScratchWorkspace` → `$APPCONFIG/scratch` + 「临时工作区」）。

分支选择器：`listGitBranches` 搜索切换已有分支（`checkoutGitBranch` / `git switch`），以及「创建并检出」。点外或 Escape 关闭菜单。

命令面板：`Dialog` + 键盘导航，三类过滤（操作 / 最近会话 / `list_git_files`）。

## 设置

左导航三组：基础设置（general / appearance / channels / ssh）、Agent 能力（runtime / subagents / mcp / skills / hooks）、数据与统计（usage / database / about）。开关即时生效，文本输入配「保存」。渠道删除时若有 live session 则后端拒绝，错误原文展示。子智能体是列表 + 弹窗 CRUD，可配模型、工具与工作区作用域。MCP 为卡片列表，支持备注、删除、Playwright 预设和导出片段。

## Git 抽屉

会话页 `⌘⇧G` / SessionHeader 打开。变更 Tab：已暂存 / 未暂存 / 未跟踪，勾选后暂存、取消暂存、丢弃，`DiffView` 按行着色，提交 + 推送。检查点 Tab：`ref_valid=false` 标失效；回滚先 `previewGitCheckpointRestore`，展示将覆盖 / 将重建 / 不会自动删除，删除新建文件默认不勾，确认按钮危险色且非默认焦点。gitignore 文件后端永不删。

## 快捷键

| 键 | 动作 |
| --- | --- |
| `⌘N` | 新建会话（清选中，回空态） |
| `⌘K` | 命令面板 |
| `⌘O` | 打开工作区选择器 |
| `⌘B` | 折叠侧栏 |
| `⌘⇧G` | Git 抽屉 |
| `⌘,` | 设置 |

Windows / Linux 把 `⌘` 换成 Ctrl。定义在 `src/lib/shortcuts.ts`。

## i18n / 主题

`fallbackLng=zh-CN`，键 `noxcode:locale`。命名空间：`common` `nav` `layout` `sessions` `settings` `ssh` `git` `apiLogs` `errors`。`localeKeys.test.ts` 强制 zh-CN / en 键结构一致。

主题键 `theme-mode` / `theme`，与 `index.html` 防闪脚本一致。
