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
3. 读取渠道，允许本次覆盖 model / effort / system_prompt / permission_mode。`AgentSessionStarted.runtime` 返回实际生效的渠道、模型、强度、权限与计划模式；live 续聊拒绝静默忽略配置变化。前端工作中锁定配置，空闲修改时先等待旧 runtime 正常结束，再从同一 transcript 恢复。
4. 建 `ModelClient`（渠道密钥 + 网络设置 + SQLite call log）。
5. 无 `resume_session_id` 时插入 `agent_sessions`（`status=running`），并写出一次启动状态（渠道 banner / 权限说明 / MCP 状态）。有 `resume_session_id` 时：runtime 仍在则把 prompt 放入同一 live 的 `input_queue`，在当前回合完整结束后执行；runtime 已不在则校验工作区后原位重激活（刷新 `started_at` / 渠道 / 执行上下文，清空 `ended_at` / `exit_code`，保留 ID、标题、置顶、`created_at`、累计 token、旧事件和 checkpoint），并静默从同一 ID 的 transcript 恢复。冷启动不把「续聊 / 已恢复」或重复启动状态写进聊天；MCP 连接失败仍写出。发出 `native-session`。
6. 组装系统提示：identity → 子 Agent 策略 → 环境 → Git → 全局模板 → `AGENTS.md` / `CLAUDE.md` → skills。
7. 若工作区是 git 仓：`create_checkpoint(kind=session_start)`，失败只打日志。
8. `auto_checkpoint_after_tool_call=true` 时，`Write` / `Edit` / `ApplyPatch` 成功后异步 `create_checkpoint(kind=after_tool_call)`，同一会话同时只允许一个在途打点；关闭开关不影响会话开始或回滚前检查点。
9. 按当前 `workspace_id` 筛选并连接 `enabled=true` 且 `scope=all` 或命中 `scope=workspaces` / `workspace_ids` 的 MCP server。
10. `run_native_loop` 转发 stdout / delta / context usage / 权限 / 计划提问 / 计划模式变化；退出时写 tokens、status、`native-exit`，并从 manager 移除。主窗口未聚焦且 `desktop_notifications=true` 时，会话结束 / 失败、权限确认和计划问题会发桌面通知。托盘 / 进程退出走 `shutdown_all_sessions`：拒绝待确认，工作中任务 cancel，空闲任务正常 `Finish`，有限等待 join，再关 SSH pool。

`session_kind` 只有 `execution` 与 `plan`，表示启动类型，不能替代当前运行模式。`plan_mode=true` 时本轮结束后保持计划模式，等待输入；不会自动注入实施指令。计划模式由启动参数决定，不写入 `native-settings.json`。`ExitPlanMode` 必须收到当前请求的用户批准才解除限制；拒绝、取消或无审批通道均保持计划模式。计划审批一直等到用户批准、退回或会话取消，不套用高风险确认超时。用户也可在会话空闲后通过模式选择器切换。runner 与 manager 共享计划模式原子状态，运行配置快照从该状态读取；`native-plan-mode` 携带 `input_queue_id` 区分每次运行，前端不允许旧启动快照覆盖同次运行的模式事件。子 Agent 的切换不会广播到父会话。

计划模式的本地与 SSH `Bash` 可用：可验证的只读命令直接执行；写入、高风险及无法确认只读的命令需用户授权，提供「本次允许 / 始终允许 / 当前会话允许所有命令 / 拒绝」。始终允许将完整命令作为字面值保存到当前工作区权限文件，附加 `plan_bash: { target, workspace_root }` 元数据以绑定执行主机和工作目录，保存成功后执行，后续计划会话命中时免确认；通配符仅作为命令内容，不扩大授权范围。可在权限设置中查看、删除，删除后重新询问。旧规则缺少该元数据时不扩权，yolo、build、普通 allow 规则和批准钩子也不跳过确认；显式 deny/ask 仍优先。命令获批后计划模式不变。复用现有权限 IPC 和原子写入流程，保存失败保留请求且不执行。命令按 PreToolUse 改写后的最终参数检查；含脚本、解释器、重定向及未验证包装器的命令保守地要求确认。本地 Bash 可按设置套操作系统沙箱，但计划模式本身不是只读沙箱；数据库查询优先使用 `SQLiteQuery`。`Write / Edit / ApplyPatch` 及写入型 MCP 仍被禁止，explore 子 Agent 不开放 Bash。

