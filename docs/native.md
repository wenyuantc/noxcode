# Native Agent 运行时

P4 把进程内编程 Agent 接到渠道 + 工作区外壳。数据流仍是 `React → Tauri IPC → Rust → SQLite`。前端只通过 [`src/lib/backend.ts`](../src/lib/backend.ts) 调命令、听事件。

## 目录

| 路径 | 职责 |
| --- | --- |
| `src-tauri/src/native/model/` | 三协议 HTTP 客户端、SSE、usage、call log |
| `src-tauri/src/native/tools/` | 本地 / SSH 工具、MCP、权限、hooks |
| `src-tauri/src/native/agent/` | 主循环、压缩、截断、子 Agent |
| `src-tauri/src/native/session.rs` | 启动 / 停止 / 原位继续 / 权限 / 计划提问 |
| `src-tauri/src/native/manager.rs` | 运行中会话（同一工作区可多个 live） |
| `src-tauri/src/native/prompt/` | identity + 环境 / Git / 项目指令 |
| `src-tauri/src/app/workspaces.rs` | 工作区 CRUD 与健康检查 |
| `src-tauri/src/app/sessions.rs` | 历史会话列表、日志、续聊判定、删除 |
| `src-tauri/src/app/notifications.rs` | 主窗口未聚焦时的桌面通知 |

## 会话生命周期

1. 同一工作区可以同时有多个 live session。`agent_sessions` 是可多次激活的逻辑会话：`resume` / `restart` / 会话内继续发送都复用同一 `session_record_id`，不为已有会话插入新行。删除工作区仍要求该工作区没有 live。
2. 解析工作区执行上下文（本地目录或 SSH 远端路径）。
3. 读取渠道，允许本次覆盖 model / effort / system_prompt。
4. 建 `ModelClient`（渠道密钥 + 网络设置 + SQLite call log）。
5. 无 `resume_session_id` 时插入 `agent_sessions`（`status=running`），并写出一次启动状态（渠道 banner / 权限说明 / MCP 状态）。有 `resume_session_id` 时：runtime 仍在则把 prompt 投到同一 live 的 `followup_tx`；runtime 已不在则校验工作区后原位重激活（刷新 `started_at` / 渠道 / 执行上下文，清空 `ended_at` / `exit_code`，保留 ID、标题、置顶、`created_at`、累计 token、旧事件和 checkpoint），并静默从同一 ID 的 transcript 恢复。冷启动不把「续聊 / 已恢复」或重复启动状态写进聊天；MCP 连接失败仍写出。发出 `native-session`。
6. 组装系统提示：identity → 子 Agent 策略 → 环境 → Git → 全局模板 → `AGENTS.md` / `CLAUDE.md` → skills。
7. 若工作区是 git 仓：`create_checkpoint(kind=session_start)`，失败只打日志。
8. `auto_checkpoint_after_tool_call=true` 时，`Write` / `Edit` / `ApplyPatch` 成功后异步 `create_checkpoint(kind=after_tool_call)`，同一会话同时只允许一个在途打点；关闭开关不影响会话开始或回滚前检查点。
9. 按当前 `workspace_id` 筛选并连接 `enabled=true` 且 `scope=all` 或命中 `scope=workspaces` / `workspace_ids` 的 MCP server。
10. `run_native_loop` 转发 stdout / delta / context usage / 权限 / 计划提问；退出时写 tokens、status、`native-exit`，并从 manager 移除。主窗口未聚焦且 `desktop_notifications=true` 时，会话结束 / 失败、权限确认和计划问题会发桌面通知。托盘 / 进程退出走 `shutdown_all_sessions`：拒绝待确认、cancel、`Finish`、await join，再关 SSH pool。

`session_kind` 只有 `execution` 与 `plan`。`plan_mode=true` 时先只读规划，本轮结束后自动放开写工具并继续实施。计划模式由启动参数决定，不写入 `native-settings.json`。

