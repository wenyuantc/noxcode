# Native Agent 运行时

P4 把进程内编程 Agent 接到渠道 + 工作区外壳。数据流仍是 `React → Tauri IPC → Rust → SQLite`。前端只通过 [`src/lib/backend.ts`](../src/lib/backend.ts) 调命令、听事件。

## 目录

| 路径 | 职责 |
| --- | --- |
| `src-tauri/src/native/model/` | 三协议 HTTP 客户端、SSE、usage、call log |
| `src-tauri/src/native/tools/` | 本地 / SSH 工具、MCP、权限、hooks |
| `src-tauri/src/native/agent/` | 主循环、压缩、截断、子 Agent |
| `src-tauri/src/native/session.rs` | 启动 / 停止 / 续聊 / 权限 / 计划提问 |
| `src-tauri/src/native/manager.rs` | 运行中会话、同一工作区单会话门控 |
| `src-tauri/src/native/prompt/` | identity + 环境 / Git / 项目指令 |
| `src-tauri/src/app/workspaces.rs` | 工作区 CRUD 与健康检查 |
| `src-tauri/src/app/sessions.rs` | 历史会话列表、日志、续聊判定、删除 |

## 会话生命周期

1. `has_workspace_processes`：同一工作区同时只允许一个 live session。
2. 解析工作区执行上下文（本地目录或 SSH 远端路径）。
3. 读取渠道，允许本次覆盖 model / effort / system_prompt。
4. 建 `ModelClient`（渠道密钥 + 网络设置 + SQLite call log）。
5. 插入 `agent_sessions`（`status=running`），发出 `native-session`。
6. 组装系统提示：identity → 子 Agent 策略 → 环境 → Git → 全局模板 → `AGENTS.md` / `CLAUDE.md` → skills。
7. 若工作区是 git 仓：`create_checkpoint(kind=session_start)`，失败只打日志。
8. `Write` / `Edit` / `ApplyPatch` 成功后异步 `create_checkpoint(kind=after_tool_call)`，同一会话同时只允许一个在途打点。
9. `run_native_loop` 转发 stdout / delta / context usage / 权限 / 计划提问；退出时写 tokens、status、`native-exit`，并从 manager 移除。

`session_kind` 只有 `execution` 与 `plan`。`plan_mode=true` 时先只读规划，本轮结束后自动放开写工具并继续实施。计划模式由启动参数决定，不写入 `native-settings.json`。

权限模式（`permission_mode`）三档：`confirm` 变更前确认；`auto_edit` 自动编辑（只放行 `Overwrite`，删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍弹确认）；`full` 完全访问（`allow_all_high_risk=true`）。旧文件的 `confirm_high_risk: false` 读成 `full`。

并发门控按工作区。`send_native_input` / `finish_native_input` 按 `session_record_id` 寻址。

## 命令

会话：`start_native_session`、`stop_native_session`、`stop_native`、`restart_native_session`、`resume_native_session`、`send_native_input`、`finish_native_input`、`resolve_native_tool_permission`、`answer_native_plan_question`。

工作区 / 历史：`list/create/update/delete_workspace`、`check_workspace_health`、`list_agent_sessions`、`get_agent_session_log_lines`、`prepare_agent_session_resume`、`delete_agent_session`。

设置：`get/update_native_settings`、`list_native_global_skills`、`open_native_skills_dir`、`list/create/update/delete_native_subagent`、`get/update/reset_mcp_servers`、`export_mcp_servers_snippet`、`list/get_native_api_call_log`。

删除工作区前会 `clear_workspace_checkpoints`。删除会话前会 `delete_checkpoints_for_session`。运行中的渠道 / 工作区 / 会话拒绝删除。

## 事件

| 事件 | 载荷 |
| --- | --- |
| `native-session` | `AgentSessionStarted` |
| `native-stdout` | `AgentSessionOutput`（已写入 `agent_session_events`） |
| `native-text-delta` | `NativeTextDelta`（仅展示，不落库） |
| `native-context-usage` | `NativeContextUsage` |
| `native-turn-state` | `NativeTurnState`（`waiting_input` / `working`，不落库） |
| `native-permission-request` | 高风险工具确认 |
| `native-plan-question` | 计划模式提问 |
| `native-exit` | `AgentSessionExit` |

前端监听：`onNativeStdout` / `onNativeExit` / `onNativeSession` / `onNativeTextDelta` / `onNativePermissionRequest` / `onNativePlanQuestion` / `onNativeContextUsage` / `onNativeTurnState`。接线见 [`frontend.md`](frontend.md)。

## 设置文件

都在 `$APPCONFIG`：

- `native-settings.json`：轮次、`permission_mode`、权限超时、子 Agent 策略、`global_prompt_template`
- `native-subagents.json`：自定义子智能体（`scope=all|workspaces`）
- `mcp-servers.json`：全局 MCP；会话只连接 `enabled=true` 的服务器（工作区绑定留 v2）
- `native-skills/`：全局技能；工作区另读 `.agents/skills` 与 `.claude/skills`

MCP 连接失败只写警告行，不中断会话。SSH 工作区在远端拉起 MCP，失败不回退本机。

## 六条链路手工验证

`npm run tauri:dev` 后在控制台依次 `invoke`：

1. `create_workspace`（本地 git 仓，或 SSH 仓 + `ssh_config_id` / `remote_repo_path`）
2. `create_ai_channel`
3. `start_native_session`，prompt：`读一下 README.md 并总结，然后在末尾追加一行`

分别跑 openai / anthropic / codex × 本地 / SSH，共六条。期望：

- 出现 `[读取]` 类 stdout
- 高风险写文件弹出确认：允许一次、拒绝一次
- `git_checkpoints` 出现 `session_start` 与 `after_tool_call`
- 最终有汇报文本
- `stop_native_session` 后 `resume_native_session` 能续聊

临时脚本与夹具只放 `/tmp`，不进仓库。本阶段无会话 UI（P5）。