「当前会话允许所有命令」使用独立 IPC 决策 `allow_session_commands`，仅为当前运行会话设置 Bash 免确认状态，立即执行当前命令并释放该会话已排队的 Bash 确认。后续本地或 SSH Bash 不再弹窗，包括高风险、写入及 ask 规则；显式 deny 仍直接拒绝。状态由当前会话及其工具上下文共享，不切换 yolo、不退出计划模式，不写权限文件或数据库，不影响其他会话及非 Bash 工具的审批；会话结束、重启或重新启动历史会话后失效。过期、取消或非 Bash 请求不能获取该授权。

权限模式（`permission_mode`）四档，对齐 ZCode：`default` 变更前确认；`edit` 自动放行 `Overwrite`（删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍弹确认）；`build` 再放行不透明 shell 与带 `readOnlyHint` 的 MCP；`yolo` 完全访问（`allow_all_high_risk=true`，只有 ask 规则仍会确认）。旧文件的 `confirm / auto_edit / full` 与 Claude Code 的 `acceptEdits / auto / bypassPermissions / dontAsk` 读入时映射到新名；`confirm_high_risk: false` 读成 `yolo`。`plan` 是会话态：既可由 Composer 选择在启动时进入，也可由模型调用 `EnterPlanMode` 进入；`ExitPlanMode` 提交计划触发 `native-plan-approval-request`，用户批准后恢复执行模式，退回则连同反馈交回模型继续修改。批准 IPC 可带 `ai_channel_id` / `model`：与当前 runtime 不同时先加载新 client 写入 live slot 并 `emit native-session`，再解除 `ExitPlanMode`；同一回合下一次 `chat()` 用实施模型。模型未变或退回则跳过加载。

## 权限规则

规则层在风险分类之前裁决：`deny → allow → ask → 未命中`（对齐 ZCode 的 `denyPriority: beforeAsk`）。每条规则 `{ capability, pattern, source, scope, note, external_path? }`：`capability` 是契约里的能力，`source` 决定匹配字段。普通命令支持前缀通配，路径 / 工具名 / 输入支持 glob。全局规则存 `$APPCONFIG/native-permissions.json`，本地工作区存 `.noxcode/permissions.json`；SSH 工作区存本机 `$APPCONFIG/ssh-workspaces/<工作区 ID 的 UTF-8 十六进制>/.noxcode/permissions.json`。同效果下工作区规则优先。

- 文件访问弹窗提供「本次允许」「始终允许」「拒绝」，展示主机、操作和所有目标路径。「始终允许」默认保存当前文件，目录搜索保存搜索目录及子目录，也可改选文件所在目录；默认当前工作区，可显式选全局。保存失败保留请求，多目标规则原子保存后才执行。
- 本地 / SSH 的 Read、Glob、Grep、Write、Edit、ApplyPatch 均支持工作区外授权。yolo 直接允许外部访问，其他模式遇到未授权路径先确认。补丁源、删除与移动目标统一检查，全部授权后才开始修改；单次授权不进入共享上下文。SSH Glob 使用指定搜索目录，显式指定目录时返回绝对路径。
- `external_path = { target, scope: exact | subtree }` 使用真实绝对路径及路径组件匹配，能力为 `read` 或 `edit`，两者分开。`target` 为 `{ kind: local }` 或 `{ kind: ssh, config_id, host, port, username }`，防止授权跨连接混用。旧规则没有此字段时不自动扩展文件边界。
- 普通工具的「始终允许」沿用 `suggested_rule`，Bash 使用命令前缀、其余工具使用工具名。设置页增删规则后同步运行中会话；文件授权仍保留只读模式、内容指纹、取消和路径验证。
- `ask` 规则命中时即便在 `yolo` 也会弹确认（`kind = rule`）。
- 子 Agent 档案可带 `permission_mode`（不共享父会话的放行开关）与 `disallowed_tools`。
- 命令：`get/update/add/delete_native_permission_rules`；设置页「权限规则」可增删规则。