权限模式（`permission_mode`）四档，对齐 ZCode：`default` 变更前确认；`edit` 自动放行 `Overwrite`（删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍弹确认）；`build` 再放行不透明 shell 与带 `readOnlyHint` 的 MCP；`yolo` 完全访问（`allow_all_high_risk=true`，只有 ask 规则仍会确认）。旧文件的 `confirm / auto_edit / full` 与 Claude Code 的 `acceptEdits / auto / bypassPermissions / dontAsk` 读入时映射到新名；`confirm_high_risk: false` 读成 `yolo`。`plan` 是会话态：既可由 Composer 选择在启动时进入，也可由模型调用 `EnterPlanMode` 进入；`ExitPlanMode` 提交计划触发 `native-plan-approval-request`，用户批准后恢复执行模式，退回则连同反馈交回模型继续修改。

## 权限规则

规则层在风险分类之前裁决：`deny → allow → ask → 未命中`（对齐 ZCode 的 `denyPriority: beforeAsk`）。每条规则 `{ capability, pattern, source, scope, note }`：`capability` 是契约里的能力（bash / edit / read / mcp / web_fetch / …），`source` 决定从调用里取哪个字段匹配（`command` 前缀通配如 `git push*`；`path` / `tool_name` / `input` 用 glob），`scope` 决定落盘位置：全局 `$APPCONFIG/native-permissions.json`，工作区 `.noxcode/permissions.json`（只对本地工作区生效，工作区规则排在全局之前）。

- 确认对话框多出「总是允许 `<pattern>`（保存规则）」：后端按 `suggested_rule`（Bash 取前两个词做前缀、文件工具取相对路径、其余取工具名）写一条工作区 allow 规则并即时生效。
- `ask` 规则命中时即便在 `yolo` 也会弹确认（`kind = rule`）。
- 子 Agent 档案可带 `permission_mode`（不共享父会话的放行开关）与 `disallowed_tools`。
- 命令：`get/update/add/delete_native_permission_rules`；设置页「权限规则」可增删规则。

`send_native_input` / `finish_native_input` 按 `session_record_id` 寻址。`resume_native_session` 若源会话仍在跑，则向同一 live 投递输入；进程不在则原位静默恢复 transcript。同一会话继续发送不是单独的「续聊」产品流程。手动停止写「收到停止请求」，`已取消` 不算失败、不写 `[ERROR] 已取消`。

## 上下文持久化

`agent_session_events` 只服务 UI 回放；模型续聊只读 `native_session_transcripts`。两表没有数据库级同步约束。

顶层 runner 在这些边界同步 UPSERT transcript（fingerprint 未变则跳过）：

- 用户消息进入 `messages` 之后、下一次模型调用之前
- live followup / steer 注入之后
- 每一轮 assistant 文本，或 assistant + 对应 tool 结果写完整之后
- `run_native_loop` 退出前再 flush 一次（覆盖错误 / 取消）

保存前会去掉 system、图片，并清洗孤立 tool pair。子 Agent 不写父会话 transcript。硬中断时至少能恢复当前用户任务和已完成的模型 / 工具轮次。

## 工具契约与结果预算

每个内置工具在 [`tools/catalog.rs`](../src-tauri/src/native/tools/catalog.rs) 声明一份 [`ToolContract`](../src-tauri/src/native/tools/contract.rs)：`read_only / destructive / concurrent_safe / side_effect_scope / risk_level / needs_approval / allowed_in_plan_mode / permission（能力）/ pattern_sources / result_budget / timeout`。MCP 工具按 `tools/list` 返回的 `annotations.readOnlyHint / destructiveHint` 动态生成契约，缺省视为需审批、串行。

