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
src/main.tsx             i18n init → applyTheme / applyCodeAppearance → App
src/App.tsx              四条路由 + 权限 / SSH 信任对话框
src/lib/backend.ts       唯一 IPC 出口
src/lib/appUpdate.ts     检查 / 下载 / 重启桌面更新
src/lib/apiLogs.ts       API 调用记录格式化 / 分页
src/lib/sessionLines.ts  行前缀解析、结构化 tool 信封、call_id / 子 Agent 分桶配对、时序 segments（同一回合按编号折叠进首个窗口，即使事件交错）/ 待办解析
src/lib/planMode.ts      按会话解析 Composer 的运行时 / 默认计划模式
src/lib/gitHelpers.ts    暂存分组、diff 行着色
src/lib/codeAppearance.ts 界面字号 / 代码主题 / 行号 / 换行 / 代码字号
src/lib/codeThemes.ts    10 个 Shiki 代码主题注册
src/lib/codeLanguage.ts  从 className / 路径推断语言
src/lib/diffLineNumbers.ts unified diff 新旧行号
src/lib/codeHighlighter.ts Shiki 单例高亮
src/locales/{zh-CN,en}   九个命名空间
src/stores/              ui / workspace / channel / session / settings / update
src/hooks/               useNativeEvents · useSshTrustEvents · useAppHotkeys
src/components/ui/       17 个 shadcn 底座
src/components/layout/   AppShell · Sidebar*
src/components/session/  Composer · EventStream · 回合行 / 计划卡 / Ask 卡 · 待办面板 · pickers · 权限对话框
src/components/settings/ SettingsLayout + 分节
src/components/apiLogs/  调用详情弹层
src/components/code/     CodeBlock · CodePreview
src/components/git/      GitPanel · DiffView · CheckpointTimeline
```

## Store

| Store | 持久化键 | 职责 |
| --- | --- | --- |
| `uiStore` | `noxcode:sidebar-width`（200–480）、`noxcode:sidebar-collapsed`、`noxcode:composer-plan-mode`、`noxcode:composer-thinking-level`、`theme` / `theme-mode`、`noxcode:ui-font-size`、`noxcode:code-theme-light` / `noxcode:code-theme-dark`、`noxcode:code-line-numbers`、`noxcode:code-soft-wrap`、`noxcode:code-font-size` | 侧栏、命令面板、Git 抽屉、Composer 草稿、计划模式、思考等级、主题、界面字号、代码外观 |
| `workspaceStore` | `noxcode:active-workspace` | 工作区列表、会话树、健康检查 |
| `channelStore` | `noxcode:active-model` | 渠道列表、当前渠道/模型 |
| `sessionStore` | — | live 会话、事件行、turn-state、按会话计划模式、权限/提问 |
| `settingsStore` | — | native / network / 快捷提示 |
| `updateStore` | — | 桌面更新检查 / 下载 / 重启；侧栏按钮与关于页共用 |

## 会话接线

`useNativeEvents` 在 `App` 挂一次，把 native 事件写入 `sessionStore`：

`native-session` / `native-stdout` / `native-text-delta` / `native-context-usage` / `native-turn-state` / `native-plan-mode` / `native-exit` / `native-permission-request` / `native-plan-question` / `native-plan-approval-request`。

打开历史会话立即改 `selectedSessionId`（侧栏高亮）；`lines` 已缓存则不重拉，无缓存拉最近 200 条。事件流只在当前选中会话的 `lines` 就绪后切换；加载期间隐藏旧会话流并显示加载态，避免侧栏、标题和输入框指向 A、内容仍显示 B。已打开的流保活最近 3 个（`hidden` 不卸 DOM）。live 行只走 `onStdout`（payload 可带 `tool` / `images`），历史结果不得覆盖已有缓存。`ensureHistory` 会解开 `{"nox":1,...}` 信封；旧会话仍按中文前缀文本兜底（FIFO + 子 Agent 分桶）。

Composer / 编辑重发 / 重试走 `submitSessionPrompt`：有选中会话则 `resumeNativeSession`（同一 `session_record_id`），首页无选中才 `startNativeSession`。Rust 判断该 ID 是 live 投递还是原位静默恢复 transcript，不把恢复过程写进聊天；前端不以 `liveBySession` 是否过期决定是否新建。历史里已落下的 `[续聊]` / 「会话已恢复|已创建」/ `[ERROR] 已取消` 行不渲染。手动停止只保留「收到停止请求」，不把取消标成错误。`sessionStore.liveBySession` 按 `session_record_id` 索引；停止按钮仍只看当前会话是否 live。权限菜单四项：变更前确认 / 自动编辑 / 计划模式 / 完全访问。前两项与完全访问写入 `permission_mode`；Composer 使用按会话解析的有效计划模式：活动会话收到 `native-plan-mode` 后优先于全局 `uiStore.composerPlanMode`，未知会话再回退到全局默认；全局默认仍写入 localStorage，新会话传 `plan_mode`。模型控件是渠道→模型级联菜单，底部「管理模型」进 `/settings/channels`。思考等级存在 `uiStore.composerThinkingLevel`（首页 Composer 与会话页 Composer 共享），解析顺序为用户选择 → 模型默认 → `medium` / 第一项，避免 DeepSeek 这类无 `medium` 的模型在切到进行中时掉回 `low`。思考等级右侧是 `ContextCapacity`：按 `selectedSessionId` 读 `sessionStore.usage`，不要求会话 live。`used/limit` 含工具 schema，弹层展示分类占比与上次调用缓存率（`cached/prompt`）。打开历史会话时 `ensureHistory` 从 `agent_sessions.context_usage_json` 灌入；没有快照则用最后一条 `[用量]` 加当前模型窗口兜底。`native-turn-state` 的 `waiting_input` / `working` 驱动发送按钮（发送中与工作中显示转圈并禁用）和停止按钮（仅 `working` 显示），不靠猜前缀。思考结束后事件行写入完整 `reasoning`，不再只记「已生成 N 字」。

`@` 调 `list_git_files` 插入 `@path`。`/` 分组列出内置/自定义命令、当前工作区与全局技能、子智能体；`$` 只列技能。技能插入 `$name`，发送 `/skill name` 或 `$name` 时会改写成「先调用 Skill 工具」提示；命中的自定义 `/command` 经 `expand_native_slash_command` 展开。提及列表支持 ↑↓ 选择、Enter / Tab 插入、Escape 关闭。输入框内 `Shift+Tab` 进入计划模式；已在计划模式则退回此前的权限模式。

事件流按时序渲染 `TurnSegment`（思考 / 查阅汇总 / 终端 / 写入或更改 / 待办摘要 / 计划卡 / 用量芯片 / 助手 markdown / 其它系统行），不再把工具、思考、正文拆成三个乱序数组。`[PLAN]` / `[计划]` 收成计划状态行或 Markdown 计划卡；`AskUserQuestion` 与 `ExitPlanMode` 批准都内嵌在时间线（`PlanAskCard` / 计划卡胶囊按钮），不再走全局 Dialog。用户气泡悬停复制（成功打钩约 2 秒）；助手回合底栏复制同样打钩后约 2 秒恢复。仅最后一条用户消息可原位编辑并重发（同样走 `submitSessionPrompt`，复用当前会话 ID），不改历史行。会话开头的 `[PERMISSION]` / `[内置 Agent]` / `[MCP]` 并入第一条用户回合，不单独占一轮「已工作 0 秒」。启动行收成带图标的状态提示，不改后端原文。查阅展开后直接出 cyan 路径 + 行号卡片，不再二次折叠。`[用量]` 解析成输入/输出/缓存芯片。工作头显示「已工作 / 工作中 N 秒」。可解析的 `[待办]` 只在流里一行摘要，完整清单叠在会话列右上角 `TodoProcessPanel`（宽卡 / 窄窗胶囊）。已结束回合若本回合有 Write/Edit/ApplyPatch 路径，底部「N 个文件已更改」卡片只列这些路径；点文件或「审查」打开 Git 抽屉，用现有 `DiffView` 预览。会话标题来自 `agent_sessions.title`（侧栏 / 命令面板 / SessionHeader），空则仍显示「会话」或 `Plan`。侧栏对 `turnState === "working"` 的会话在标题左侧显示旋转星标，未选中的后台会话同样显示。侧栏顶部「已置顶」列出 `pinned != 0` 的会话；悬停未置顶行显示图钉，点击 `set_agent_session_pinned` 后从工作区分组抽到顶部，再点取消回原工作区。每个工作区默认 5 条未置顶会话，「显示更多」每次 +10。`@tanstack/react-virtual` + `measureElement` 动态高度。

## 工作区 / 分支 / 命令面板

工作区选择器五条路径：搜索切换、打开文件夹（`plugin-dialog` `open({directory:true})` 后 `createWorkspace` local）、远程连接（选或建 SSH 配置 + 远端路径）、不在项目中工作（`ensureScratchWorkspace` → `$APPCONFIG/scratch` + 「临时工作区」）。打开历史会话时，若该会话有 `workspace_id` 且与当前不同，则把 `activeWorkspaceId` 同步到该工作区。

侧栏工作区行悬停显示操作菜单，可重命名或删除。删除先弹不可撤销确认；后端发现该工作区仍有运行中的会话时拒绝删除并把错误展示给用户。

分支选择器：`listGitBranches` 搜索切换已有分支（`checkoutGitBranch` / `git switch`），以及「创建并检出」。点外或 Escape 关闭菜单。

命令面板：`Dialog` + 键盘导航，三类过滤（操作 / 最近会话 / `list_git_files`）。

## 设置

左导航三组：基础设置（general / appearance / channels / ssh）、Agent 能力（runtime / subagents / mcp / skills / hooks）、数据与统计（usage / database / about）。开关即时生效，文本输入配「保存」。通用设置提供桌面通知开关；Native runtime 提供工具后自动 checkpoint 开关和保留天数（`0` 不清理）；SSH 编辑弹窗提供 KEX / Host Key / Cipher / MAC 高级算法区，以及旧服务器预设 / 恢复默认；MCP 卡片可在全部工作区和指定工作区之间切换并勾选绑定工作区。

渠道删除时若有 live session 则后端拒绝，错误原文展示。子智能体是列表 + 弹窗 CRUD，可配模型、工具与工作区作用域。MCP 还支持备注、删除、Playwright 预设和导出片段。数据库维护展示路径与迁移版本、备份范围（SQL 本体 vs 配置目录/密钥环），并提供导出 / 导入 SQL 与打开数据库目录。关于页用 `getVersion()` 显示真实版本；进入该页会自动检查更新，也可手动再检查 / 下载 / 重启。开发模式会提示无法检查。应用启动后静默检查；有新版本时首页侧栏底部显示「更新 / 下载中 / 重启更新」，与关于页共用 `updateStore`。

## Git 抽屉

会话页 `⌘⇧G` / SessionHeader 打开。变更 Tab：已暂存 / 未暂存 / 未跟踪，勾选后暂存、取消暂存、丢弃，`DiffView` 按行着色，提交 + 推送。检查点 Tab：`ref_valid=false` 标失效；可经确认清除本仓库全部 checkpoint，并在可折叠的「回滚记录」中查看回滚成功 / 失败审计。回滚先 `previewGitCheckpointRestore`，展示将覆盖 / 将重建 / 不会自动删除；同工作区有 `working` 会话时预览和执行都会被后端拦截。删除新建文件默认不勾，确认按钮危险色且非默认焦点，gitignore 文件后端永不删。

## 快捷键

| 键 | 动作 |
| --- | --- |
| `⌘N` | 新建会话（清选中，回空态） |
| `⌘K` | 命令面板 |
| `⌘O` | 打开工作区选择器 |
| `⌘B` | 折叠侧栏 |
| `⌘⇧G` | Git 抽屉 |
| `⌘,` | 设置 |
| `Shift+Tab` | Composer 内切换计划模式 / 还原此前权限模式 |

Windows / Linux 把 `⌘` 换成 Ctrl。定义在 `src/lib/shortcuts.ts`。`Shift+Tab` 只绑在 Composer 输入框，不进全局热键表。

## i18n / 主题

`fallbackLng=zh-CN`，键 `noxcode:locale`。命名空间：`common` `nav` `layout` `sessions` `settings` `ssh` `git` `apiLogs` `errors`。`localeKeys.test.ts` 强制 zh-CN / en 键结构一致。

主题键 `theme-mode` / `theme`，与 `index.html` 防闪脚本一致。代码外观键 `noxcode:ui-font-size`、`noxcode:code-theme-light`、`noxcode:code-theme-dark`、`noxcode:code-line-numbers`、`noxcode:code-soft-wrap`、`noxcode:code-font-size` 同样走 localStorage；启动时写入 `--ui-font-size` / `--code-font-size`。围栏代码、文件读取预览、Git diff 与 API 日志 JSON 经 `CodeBlock` / Shiki 使用当前浅色或深色代码主题。