`send_native_input` / `finish_native_input` 按 `session_record_id` 寻址。`resume_native_session` 若源会话仍在跑，则向同一 live 投递输入；进程不在则原位静默恢复 transcript。同一会话继续发送不是单独的「续聊」产品流程。手动停止写「收到停止请求」，`已取消` 不算失败、不写 `[ERROR] 已取消`。

`finish_native_input` 只正常结束空闲且无排队输入的会话，保留自动记忆抽取机会，并等待资源释放；结束期间拒绝追加输入与配置重启。它用于配置切换、`/fork` 和应用退出时的内部收尾，输入栏不再提供常驻「结束会话」按钮。工作中保留停止按钮，空闲时直接继续发送即可。

应用退出由统一生命周期协调器并发收尾会话，普通退出保留空闲会话的记忆处理机会，总预算 30 秒；更新重启总预算 5 秒，其中会话、MCP 与窗口保存最多 3 秒，并跳过记忆提取。退出开始后拒绝会话登记和追加输入，取消后台任务并跳过桌面通知。超时中止剩余任务，保留已持久化的 transcript，不无限等待资源回收。

运行中通过 `send_native_input` 追加的消息进入后端 FIFO 队列（最多 8 条），不作为即时 steer 注入当前模型上下文。主 Agent 汇总当前工具及子 Agent 结果、完成回答并排空输出事件后，才按顺序逐条执行下一回合。输入框上方显示「待执行指令」，开始执行时才将该条写入聊天记录。`list_native_queued_inputs` / `update_native_queued_input` / `remove_native_queued_input` 支持查看、编辑和移除；编辑中的队首会暂停出队，保存或取消编辑后继续，不能跳过队首执行后续条目。出队与编辑在同一锁内判定，已开始的条目拒绝编辑。队列仅属于当前 runtime，停止或退出时清空，不跨应用重启保存。

`native-session.input_queue_id` 区分同一会话的运行实例；队列 IPC 和 `native-input-queue` 返回带单调 `revision` 的完整快照，前端忽略旧运行实例及过期快照。`/compact` 仍走独立控制通道，可在运行中处理；后台子 Agent 的 `SendMessage` 仍是定向 steer，不受主会话排队语义影响。权限、提问和计划审批按会话与请求 ID 隔离，IPC 成功后才移除请求，失败保留重试；历史计划不附着新请求的审批按钮。后台会话启动不改变当前选中会话。

## 上下文持久化

`agent_session_events` 只服务 UI 回放；模型续聊只读 `native_session_transcripts`。两表没有数据库级同步约束。

顶层 runner 在这些边界同步 UPSERT transcript（fingerprint 未变则跳过）：

- 用户消息进入 `messages` 之后、下一次模型调用之前
- 新回合输入或子 Agent 定向 steer 注入之后（未执行的排队消息不进入 transcript）
- 每一轮 assistant 文本，或 assistant + 对应 tool 结果写完整之后
- `run_native_loop` 退出前再 flush 一次（覆盖错误 / 取消）

保存前会去掉 system、图片，并清洗孤立 tool pair。子 Agent 不写父会话 transcript。硬中断时至少能恢复当前用户任务和已完成的模型 / 工具轮次。

## 工具契约与结果预算

本地会话提供 `SQLiteQuery(file_path, query, parameters?, limit?)`，在计划模式和 explore 子 Agent 中也可使用。该工具通过独立的 Rust SQLite 只读连接查询数据库，复用 `Read` 的路径授权、deny/ask 和白名单；不会使用应用的可写数据库连接。可查询 `sqlite_schema`、执行 SELECT / WITH / EXPLAIN 和表结构 PRAGMA，参数用 `?` 绑定。SQLite 自身解析并限制单条只读语句，禁止写入、ATTACH、加载扩展及 shell 点命令；WAL 模式下可读取应用已提交的最新日志。返回 `{ columns, rows, row_count, truncated }`，默认 200 行、最多 1000 行，结果约 1 MB 上限，查询有超时和取消限制。此工具只面向本地数据库，不增加运行时外部命令依赖。