- 计划模式与 explore 子 Agent 的只读白名单来自契约的 `allowed_in_plan_mode`，不再硬编码。
- 同一轮里连续的 `concurrent_safe && !destructive && !needs_approval` 调用（Read / Glob / Grep / WebFetch / WebSearch / Skill / TodoRead）并行执行，上限 8，结果按模型给出的顺序回填；写工具与 Bash 串行；连续 `Agent` 调用仍成批并行。
- 结果预算：输出超过 `result_budget.max_model_bytes` 且策略为 `Artifact` 时，完整内容写入 `$APPCONFIG/artifacts/<session>/<id>.txt` 并登记 `native_tool_artifacts`，模型只看到头（Glob / Grep / WebFetch / Agent / MCP）或尾（Bash）预览加 artifact 路径；`Read` 允许读取 artifact 目录。之后仍按 `max_tool_output_tokens` 截断兜底。
- 逐工具超时：Read / Write / Edit / Glob / Skill / Todo 30 秒，Grep 60 秒，ApplyPatch 60 秒，WebFetch / WebSearch 45 秒；Bash 自带超时（默认 `bash_default_timeout_secs`，模型可覆盖到 600 秒）；Agent 与 AskQuestion 不设超时。
- `Edit` 匹配策略链：exact → quote_normalized → line_number_prefix_stripped → escape_normalized → unicode_escape_normalized → indentation_flexible → line_trimmed → block_anchor，结果里注明命中策略；CRLF 文件保持 CRLF。本地 Write / Edit 会校验文件自上次 Read 后未被修改，否则要求重新 Read；文件不存在时给出同目录相近文件名提示。
- `Read` 支持 png / jpg / gif / webp：图片作为紧随工具结果的用户消息附件交给模型。
- `Bash`：会话开始时导出一次 login shell 快照（函数 / 别名 / shell 选项 / PATH）到 `$APPCONFIG/shell-snapshots/`，之后每次只 `source` 快照再 `eval` 命令；导出失败或关闭 `shell_snapshot_enabled` 时回退 `bash -lc`。`Grep` 在 `rg_sidecar_enabled` 且找到打包的 `tools/rg` 或 PATH 上的 `rg` 时用 ripgrep，否则用 Rust 正则遍历。
- `WebFetch` 有 15 分钟 / 50 MB 的内存缓存。

## 钩子

七类事件：`session_start`（输出注入系统提示尾段）、`user_prompt_submit`（可阻断本次输入或注入上下文）、`pre_tool_use`（可阻断、改写参数、注入上下文）、`post_tool_use`（告警 / 注入上下文）、`post_tool_use_failure`（仅告警）、`permission_request`（可代替用户给出 allow / deny，ask 则继续弹窗）、`stop`（`continue: true` 或 `block` 要求模型继续，一个用户回合最多 3 次）。也接受 Claude Code 的 PascalCase 事件名。

处理器三种：`command`（shell，载荷在 `NATIVE_HOOK_PAYLOAD`，退出码 2 = 阻断）、`http`（POST JSON，2xx 响应体按同一协议解释）、`agent`（用当前会话模型做一次无工具判定）。输出协议 `{ decision: allow|deny|ask|block, reason, updated_input, additional_context, continue }`，兼容 `hookSpecificOutput.permissionDecision / updatedInput / additionalContext` 与 `stopReason`。

来源：设置页的全局钩子（`native-settings.json`）+ 本地工作区的 `.noxcode/hooks.json`（`{ "hooks": [...] }`）与 `.claude/settings.json` / `.claude/settings.local.json` 的 `hooks` 段（`type: prompt` 映射为 `agent`，`matcher` 的 `A|B` 转成工具名列表）。全局先执行，工作区后执行。实现见 [`tools/hooks.rs`](../src-tauri/src/native/tools/hooks.rs) 与 [`hooks_config.rs`](../src-tauri/src/native/hooks_config.rs)。

## 子 Agent 档案与后台任务