每个内置工具在 [`tools/catalog.rs`](../src-tauri/src/native/tools/catalog.rs) 声明一份 [`ToolContract`](../src-tauri/src/native/tools/contract.rs)：`read_only / destructive / concurrent_safe / side_effect_scope / risk_level / needs_approval / allowed_in_plan_mode / permission（能力）/ pattern_sources / result_budget / timeout`。MCP 工具按 `tools/list` 返回的 `annotations.readOnlyHint / destructiveHint` 动态生成契约，缺省视为需审批、串行。

- 计划模式与 explore 子 Agent 的只读白名单来自契约的 `allowed_in_plan_mode`，不再硬编码。
- 同一轮里连续的 `concurrent_safe && !destructive && !needs_approval` 调用（Read / Glob / Grep / Lsp / WebFetch / WebSearch / Skill / TodoRead）并行执行，上限 8，结果按模型给出的顺序回填；写工具与 Bash 串行；连续 `Agent` 调用仍成批并行。
- 结果预算：输出超过 `result_budget.max_model_bytes` 且策略为 `Artifact` 时，完整内容写入 `$APPCONFIG/artifacts/<session>/<id>.txt` 并登记 `native_tool_artifacts`，模型只看到头（Glob / Grep / WebFetch / Agent / MCP）或尾（Bash）预览加 artifact 路径；`Read` 允许读取 artifact 目录。之后仍按 `max_tool_output_tokens` 截断兜底。
- 逐工具超时：Read / Write / Edit / Glob / Skill / Todo 30 秒，Grep 60 秒，ApplyPatch 60 秒，WebFetch / WebSearch 45 秒；Bash 自带超时（默认 `bash_default_timeout_secs`，模型可覆盖到 600 秒）；Agent、AskQuestion 与 ExitPlanMode 不设超时。
- `Edit` 匹配策略链：exact → quote_normalized → line_number_prefix_stripped → escape_normalized → unicode_escape_normalized → indentation_flexible → line_trimmed → block_anchor，结果里注明命中策略；CRLF 文件保持 CRLF。本地 Write / Edit 会校验文件自上次 Read 后未被修改，否则要求重新 Read；文件不存在时给出同目录相近文件名提示。
- `Read` 支持 png / jpg / gif / webp：图片作为紧随工具结果的用户消息附件交给模型。
- `Bash`：会话开始时导出一次 login shell 快照（函数 / 别名 / shell 选项 / PATH）到 `$APPCONFIG/shell-snapshots/`，之后每次只 `source` 快照再 `eval` 命令；导出失败或关闭 `shell_snapshot_enabled` 时回退 `bash -lc`。`run_in_background=true` 把命令登记到会话进程表，立即返回 `process_id`；用 `ProcessList` / `ProcessOutput` / `ProcessStop` / `Monitor` 跟踪。仅本地会话支持后台 Bash。`Grep` 在 `rg_sidecar_enabled` 且找到打包的 `tools/rg` 或 PATH 上的 `rg` 时用 ripgrep，否则用 Rust 正则遍历。
- `Lsp`：本地会话按文件扩展名懒启动 `rust-analyzer` / `typescript-language-server` / `pyright-langserver` / `gopls` / `clangd`。支持 goToDefinition、findReferences、hover、documentSymbol、workspaceSymbol、goToImplementation、diagnostics。`Write` / `Edit` / `ApplyPatch` 成功后把诊断附到工具结果。设置项 `lsp_enabled` 默认开；SSH 工作区不启动 language server。
- 本地 Bash 可开启操作系统沙箱（`bash_sandbox_enabled`，默认关）：Linux 用 `bwrap` 只读根 + 可写工作区，macOS 用 `sandbox-exec`。找不到实现时回退并在结果里注明。SSH 不套沙箱。
- 首页 Composer 用「当前工作区 | 隔离工作树」两段切换（启动参数 `isolate_worktree`）：新会话创建 git worktree 并检出唯一分支 `noxcode/wt-<session>`（不再 detached）。本地默认 `~/.noxcode/worktrees/<session_id>`，也可在设置里改根目录；SSH 为 `$HOME/.noxcode/worktrees/<session_id>`。默认不隔离。非 git 仓库跳过并提示。开启「创建前拉取」时会先 `git fetch --all --prune`（失败仍创建）。开启自动清理时，现存托管工作树超过上限会删掉最旧且未在使用的（删前打 checkpoint）。会话内也可用 `EnterWorktree` / `ExitWorktree`。工具后 checkpoint 打在当前活动目录。删除会话或在设置页手动删除时会移除托管 worktree。任务写完后可在会话标题栏、本轮结束或进程退出时选择：合并回当前分支、建成新分支、或先保留；合并前可手写或用侧边栏同款能力生成提交说明。Git 侧栏看当前会话的隔离 worktree（若有）。合并冲突停在中间态，可选 AI 自动解决或打开 Git 面板手动处理，中止才 `merge --abort`。
- 未验证 Shell 命令默认需要授权；重定向覆盖、`cp/mv`、所有 `git restore` 均进入风险判断。本地 Bash 同时排空两路输出并限内存，超时或取消时终止独立进程组；SSH Bash 透传 deadline / cancel 并发送终止信号、关闭通道。超出硬上限的输出仅保留尾部，不能从 artifact 恢复被丢弃前缀。
- 本地文件工具按真实路径及最近存在父目录检查边界，额外读写根保持各自权限，递归搜索不跟随符号链接；SSH 文件工具拒绝符号链接路径。SSH Write 支持防覆盖创建新文件，覆盖旧文件仍要求 Read 与内容指纹匹配。这些边界不等同于操作系统级 Shell 沙箱。
- 本地会话的 `Read / Glob / Grep` 默认允许只读访问当前有效技能目录及其附属文件，遵守启停、重名覆盖和子 Agent 的技能筛选。技能父目录及写入需额外授权，yolo 按完全访问处理。相对路径以工作区为基准，未指定路径的搜索只扫描工作区。链接按真实目录检查，递归搜索不跟随链接逃逸；直接访问外部链接目标仍须经过外部路径授权。
- `WebFetch` 有 15 分钟 / 50 MB 的内存缓存。