- `.md` 档案：`<workspace>/.noxcode/agents/*.md`、`.claude/agents/*.md`、`$APPCONFIG/agents/*.md`。frontmatter：`name`（必填）、`description`、`tools`（逗号或数组；空 / `*` = 全部）、`disallowedTools`、`permissionMode`、`maxTurns`、`skills`（只对子 Agent 开放的技能名）、`injectAgentsMd`；正文即系统提示。与设置页 json 同名时 json 优先；档案 `source = file`，设置页只展示不可编辑。解析见 [`subagents.rs`](../src-tauri/src/native/subagents.rs) `parse_subagent_markdown`。
- 后台任务：`Agent(run_in_background=true)` 立即返回 `task_id`，子 Agent 在独立 tokio 任务里运行（自己的 CancelFlag，父取消会级联）。父 Agent 用 `TaskOutput(task_id, wait, timeout_ms)` 读取 / 等待、`TaskStop` 取消、`SendMessage` 追加指令（进子 Agent 的 steer 通道）；子 Agent 用 `RespondToCoordinator` 留言。完成与留言在父 Agent 下一次模型调用前以 `[后台任务提醒]` 注入。注册表见 [`agent/background.rs`](../src-tauri/src/native/agent/background.rs)；会话结束时停掉全部后台任务。

## 自动化、目标与跨会话上下文

- Cron 自动化（[`scheduler.rs`](../src-tauri/src/native/scheduler.rs)）：五段 cron + `@hourly/@daily/@weekly/@monthly`，本地时区算 `next_run_at`；调度器每 30 秒扫描，工作区有会话在工作中则推迟 1 分钟，到期时用 `start_native_with_manager` 启动新会话（提示词前缀 `[自动化 名称]`）。工具 `CronCreate`（需确认，`kind = automation`）/ `CronList` / `CronDelete`；命令 `list/create/update/delete_native_automations`、`run_native_automation_now`；设置页「自动化」。
- 目标（[`goals.rs`](../src-tauri/src/native/goals.rs)）：`Goal(action=set|update|complete|clear, title, checklist, note)` 维护会话的当前目标与进度清单，`GoalRead` 读取；每次变更写 `[GOAL] {json}` 行，前端渲染为 `GoalRow`。
- `ReadSessionContext`：不带 `session_id` 列出同工作区最近会话（标题、时间、轮数、最后回复摘录）；带 `session_id` 返回该会话最近的用户 / 助手对话摘录。
- `/fork [checkpoint_id]` → `fork_native_session`：把已结束会话的 transcript 复制到一条新的会话记录（标题加「（分叉）」，`resume_session_id` 指向源会话），可选先回滚到某个 Git 检查点；新会话可直接续聊。
- 以上工具通过 `ToolCtx.session_scope`（数据库池、工作区、渠道、模型）访问数据库，只对主 Agent 可见（`ReadSessionContext` 子 Agent 也可用）。

## 记忆（MEMORY.md）

本地工作区且 `memory_enabled` 时，每个工作区一个目录 `$APPCONFIG/memory/<project_key>/`（`project_key` = 目录名 + 8 位哈希）：`MEMORY.md` 是索引（每行 `- [名称](文件.md) — 描述 (type)`，≤ 200 行），事实文件带 frontmatter `name / description / type(user|feedback|project|reference) / created_at / updated_at`。实现见 [`memory.rs`](../src-tauri/src/native/memory.rs)。

- 注入：系统提示的「# 记忆（MEMORY.md）」块（索引 + 维护约定），记忆目录加入 `extra_write_roots`，模型可直接 Read / Write / Edit 记忆文件。
- recall：每个用户回合按关键词（ASCII 词 + CJK 双字，名称 ×3 / 描述 ×2 / 正文 ×1）取前 3 条，以「[记忆回忆]」附在用户消息末尾（不进事件流）。
- extract：会话结束（非取消、至少一问一答）后用轻量模型抽取候选，去重后落盘，事件流写 `[记忆] 已保存 N 条记忆`。
- dream：每 `memory_dream_interval` 次抽取（默认 10，0 = 从不）或设置页「立即整理」时，把全部记忆交给模型合并 / 去重 / 重写。
- 命令：`list_native_memories`、`save_native_memory`、`delete_native_memory`、`open_native_memory_dir`、`dream_native_memory`。
- `/init`：Composer 展开为「摸底仓库并生成 / 补充 AGENTS.md」的提示词，走普通 Agent 回合。