## 钩子

七类事件：`session_start`（输出注入系统提示尾段）、`user_prompt_submit`（可阻断本次输入或注入上下文）、`pre_tool_use`（可阻断、改写参数、注入上下文）、`post_tool_use`（告警 / 注入上下文）、`post_tool_use_failure`（仅告警）、`permission_request`（可代替用户给出 allow / deny，ask 则继续弹窗）、`stop`（`continue: true` 或 `block` 要求模型继续，一个用户回合最多 3 次）。也接受 Claude Code 的 PascalCase 事件名。

处理器三种：`command`（shell，载荷在 `NATIVE_HOOK_PAYLOAD`，退出码 2 = 阻断）、`http`（POST JSON，2xx 响应体按同一协议解释）、`agent`（用当前会话模型做一次无工具判定）。输出协议 `{ decision: allow|deny|ask|block, reason, updated_input, additional_context, continue }`，兼容 `hookSpecificOutput.permissionDecision / updatedInput / additionalContext` 与 `stopReason`。

来源：设置页的全局钩子（`native-settings.json`）+ 本地工作区的 `.noxcode/hooks.json`（`{ "hooks": [...] }`）与 `.claude/settings.json` / `.claude/settings.local.json` 的 `hooks` 段（`type: prompt` 映射为 `agent`，`matcher` 的 `A|B` 转成工具名列表）。全局先执行，工作区后执行。实现见 [`tools/hooks.rs`](../src-tauri/src/native/tools/hooks.rs) 与 [`hooks_config.rs`](../src-tauri/src/native/hooks_config.rs)。

工作区及工作区插件贡献的钩子必须先通过 `WorkspaceHooks` 显式批准本次会话，拒绝、超时或取消均不执行。该信任请求不受 yolo、权限规则或其他自动批准钩子绕过，也不会连带放行其他工具。

## 子 Agent 档案与后台任务