## 模型角色与调用日志

渠道可配置 `lite_model`（必须在该渠道模型列表内）。压缩摘要、记忆抽取 / 整理、`agent` 钩子判定优先用它。`native_api_call_logs` 新增 `operation`（`agent_step` / `compact` / `memory_extract` / `memory_dream` / `hook_agent` / `subagent` / `one_shot`）与 `model_role`（`main` / `lite`）两列，`CallLogContext::with_operation / with_model_role` 写入。

## 上下文压缩

统一入口 `AgentRunner::run_compaction(trigger, instructions)`，顺序：微压缩（把最近 6 条之外、超过 400 字的工具结果替换成一行占位，`microcompact_enabled` 控制）→ 模型摘要（`compaction_prompt_with_instructions`，可带 `/compact` 指令）→ 本地摘要 → 重置。触发方式：

- `auto`：`total_tokens ≥ 窗口 × auto_compact_threshold_percent`（默认 85%，设置页可调 30–99）。
- `manual`：`/compact [指令]`（Composer 拦截）→ `compact_native_session` 命令 → `NativeFollowup::Compact`。等待输入时立刻执行并写回 transcript；工作中则在下一次模型调用前执行。
- `reactive`：模型返回上下文溢出类错误（`is_context_overflow_error`）时被动压缩后重试，一个回合最多 2 次。
- `downshift`：会话恢复时历史已超过当前模型窗口阈值，首轮调用前压缩。

每次压缩写一行 `[COMPACT_BOUNDARY] {trigger, source, pre_tokens, post_tokens, pre_messages, post_messages, instructions}` 到事件流，前端渲染为分隔线（`CompactBoundaryRow`），并刷新 `native-context-usage`。

## 模型层缓存与重试

- Prompt cache：Anthropic 官方端点在 system 末块、最后一个工具、最后一条消息打 `cache_control: ephemeral`；OpenAI / Responses 官方端点传会话级 `prompt_cache_key`。第三方兼容网关默认不改请求体（`PromptCacheMode::Auto`）。系统提示的易变块（日期、Git 状态、权限模式）移到最后，静态前缀才能命中缓存。
- 重试：`model_retry_*` 设置控制指数退避（默认 6 次、1 s 起、上限 30 s、倍数 2、带抖动），服务端 `Retry-After` 优先但不超过上限；流读取中断视为可重试，重试前清空已流式显示的半截内容。

## 工具与 MCP 子进程环境

会话启动时把 `network-settings.json` 转成代理 / CA 环境变量：代理写入大小写 `HTTP_PROXY` / `HTTPS_PROXY`，不代理地址写入 `NO_PROXY`，自定义 CA 写入 `SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS`。本地 Bash 和本地 MCP 子进程会注入这些变量；MCP 自身的 `env` 随后应用。SSH 远端 Bash 与远端 MCP 不注入本机网络设置。

子 Agent 克隆父 Agent 的 `ToolCtx.extra_env`，因此本地 Bash 的网络环境在子 Agent 中保持一致。

## 命令

会话：`start_native_session`、`stop_native_session`、`stop_native`、`restart_native_session`、`resume_native_session`、`send_native_input`、`finish_native_input`、`resolve_native_tool_permission`（决策含 `allow_always`）、`answer_native_plan_question`、`resolve_native_plan_approval`、`compact_native_session`。