- `.md` 档案：`<workspace>/.noxcode/agents/*.md`、`.claude/agents/*.md`、`$APPCONFIG/agents/*.md`。frontmatter：`name`（必填）、`description`、`tools`（逗号或数组；空 / `*` = 全部）、`disallowedTools`、`permissionMode`、`maxTurns`、`skills`（只对子 Agent 开放的技能名）、`injectAgentsMd`；正文即系统提示。与设置页 json 同名时 json 优先；档案 `source = file`，设置页只展示不可编辑。解析见 [`subagents.rs`](../src-tauri/src/native/subagents.rs) `parse_subagent_markdown`。
- 后台任务：`Agent(run_in_background=true)` 立即返回 `task_id`，子 Agent 在独立 tokio 任务里运行（自己的 CancelFlag，父取消会级联）。父 Agent 用 `TaskOutput(task_id, wait, timeout_ms)` 读取 / 等待、`TaskStop` 取消、`SendMessage` 追加指令（进子 Agent 的 steer 通道）；子 Agent 用 `RespondToCoordinator` 留言。完成与留言在父 Agent 下一次模型调用前以 `[后台任务提醒]` 注入。注册表见 [`agent/background.rs`](../src-tauri/src/native/agent/background.rs)；会话结束时停掉全部后台任务。
- Agent 特殊调度与普通工具共用权限、只读检查及 Hook 前后置入口；前台和后台跨批共享同一并发许可。后台状态包含 queued / running / done / failed / stopped，消息在模型调用边界和最终返回前消费，队列满或任务已关闭立即报错。前端可查看任务、发送消息与停止任务，使用 `list_native_background_tasks`、`send_native_background_message`、`stop_native_background_task`。

## 自动化、目标与跨会话上下文

- Cron 自动化（[`scheduler.rs`](../src-tauri/src/native/scheduler.rs)）：五段 cron + `@hourly/@daily/@weekly/@monthly`，本地时区算 `next_run_at`；调度器每 30 秒扫描，工作区有会话在工作中则推迟 1 分钟，到期时用 `start_native_with_manager` 启动新会话（提示词前缀 `[自动化 名称]`）。工具 `CronCreate`（需确认，`kind = automation`）/ `CronList` / `CronDelete`；命令 `list/create/update/delete_native_automations`、`run_native_automation_now`；设置页「自动化」。
- 目标（[`goals.rs`](../src-tauri/src/native/goals.rs)）：`Goal(action=set|update|complete|clear, title, checklist, note)` 维护会话的当前目标与进度清单，`GoalRead` 读取；每次变更写 `[GOAL] {json}` 行，前端渲染为 `GoalRow`。
- `ReadSessionContext`：不带 `session_id` 列出同工作区最近会话（标题、时间、轮数、最后回复摘录）；带 `session_id` 仍校验工作区归属，再返回最近的用户 / 助手对话摘录。
- `/fork [checkpoint_id]` → `fork_native_session`：把已结束会话的 transcript 复制到一条新的会话记录（标题加「（分叉）」，`resume_session_id` 指向源会话），可选先回滚到某个 Git 检查点；新会话可直接续聊。
- Composer 斜杠：自定义命令来自工作区 `.noxcode/commands`、`.claude/commands`、`.zcode/commands`、已启用插件 `commands/` 与 `$APPCONFIG/native-commands/`。内置 `/mode` `/model` `/effort` `/plan` `/new` `/clear` `/help` `/diff` `/context` `/permissions` `/memory` `/mcp` `/plugins` 由前端执行；`/init` `/goal` `/review` `/create-skill` `/create-subagent` 展开成提示词后走普通 Agent 回合。`/create-skill name` 写入 `.noxcode/skills/<name>/SKILL.md`；`/create-subagent name` 写入 `.noxcode/agents/<name>.md`。
- 以上工具通过 `ToolCtx.session_scope`（数据库池、工作区、渠道、模型）访问数据库，只对主 Agent 可见（`ReadSessionContext` 子 Agent 也可用）。

## 记忆（MEMORY.md）