工作区 / 历史：`list/create/update/delete_workspace`、`check_workspace_health`、`list_agent_sessions`、`get_agent_session_log_lines`、`prepare_agent_session_resume`、`set_agent_session_pinned`、`delete_agent_session`、`list_activity_logs`。

设置：`get/update_native_settings`、`list_native_global_skills`、`open_native_skills_dir`、`list/create/update/delete_native_subagent`、`get/update/reset_mcp_servers`、`export_mcp_servers_snippet`、`list/get_native_api_call_log`。

删除工作区前会 `clear_workspace_checkpoints`。删除会话前会 `delete_checkpoints_for_session`。运行中的渠道 / 工作区 / 会话拒绝删除。

## 事件

| 事件 | 载荷 |
| --- | --- |
| `native-session` | `AgentSessionStarted` |
| `native-stdout` | `AgentSessionOutput`（已写入 `agent_session_events`）。工具 start/result 带可选 `tool`（`call_id` / `name` / `title` / `ok` / `duration_ms` 等）和 live-only `images`；落库 `message` 为 `{"nox":1,"line":"...","tool":{...}}` 信封，旧纯文本行仍可回放。 |
| `native-text-delta` | `NativeTextDelta`（仅展示，不落库） |
| `native-context-usage` | `NativeContextUsage`（`used` = 工具 schema + 消息；分类字段 + 上次调用 `prompt_tokens` / `cached_tokens`；仅父 Agent；同时写入 `agent_sessions.context_usage_json`） |
| `native-turn-state` | `NativeTurnState`（`waiting_input` / `working`，不落库） |
| `native-permission-request` | 高风险工具确认（含 `suggested_rule`） |
| `native-plan-question` | `AskUserQuestion` 提问（所有模式可用） |
| `native-plan-approval-request` | `ExitPlanMode` 提交的计划，等待批准 / 退回 |
| `native-exit` | `AgentSessionExit` |

前端监听：`onNativeStdout` / `onNativeExit` / `onNativeSession` / `onNativeTextDelta` / `onNativePermissionRequest` / `onNativePlanQuestion` / `onNativeContextUsage` / `onNativeTurnState`。接线见 [`frontend.md`](frontend.md)。

## 设置文件

都在 `$APPCONFIG`：

- `native-settings.json`：轮次、`permission_mode`、权限超时、子 Agent 策略、`global_prompt_template`、`auto_checkpoint_after_tool_call`（默认 `true`）、`checkpoint_retention_days`（默认 7，`0` 不清理）、`desktop_notifications`（默认 `true`）、`artifact_retention_days`（默认 7）、`model_retry_max_retries / model_retry_base_delay_ms / model_retry_max_delay_ms / model_retry_backoff_factor`、`bash_default_timeout_secs`（默认 120）、`shell_snapshot_enabled`、`rg_sidecar_enabled`、`auto_compact_threshold_percent`（默认 85）、`microcompact_enabled`、`memory_enabled`（默认 `true`）、`memory_dream_interval`（默认 10）
- `memory/<project_key>/`：工作区记忆（`MEMORY.md` 索引 + 事实文件 + `.state.json`）
- `artifacts/<session_record_id>/`：超预算工具输出；`shell-snapshots/`：login shell 快照（只保留最近 5 份）
- `native-permissions.json`：全局权限规则；工作区规则在 `<workspace>/.noxcode/permissions.json`
- 工作区钩子：`<workspace>/.noxcode/hooks.json`、`.claude/settings.json`、`.claude/settings.local.json`
- `native-subagents.json`：自定义子智能体（`scope=all|workspaces`）
- `mcp-servers.json`：MCP server 支持 `scope=all|workspaces` 与 `workspace_ids`；会话只连接已启用且匹配当前工作区的服务器
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
- `stop_native_session` 后在同一会话再发送，能静默恢复 transcript 并继续

临时脚本与夹具只放 `/tmp`，不进仓库。会话 UI 与设置入口见 [`frontend.md`](frontend.md)。