本地工作区且 `memory_enabled` 时，每个工作区一个目录 `$APPCONFIG/memory/<project_key>/`（`project_key` = 目录名 + 8 位哈希）：`MEMORY.md` 是索引（每行 `- [名称](文件.md) — 描述 (type)`，≤ 200 行），事实文件带 frontmatter `name / description / type(user|feedback|project|reference) / created_at / updated_at`。实现见 [`memory.rs`](../src-tauri/src/native/memory.rs)。

- 注入：系统提示的「# 记忆（MEMORY.md）」块（索引 + 维护约定），记忆目录加入 `extra_write_roots`，模型可直接 Read / Write / Edit 记忆文件。
- recall：每个用户回合按关键词（ASCII 词 + CJK 双字，名称 ×3 / 描述 ×2 / 正文 ×1）取前 3 条，以「[记忆回忆]」附在用户消息末尾（不进事件流）。
- extract：会话正常结束（非取消、至少一问一答）后用轻量模型抽取候选，去重后落盘，事件流写 `[记忆] 已保存 N 条记忆`。结束时抽取和 dream 合计最多等待 20 秒，不无限阻塞退出。
- dream：每 `memory_dream_interval` 次抽取（默认 10，0 = 从不）或设置页「立即整理」时，把全部记忆交给模型合并 / 去重 / 重写。
- 命令：`list_native_memories`、`save_native_memory`、`delete_native_memory`、`open_native_memory_dir`、`dream_native_memory`。
- `/init`：Composer 展开为「摸底仓库并生成 / 补充 AGENTS.md」的提示词，走普通 Agent 回合。

## 模型角色与调用日志

渠道可配置 `lite_model`（必须在该渠道模型列表内）。压缩摘要、记忆抽取 / 整理、`agent` 钩子判定优先用它。`native_api_call_logs` 新增 `operation`（`agent_step` / `compact` / `memory_extract` / `memory_dream` / `hook_agent` / `subagent` / `one_shot` / `commit_message` / `session_title`）与 `model_role`（`main` / `lite`）两列，`CallLogContext::with_operation / with_model_role` 写入。Codex 渠道的 `responses_continuation` 控制是否发送 `previous_response_id`，见 [`channels.md`](channels.md)。

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

会话启动时把 `network-settings.json` 转成代理 / CA 环境变量：代理写入大小写 `HTTP_PROXY` / `HTTPS_PROXY`，不代理地址写入 `NO_PROXY`，自定义 CA 写入 `SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS`。本地 Bash 和本地 MCP 子进程会注入这些变量；MCP 自身的 `env` 随后应用。本地 stdio MCP 还会把 Homebrew / nvm / fnm / volta 等常见目录并入 PATH，并把 `npx` 这类裸命令解析成绝对路径后再 spawn，避免桌面进程 PATH 过短导致 `os error 2`。SSH 远端 Bash 与远端 MCP 不注入本机网络设置。

子 Agent 克隆父 Agent 的 `ToolCtx.extra_env`，因此本地 Bash 的网络环境在子 Agent 中保持一致。

## 命令

会话：`start_native_session`、`stop_native_session`、`stop_native`、`restart_native_session`、`resume_native_session`、`send_native_input`、`finish_native_input`、`resolve_native_tool_permission`（决策含 `allow_always`）、`answer_native_plan_question`、`resolve_native_plan_approval`（可选 `ai_channel_id` / `model` / `reasoning_effort`，批准时热更换实施模型与思考等级）、`compact_native_session`。

工作区 / 历史：`list/create/update/delete_workspace`、`check_workspace_health`、`list_agent_sessions`、`get_agent_session_log_lines`、`prepare_agent_session_resume`、`set_agent_session_pinned`、`delete_agent_session`、`list_activity_logs`。

设置：`get/update_native_settings`、`list_native_global_skills`、`list_native_skills`、`open_native_skills_dir`、`create/delete/copy/import/scan/set_enabled` 技能命令、`list/create/update/delete_native_subagent`、`get/update/reset_mcp_servers`、`export_mcp_servers_snippet`、`list/get_native_api_call_log`。

删除工作区前会 `clear_workspace_checkpoints`。删除会话前会 `delete_checkpoints_for_session`。运行中的渠道 / 工作区 / 会话拒绝删除。

## 事件

| 事件 | 载荷 |
| --- | --- |
| `native-session` | `AgentSessionStarted` |
| `native-input-queue` | `session_record_id` + `queue_id` + `revision` + `items(id/text/image_count/editing)`，待执行指令完整快照。 |
| `native-request-resolved` | `session_record_id` + `request_id` + `kind(permission/question/plan_approval)`，仅清除对应请求。 |
| `native-background-tasks` | `session_record_id` + `tasks`，后台任务完整快照。 |
| `native-stdout` | `AgentSessionOutput`（已写入 `agent_session_events`）。工具 start/result 带可选 `tool`（`call_id` / `name` / `title` / `ok` / `duration_ms` 等）和 live-only `images`；落库 `message` 为 `{"nox":1,"line":"...","tool":{...}}` 信封，旧纯文本行仍可回放。 |
| `native-text-delta` | `NativeTextDelta`（仅展示，不落库） |
| `native-context-usage` | `NativeContextUsage`（`used` = 工具 schema + 消息；分类字段 + 上次调用 `prompt_tokens` / `cached_tokens`；仅父 Agent；同时写入 `agent_sessions.context_usage_json`） |
| `native-turn-state` | `NativeTurnState`（`waiting_input` / `working`，不落库） |
| `native-plan-mode` | `NativePlanModeChanged`（`session_record_id` + 当前 `plan_mode`，不落库） |
| `native-permission-request` | 高风险工具确认（含 `suggested_rule`） |
| `native-plan-question` | `AskUserQuestion` 提问（所有模式可用） |
| `native-plan-approval-request` | `ExitPlanMode` 提交的计划，等待批准 / 退回 |
| `native-exit` | `AgentSessionExit` |

前端监听：`onNativeStdout` / `onNativeExit` / `onNativeSession` / `onNativeTextDelta` / `onNativePermissionRequest` / `onNativePlanQuestion` / `onNativeContextUsage` / `onNativeTurnState` / `onNativePlanMode`。接线见 [`frontend.md`](frontend.md)。

## 设置文件

都在 `$APPCONFIG`：

- `native-settings.json`：轮次、`permission_mode`、权限超时、子 Agent 策略、`global_prompt_template`、`auto_checkpoint_after_tool_call`（默认 `true`）、`checkpoint_retention_days`（默认 7，`0` 不清理）、`desktop_notifications`（默认 `true`）、`artifact_retention_days`（默认 7）、`model_retry_max_retries / model_retry_base_delay_ms / model_retry_max_delay_ms / model_retry_backoff_factor`、`bash_default_timeout_secs`（默认 120）、`shell_snapshot_enabled`、`rg_sidecar_enabled`、`auto_compact_threshold_percent`（默认 85）、`microcompact_enabled`、`memory_enabled`（默认 `true`）、`memory_dream_interval`（默认 10）、`worktree_root`（空=默认 `~/.noxcode/worktrees`）、`worktree_fetch_before_create`（默认关）、`worktree_auto_prune`（默认开）、`worktree_auto_prune_limit`（默认 15，范围 1–200）
- `memory/<project_key>/`：工作区记忆（`MEMORY.md` 索引 + 事实文件 + `.state.json`）
- `artifacts/<session_record_id>/`：超预算工具输出；`shell-snapshots/`：login shell 快照（只保留最近 5 份）
- `native-permissions.json`：全局权限规则；工作区规则在 `<workspace>/.noxcode/permissions.json`
- 工作区钩子：`<workspace>/.noxcode/hooks.json`、`.claude/settings.json`、`.claude/settings.local.json`
- `native-subagents.json`：自定义子智能体（`scope=all|workspaces`）
- `mcp-servers.json`：MCP server 支持 `scope=all|workspaces` 与 `workspace_ids`；会话只连接已启用且匹配当前工作区的服务器
- `native-skills-state.json`：技能启停名单（按 `SKILL.md` 路径）
- 用户级技能：`~/.noxcode/skills`（旧 `$APPCONFIG/native-skills` 仅只读兜底）；工作区另读 `.noxcode/skills`、`.zcode/skills`、`.agents/skills`、`.claude/skills`（cwd 与 git 根）

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
