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
8. `auto_checkpoint_after_tool_call=true` 时，`Write` / `Edit` / `ApplyPatch` 成功后异步 `create_checkpoint(kind=after_tool_call)`，同一会话同时只允许一个在途打点；同一开关打开时，这些受控写入还会同步记下路径级 before/after，供消息回滚使用。关闭开关后不再记路径快照，预览会说明文件回滚不可用。整库打点的在途跳过不影响路径记录。关闭开关不影响会话开始或回滚前检查点。
9. 按当前 `workspace_id` 筛选并连接 `enabled=true` 且 `scope=all` 或命中 `scope=workspaces` / `workspace_ids` 的 MCP server。
10. `run_native_loop` 转发 stdout / delta / context usage / 权限 / 计划提问 / 计划模式变化；退出时写 tokens、status、`native-exit`，并从 manager 移除。主窗口未聚焦且 `desktop_notifications=true` 时，会话结束 / 失败、权限确认和计划问题会发桌面通知。托盘 / 进程退出走 `shutdown_all_sessions`：拒绝待确认，工作中任务 cancel，空闲任务正常 `Finish`，有限等待 join，再关 SSH pool。

`session_kind` 只有 `execution` 与 `plan`，表示启动类型，不能替代当前运行模式。`plan_mode=true` 时本轮结束后保持计划模式，等待输入；不会自动注入实施指令。计划模式由启动参数决定，不写入 `native-settings.json`。`ExitPlanMode` 必须收到当前请求的用户批准才解除限制；拒绝、取消或无审批通道均保持计划模式。计划审批一直等到用户批准、退回或会话取消，不套用高风险确认超时。用户也可在会话空闲后通过模式选择器切换。runner 与 manager 共享计划模式原子状态，运行配置快照从该状态读取；`native-plan-mode` 携带 `input_queue_id` 区分每次运行，前端不允许旧启动快照覆盖同次运行的模式事件。计划模式顶层 runner 仍在模型请求中提供 `Agent` 工具及完整目录摘要（内置 general / explore 与自定义 Agent 的名称、描述、工具摘要），但执行层只允许显式启动内置只读 `explore`；省略类型所默认的 general、显式 general 及所有自定义类型均拒绝。普通只读 runner 与子 Agent 仍不提供 `Agent`。子 Agent 的切换不会广播到父会话。

计划模式的本地与 SSH `Bash` 可用：可验证的只读命令直接执行；写入、高风险及无法确认只读的命令需用户授权，提供「本次允许 / 始终允许 / 当前会话允许所有命令 / 拒绝」。始终允许将完整命令作为字面值保存到当前工作区权限文件，附加 `plan_bash: { target, workspace_root }` 元数据以绑定执行主机和工作目录，保存成功后执行，后续计划会话命中时免确认；通配符仅作为命令内容，不扩大授权范围。可在权限设置中查看、删除，删除后重新询问。旧规则缺少该元数据时不扩权，build、普通 allow 规则和批准钩子也不跳过确认；`yolo` 完全访问会跳过计划模式 Bash 确认。显式 deny 仍优先；ask 仅在非 yolo 时确认。命令获批后计划模式不变。复用现有权限 IPC 和原子写入流程，保存失败保留请求且不执行。命令按 PreToolUse 改写后的最终参数检查；含脚本、解释器、重定向及未验证包装器的命令保守地要求确认。本地 Bash 可按设置套操作系统沙箱，但计划模式本身不是只读沙箱；数据库查询优先使用 `SQLiteQuery`。`Write / Edit / ApplyPatch` 及写入型 MCP 仍被禁止。explore 子 Agent 与计划模式共用只读 Bash：已审计的只读命令直接执行；写入与不透明命令在非 yolo 时需确认。`yolo` 完全访问跳过这些 Bash 确认；`Write` / `Edit` / `ApplyPatch` 仍被只读规划禁止。

「当前会话允许所有命令」使用独立 IPC 决策 `allow_session_commands`，仅为当前运行会话设置 Bash 免确认状态，立即执行当前命令并释放该会话已排队的 Bash 确认。后续本地或 SSH Bash 不再弹窗，包括高风险、写入及 ask 规则；显式 deny 仍直接拒绝。状态由当前会话及其工具上下文共享，不切换 yolo、不退出计划模式，不写权限文件或数据库，不影响其他会话及非 Bash 工具的审批；会话结束、重启或重新启动历史会话后失效。过期、取消或非 Bash 请求不能获取该授权。

权限模式（`permission_mode`）四档，对齐 ZCode：`default` 变更前确认；`edit` 自动放行 `Overwrite`（删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍弹确认）；`build` 再放行不透明 shell 与带 `readOnlyHint` 的 MCP；`yolo` 完全访问（`allow_all_high_risk=true`，不弹 MCP / 工作区钩子 / 命令 / ask 规则确认，deny 仍拒绝）。旧文件的 `confirm / auto_edit / full` 与 Claude Code 的 `acceptEdits / auto / bypassPermissions / dontAsk` 读入时映射到新名；`confirm_high_risk: false` 读成 `yolo`。`plan` 是会话态：既可由 Composer 选择在启动时进入，也可由模型调用 `EnterPlanMode` 进入；`ExitPlanMode` 提交计划触发 `native-plan-approval-request`，用户批准后恢复执行模式，退回则连同反馈交回模型继续修改。批准 IPC 可带 `ai_channel_id` / `model`：与当前 runtime 不同时先加载新 client 写入 live slot 并 `emit native-session`，再解除 `ExitPlanMode`；同一回合下一次 `chat()` 用实施模型。模型未变或退回则跳过加载。

待批准计划写入 `agent_sessions.pending_plan_json`（`{request_id, plan, created_at}`）；停止或退出后仍保留。live 与 detached 审批统一走 `resolve_native_plan_approval`：后端读取对应请求的正文并校验实施模型，先保存 `approved_plan_json` 授权快照，再将计划原子写入实际会话目录的 `.noxcode/plans/plan-<session>.md`（支持隔离 worktree 和 SSH），成功后才应用实施模型并解除只读。普通执行模式续聊不能跳过尚未解决的计划审批；停止后的批准由后端启动实施续聊，前端不再拼接批准提示词绕过保存。

已批准快照包含正文、补充意见、工作目录、路径、哈希、保存状态及实施模型。保存失败保留审批和快照，卡片可「重试保存并实施」；正文、补充意见或模型选择变化后需要新的批准。写入使用同目录临时文件和原子替换；已有文件与上次成功保存的哈希不一致时报冲突，不覆盖用户改动。停止/新计划会先使旧的在途授权失效，解除只读与实施确认在同一授权锁下提交。成功记录在普通续聊中保留，工具结果、计划卡片和后续回合上下文提供计划路径。实现见 [`plans.rs`](../src-tauri/src/native/plans.rs)。

## 权限规则

规则层在风险分类之前裁决：`deny → allow → ask → 未命中`（对齐 ZCode 的 `denyPriority: beforeAsk`）。每条规则 `{ capability, pattern, source, scope, note, external_path? }`：`capability` 是契约里的能力，`source` 决定匹配字段。普通命令支持前缀通配，路径 / 工具名 / 输入支持 glob。全局规则存 `$APPCONFIG/native-permissions.json`，本地工作区存 `.noxcode/permissions.json`；SSH 工作区存本机 `$APPCONFIG/ssh-workspaces/<工作区 ID 的 UTF-8 十六进制>/.noxcode/permissions.json`。同效果下工作区规则优先。

- 文件访问弹窗提供「本次允许」「始终允许」「拒绝」，展示主机、操作和所有目标路径。「始终允许」默认保存当前文件，目录搜索保存搜索目录及子目录，也可改选文件所在目录；默认当前工作区，可显式选全局。保存失败保留请求，多目标规则原子保存后才执行。
- 本地 / SSH 的 Read、Glob、Grep、Write、Edit、ApplyPatch 均支持工作区外授权。yolo 直接允许外部访问，其他模式遇到未授权路径先确认。补丁源、删除与移动目标统一检查，全部授权后才开始修改；单次授权不进入共享上下文。SSH Glob 使用指定搜索目录，显式指定目录时返回绝对路径。
- `external_path = { target, scope: exact | subtree }` 使用真实绝对路径及路径组件匹配，能力为 `read` 或 `edit`，两者分开。`target` 为 `{ kind: local }` 或 `{ kind: ssh, config_id, host, port, username }`，防止授权跨连接混用。旧规则没有此字段时不自动扩展文件边界。
- 普通工具的「始终允许」沿用 `suggested_rule`，Bash 使用命令前缀、其余工具使用工具名。设置页增删规则后同步运行中会话；文件授权仍保留只读模式、内容指纹、取消和路径验证。
- `ask` 规则命中时在非 `yolo` 模式弹确认（`kind = rule`）；`yolo` 跳过该确认。
- 子 Agent 档案可带 `permission_mode`（不共享父会话的放行开关）与 `disallowed_tools`。
- 命令：`get/update/add/delete_native_permission_rules`；设置页「权限规则」可增删规则。

`send_native_input` / `finish_native_input` 按 `session_record_id` 寻址。`resume_native_session` 若源会话仍在跑，则向同一 live 投递输入；进程不在则原位静默恢复 transcript。同一会话继续发送不是单独的「续聊」产品流程。手动停止写「收到停止请求」，`已取消` 不算失败、不写 `[ERROR] 已取消`。

`finish_native_input` 只正常结束空闲且无排队输入的会话，保留自动记忆抽取机会，并等待资源释放；结束期间拒绝追加输入与配置重启。它用于配置切换、`/fork` 和应用退出时的内部收尾，输入栏不再提供常驻「结束会话」按钮。工作中保留停止按钮，空闲时直接继续发送即可。

应用退出由统一生命周期协调器并发收尾会话，普通退出保留空闲会话的记忆处理机会，总预算 30 秒；更新重启总预算 5 秒，其中会话、MCP 与窗口保存最多 3 秒，并跳过记忆提取。退出开始后拒绝会话登记和追加输入，取消后台任务并跳过桌面通知。超时中止剩余任务，保留已持久化的 transcript，不无限等待资源回收。

运行中通过 `send_native_input` 追加的消息进入后端 FIFO 队列（最多 8 条），不作为即时 steer 注入当前模型上下文。主 Agent 汇总当前工具及子 Agent 结果、完成回答并排空输出事件后，才按顺序逐条执行下一回合。输入框上方显示「待执行指令」，开始执行时才将该条写入聊天记录。`list_native_queued_inputs` / `update_native_queued_input` / `remove_native_queued_input` 支持查看、编辑和移除；编辑中的队首会暂停出队，保存或取消编辑后继续，不能跳过队首执行后续条目。出队与编辑在同一锁内判定，已开始的条目拒绝编辑。队列仅属于当前 runtime，停止或退出时清空，不跨应用重启保存。

`native-session.input_queue_id` 区分同一会话的运行实例；队列 IPC 和 `native-input-queue` 返回带单调 `revision` 的完整快照，前端忽略旧运行实例及过期快照。`/compact` 仍走独立控制通道，可在运行中处理；后台子 Agent 的 `SendMessage` 仍是定向 steer，不受主会话排队语义影响。权限、提问和计划审批按会话与请求 ID 隔离，IPC 成功后才移除请求，失败保留重试；历史计划不附着新请求的审批按钮。后台会话启动不改变当前选中会话。

工作中可另选「转向当前回合」，通过 `submit_native_steer` 提交 `session_record_id`、`expected_turn_id`、客户端 UUID `input_id`、文字和已暂存图片路径。后端为每个用户回合生成 `turn_id`，接收与最终关闭共用同一信箱锁；同一 UUID 的重试先查持久化回执，不重复消费或重新读取已清理的附件。接收上限是 8 条待消费输入、每条 200 KiB UTF-8 文字，图片沿用每条 8 张/每张 8 MiB，待消费图片合计最多 64 MiB。只有校验及持久化接收成功才清理暂存图片；失败不改投下一回合。

转向在模型响应返回后、串行工具之间、已启动并行批次完成后及最终关闭前消费。已开始的工具等到安全边界；旧响应尚未开始的工具写入「因转向未执行」结果，维持调用/结果配对。接收会使主回合旧的权限、提问、计划审批失效，后台子 Agent 的独立交互保留；计划模式保持，旧审批不能再解除限制。输入仍经过用户输入 Hook，拒绝会明确记录；转向不重置资源预算或输出续写次数。

`native_steer` 回执复用 `agent_session_events`，记录 `accepted / applied / rejected / cancelled`，普通聊天日志分页不包含这些状态事件。`get_native_steer_snapshot` 和 `native-steer` 提供运行实例、当前回合、单调版本、回执与最近生命周期状态；快照可恢复漏收的回合广播。回合关闭后保留已完成的回合身份供空闲压缩状态使用，每次状态变化仍递增版本。进程重启不自动重放尚未消费的转向；界面显示中断并支持恢复文字，图片需重新选择。后端内存仅保留待消费项与最近 128 条终态回执，UUID 去重查持久化记录，不限制单回合累计转向次数；前端展示最近 256 条状态。实现见 [`steer.rs`](../src-tauri/src/native/steer.rs)。

## 上下文持久化

`agent_session_events` 只服务 UI 回放。模型续聊的权威记录是追加式历史：`native_history_messages` 保留原文和稳定消息身份，`native_context_anchors` 是可重建的上下文投影。`native_session_transcripts.messages_json` 只缓存当前投影，供兼容读取。压缩替换投影中的工具结果或折叠旧回合，不删除历史，也不改消息身份。

顶层 runner 在这些边界把历史、分支修订号和投影放进同一事务（fingerprint 未变则跳过）：

- 用户消息进入 `messages` 之后、下一次模型调用之前
- 新回合输入、用户转向或子 Agent 定向 steer 注入之后（未执行的排队消息不进入 transcript）
- 每一轮 assistant 文本，或 assistant + 对应 tool 结果写完整之后
- `run_native_loop` 退出前再 flush 一次（覆盖错误 / 取消）

事务用 `BEGIN IMMEDIATE` 一开始就拿写锁。遇到 `database is locked`（含 SQLite 517 `BUSY_SNAPSHOT`）会回滚并重试最多 5 次，短暂并发写入不会把回合判失败。其他保存错误仍向上返回并停止该回合，而且不会发出 `native-history-committed`。提交成功后才发出该事件（含 `branch_id`、`revision`、`message_ids`）。投影会去掉 system、图片字节，并排除尚未配对的工具调用；原始历史仍保留该工具调用，图片标为不可恢复。子 Agent 不写父会话 transcript。

`list_native_history_boundaries` 返回活动分支、修订号、能力缺口和每条消息前后是否可选。会拆开工具调用与结果的位置 `selectable_after` 为假。旧会话只在 transcript 行还在时建立兼容基线；压缩前丢失的原文、图片，以及没有消息归属的旧检查点记为 `unrecoverable`，不推测关联。`/fork` 新会话引用源分支的历史，不复制消息行；删除源会话不会级联删除这些行。仍被活动分支引用的历史不参与过期清理。

硬中断时至少能恢复当前用户任务和已完成的模型 / 工具轮次。尚未提交的半截模型响应不进入投影。

模型响应通过完整性检查后，先把整批工具调用写入 `native_tool_runs`。每个工具开始前把状态改成 `started`，执行完立即提交结果。并行结果可以各自提交，交给模型时仍按原来的调用顺序。恢复时复用已提交结果；还没开始的调用，以及已开始但没有结果的无副作用调用，会重新检查权限再执行。已开始但结果未提交的副作用标成「结果未知：操作已开始但结果未提交，不会自动重放」。未确认的半截工具调用不会进入上下文，也不会交给执行器。Responses 的 `previous_response_id` 不能代替缺失的本地提交锚点。外部副作用不保证恰好执行一次。旧运行实例的事件和审批会被拒绝。实现见 [`recovery.rs`](../src-tauri/src/native/recovery.rs)。

## 工具契约与结果预算

本地会话提供 `SQLiteQuery(file_path, query, parameters?, limit?)`，在计划模式和 explore 子 Agent 中也可使用。该工具通过独立的 Rust SQLite 只读连接查询数据库，复用 `Read` 的路径授权、deny/ask 和白名单；不会使用应用的可写数据库连接。可查询 `sqlite_schema`、执行 SELECT / WITH / EXPLAIN 和表结构 PRAGMA，参数用 `?` 绑定。SQLite 自身解析并限制单条只读语句，禁止写入、ATTACH、加载扩展及 shell 点命令；WAL 模式下可读取应用已提交的最新日志。返回 `{ columns, rows, row_count, truncated }`，默认 200 行、最多 1000 行，结果约 1 MB 上限，查询有超时和取消限制。此工具只面向本地数据库，不增加运行时外部命令依赖。

每个内置工具在 [`tools/catalog.rs`](../src-tauri/src/native/tools/catalog.rs) 声明一份 [`ToolContract`](../src-tauri/src/native/tools/contract.rs)：`read_only / destructive / concurrent_safe / side_effect_scope / risk_level / needs_approval / allowed_in_plan_mode / permission（能力）/ pattern_sources / result_budget / timeout`。MCP 工具按 `tools/list` 返回的 `annotations.readOnlyHint / destructiveHint` 动态生成契约，缺省视为需审批、串行。

- 计划模式与 explore 子 Agent 的常规只读白名单来自契约的 `allowed_in_plan_mode`，不再硬编码。`Agent` 保持 `allowed_in_plan_mode=false`：仅计划模式顶层 runner 在工具广告和预检中作显式特例，再由 `run_agent_batch` 按 `SubagentKind::Explore` 收紧；普通只读 runner、general、自定义 Agent 与嵌套委派不会因此放开。
- 同一轮里连续的 `concurrent_safe && !destructive && !needs_approval` 调用（Read / Glob / Grep / Lsp / WebFetch / WebSearch / Skill / TodoRead）并行执行，上限 8，结果按模型给出的顺序回填；写工具与 Bash 串行；连续 `Agent` 调用仍成批并行。
- 结果预算：输出超过 `result_budget.max_model_bytes` 且策略为 `Artifact` 时，完整内容写入 `$APPCONFIG/artifacts/<session>/<id>.txt` 并登记 `native_tool_artifacts`，模型只看到头（Glob / Grep / WebFetch / Agent / MCP）或尾（Bash）预览加 artifact 路径；`Read` 允许读取 artifact 目录。之后仍按 `max_tool_output_tokens` 截断兜底。
- 逐工具超时：Read / Write / Edit / Glob / Skill / Todo 30 秒，Grep 60 秒，ApplyPatch 60 秒，WebSearch 45 秒；WebFetch 单次抓取共用 30 秒网络预算，用户授权等待不计入；Bash 自带超时（默认 `bash_default_timeout_secs`，模型可覆盖到 600 秒）；Agent、AskQuestion 与 ExitPlanMode 不设超时。
- `Edit` 匹配策略链：exact → quote_normalized → line_number_prefix_stripped → escape_normalized → unicode_escape_normalized → indentation_flexible → line_trimmed → block_anchor，结果里注明命中策略；CRLF 文件保持 CRLF。本地 Write / Edit 会校验文件自上次 Read 后未被修改，否则要求重新 Read；文件不存在时给出同目录相近文件名提示。
- `Read` 支持 png / jpg / gif / webp：图片作为紧随工具结果的用户消息附件交给模型。
- `Bash`：会话开始时导出一次 login shell 快照（函数 / 别名 / shell 选项 / PATH）到 `$APPCONFIG/shell-snapshots/`，之后每次只 `source` 快照再 `eval` 命令；导出失败或关闭 `shell_snapshot_enabled` 时回退 `bash -lc`。`run_in_background=true` 把命令登记到会话进程表，立即返回 `process_id`；用 `ProcessList` / `ProcessOutput` / `ProcessStop` / `Monitor` 跟踪。仅本地会话支持后台 Bash。`Grep` 在 `rg_sidecar_enabled` 且找到打包的 `tools/rg` 或 PATH 上的 `rg` 时用 ripgrep，否则用 Rust 正则遍历。
- `Lsp`：本地会话按文件扩展名懒启动已安装的 language server，覆盖 Rust、TypeScript / JavaScript、Python、Go、C / C++、Java、Kotlin、C#、PHP、Ruby、Swift、Dart、Lua、HTML、CSS、JSON、YAML、Bash、Markdown、SQL、Vue、Svelte、Dockerfile 与 Terraform。支持 goToDefinition、findReferences、hover、documentSymbol、workspaceSymbol、goToImplementation、diagnostics；`workspaceSymbol` 可用 `language` 参数明确选择语言，并会先打开一份对应语言的源文件（避免 typescript-language-server 在空项目上报 `No Project`）。`Write` / `Edit` / `ApplyPatch` 成功后把诊断附到工具结果。设置项 `lsp_enabled` 默认开；独立 LSP 设置页（`/settings/lsp`）提供开关以及 language server 检测和安装按钮，会使用本机已有的包管理器执行固定安装命令；安装完成后需要重新打开会话。已安装项可点「测试」调用 `test_lsp_server`：在固定临时工作目录里启动该 server、完成一次 initialize 握手（可选再打一次 `workspace/symbol`；TypeScript / JavaScript 会先写入并打开一份源文件），随后 shutdown / exit，返回 serverInfo 名称版本与耗时，用于确认服务器真的可用。SSH 工作区不启动 language server。
- `Computer`：按应用的本机电脑控制（实现见 [`tools/desktop.rs`](../src-tauri/src/native/tools/desktop.rs)、[`app_target.rs`](../src-tauri/src/native/tools/app_target.rs)、[`background_input/`](../src-tauri/src/native/tools/background_input)），对齐 Codex Sky 的交互而不是隔离桌面。单一工具、单一权限能力 `computer`，动作为 `list_apps` / `get_app_state` / `click` / `set_value` / `type_text` / `press_key` / `scroll` / `drag` / `wait`。默认 `dispatch=background`：先 `get_app_state(app)` 拿到**单个窗口**截图和带编号的无障碍树（role / title / value / bounds / actions），再对这一轮的 `element_index`（或相对该窗口截图的坐标）动手；下标只对这一轮有效。后台优先走辅助功能动作，坐标则投递到目标进程（macOS `CGEventPostToPid`，Windows `PostMessage`，Linux AT-SPI / `XSendEvent`），不移动用户光标、不抢前台。后台做不到时返回中文 `background_unavailable`，**禁止**悄悄回退到全局 `enigo` / XTEST / SendInput。只有显式 `dispatch=foreground` 才允许现有光标注入，文案会写明会抢鼠标。契约仍为 `SideEffectScope::System`、`RiskLevel::High`、`needs_approval: true`、`destructive: true`、`concurrent_safe: false`、`allowed_in_plan_mode: false`，超时约 15 秒。`computer_control_enabled` 仍是总开关，默认关，写在 `native-settings.json`。SSH / 计划模式 / 子 Agent 不提供该工具。第一次操作某个应用要确认；「始终允许」写入 `capability=computer`、pattern 为 bundle id / exe 路径（`source=input`）。`yolo` 仍尊重显式 deny，总开关关闭时 yolo 也不能用。macOS 最接近 Codex（AX + `CGWindowListCreateImage` + 投递到 PID；窗口在其他 Space 上没有像素；Catalyst 会丢掉后台事件）。Windows 用 UIA + PrintWindow + PostMessage，Chromium / 部分 UWP 会丢后台消息。Linux 用 AT-SPI + 按窗口截图，Wayland 可能只能返回树。第一版不为 Chromium 单独接 CDP，也不走 Docker / Xvfb / CreateDesktop / Cua VM。设置页 `/settings/computer` 提供总开关、系统权限、已授权应用（可删）和三端能力说明；命令仍经 [`backend.ts`](../src/lib/backend.ts)，前端不直连 OS API。
- 本地 Bash 可开启操作系统沙箱（`bash_sandbox_enabled`，默认关）：Linux 用 `bwrap`（只读根 + PID 隔离 + 工作区 / `/tmp` / `/var/tmp` 可写，无网络隔离），macOS 用 `sandbox-exec`。找不到 `bwrap` / `sandbox-exec` 时回退并在结果里注明。SSH 不套沙箱。
- 首页 Composer 用「当前工作区 | 隔离工作树」两段切换（启动参数 `isolate_worktree`）：新会话创建 git worktree 并检出唯一分支 `noxcode/wt-<session>`（不再 detached）。本地默认 `~/.noxcode/worktrees/<session_id>`，也可在设置里改根目录；SSH 为 `$HOME/.noxcode/worktrees/<session_id>`。默认不隔离。非 git 仓库跳过并提示。开启「创建前拉取」时会先 `git fetch --all --prune`（失败仍创建）。开启自动清理时，现存托管工作树超过上限会删掉最旧且未在使用的（删前打 checkpoint）。会话内也可用 `EnterWorktree` / `ExitWorktree`。工具后 checkpoint 打在当前活动目录。删除会话或在设置页手动删除时会移除托管 worktree。任务写完后可在会话标题栏、本轮结束或进程退出时选择：合并回当前分支、建成新分支、或先保留；合并前可手写或用侧边栏同款能力生成提交说明。合并或建分支后隔离树会 reset 到打点提交（建成新分支还会切到该分支），避免侧栏仍把同一文件列在已暂存和未暂存。标题栏分支选择器与 Git 侧栏都看当前会话的隔离 worktree（若有）。合并冲突停在中间态；「AI 自动解决」发到当前会话里处理（先 ExitWorktree 再改主工作区文件，不在弹窗里等待 one-shot），回合结束后 `complete` 提交并切回当前隔离 worktree，后续对话不再落在主仓路径；也可打开 Git 面板手动处理，中止才 `merge --abort`。`ExitWorktree` 只暂时把活动目录改到主仓，不丢掉隔离路径；回合结束或 `restore_session_worktree` 会切回。
- 未验证 Shell 命令默认需要授权；重定向覆盖、`cp/mv`、所有 `git restore` 均进入风险判断。本地 Bash 同时排空两路输出并限内存，超时或取消时终止独立进程组；SSH Bash 透传 deadline / cancel 并发送终止信号、关闭通道。超出硬上限的输出仅保留尾部，不能从 artifact 恢复被丢弃前缀。
- 本地文件工具按真实路径及最近存在父目录检查边界，额外读写根保持各自权限，递归搜索不跟随符号链接；SSH 文件工具拒绝符号链接路径。SSH Write 支持防覆盖创建新文件，覆盖旧文件仍要求 Read 与内容指纹匹配。这些边界不等同于操作系统级 Shell 沙箱。
- 本地会话的 `Read / Glob / Grep` 默认允许只读访问当前有效技能目录及其附属文件，遵守启停、重名覆盖和子 Agent 的技能筛选。技能父目录及写入需额外授权，yolo 按完全访问处理。相对路径以工作区为基准，未指定路径的搜索只扫描工作区。链接按真实目录检查，递归搜索不跟随链接逃逸；直接访问外部链接目标仍须经过外部路径授权。
- `WebFetch` 有 15 分钟 / 50 MiB 的内存缓存，按当前会话与网络配置隔离；命中前仍重新检查权限、DNS 和缓存的重定向链。

## 钩子

七类事件：`session_start`（输出注入系统提示尾段）、`user_prompt_submit`（可阻断本次输入或注入上下文）、`pre_tool_use`（可阻断、改写参数、注入上下文）、`post_tool_use`（告警 / 注入上下文）、`post_tool_use_failure`（仅告警）、`permission_request`（可代替用户给出 allow / deny，ask 则继续弹窗）、`stop`（`continue: true` 或 `block` 要求模型继续，一个用户回合最多 3 次）。也接受 Claude Code 的 PascalCase 事件名。

处理器三种：`command`（shell，载荷在 `NATIVE_HOOK_PAYLOAD`，退出码 2 = 阻断）、`http`（POST JSON，2xx 响应体按同一协议解释）、`agent`（用当前会话模型做一次无工具判定）。输出协议 `{ decision: allow|deny|ask|block, reason, updated_input, additional_context, continue }`，兼容 `hookSpecificOutput.permissionDecision / updatedInput / additionalContext` 与 `stopReason`。

来源：设置页的全局钩子（`native-settings.json`）+ 本地工作区的 `.noxcode/hooks.json`（`{ "hooks": [...] }`）与 `.claude/settings.json` / `.claude/settings.local.json` 的 `hooks` 段（`type: prompt` 映射为 `agent`，`matcher` 的 `A|B` 转成工具名列表）。全局先执行，工作区后执行。实现见 [`tools/hooks.rs`](../src-tauri/src/native/tools/hooks.rs) 与 [`hooks_config.rs`](../src-tauri/src/native/hooks_config.rs)。

工作区及工作区插件贡献的钩子必须先通过 `WorkspaceHooks` 显式批准本次会话，拒绝、超时或取消均不执行。`yolo` 完全访问视为已信任并直接启用这些钩子；其它模式不受权限规则或 `permission_request` 钩子绕过，也不会连带放行其他工具。

## 子 Agent 档案与后台任务

- `.md` 档案：`<workspace>/.noxcode/agents/*.md`、`.claude/agents/*.md`、`$APPCONFIG/agents/*.md`。frontmatter：`name`（必填）、`description`、`tools`（逗号或数组；空 / `*` = 全部）、`disallowedTools`、`permissionMode`、`maxTurns`、`skills`（只对子 Agent 开放的技能名）、`injectAgentsMd`；正文即系统提示。与设置页 json 同名时 json 优先；档案 `source = file`，设置页只展示不可编辑。解析见 [`subagents.rs`](../src-tauri/src/native/subagents.rs) `parse_subagent_markdown`。设置页 json 子智能体指定渠道模型时可另选思考等级，未设置则用模型默认；`.md` 档案不支持。
- 后台任务：`Agent(run_in_background=true)` 立即返回 `task_id`，子 Agent 在独立 tokio 任务里运行（自己的 CancelFlag，父取消会级联）。父 Agent 用 `TaskOutput(task_id, wait, timeout_ms)` 读取 / 等待、`TaskStop` 取消、`SendMessage` 追加指令（进子 Agent 的 steer 通道）；子 Agent 用 `RespondToCoordinator` 留言。完成与留言在父 Agent 下一次模型调用前以 `[后台任务提醒]` 注入。注册表见 [`agent/background.rs`](../src-tauri/src/native/agent/background.rs)；会话结束时停掉全部后台任务。
- Agent 特殊调度与普通工具共用权限、只读检查及 Hook 前后置入口；前台和后台跨批共享同一并发许可。后台状态包含 queued / running / done / failed / stopped，消息在模型调用边界和最终返回前消费，队列满或任务已关闭立即报错。前端可查看任务、发送消息与停止任务，使用 `list_native_background_tasks`、`send_native_background_message`、`stop_native_background_task`。

## 自动化、目标与跨会话上下文

- Cron 自动化（[`scheduler.rs`](../src-tauri/src/native/scheduler.rs)）：五段 cron + `@hourly/@daily/@weekly/@monthly`，本地时区算 `next_run_at`；调度器每 30 秒扫描，工作区有会话在工作中则推迟 1 分钟，到期时用 `start_native_with_manager` 启动新会话（提示词前缀 `[自动化 名称]`）。工具 `CronCreate` / `CronList` / `CronUpdate` / `CronDelete`；写操作需确认（`kind = automation`）；命令 `list/create/update/delete_native_automations`、`run_native_automation_now`；设置页「自动化」。
- `CronUpdate(id, name?, prompt?, cron?, enabled?, channel_id?, model?)` 仅主 Agent 可用，计划模式禁止调用；只能更新当前工作区的自动化，至少提供一个更新字段。未提供字段保持不变，名称/提示词/cron 不接受空白；渠道和模型可用空字符串清除，清除渠道同时清除模型。修改渠道/模型会校验有效组合；仅 cron 或启停变化重算下次执行时间，改名或改提示词保留调度时间及既有运行记录，不立即执行。
- 目标（[`goals.rs`](../src-tauri/src/native/goals.rs)）：`Goal(action=set|update|complete|clear, title, checklist, note)` 维护会话的当前目标与进度清单，`GoalRead` 读取；每次变更写 `[GOAL] {json}` 行，前端渲染为 `GoalRow`。
- `ReadSessionContext`：不带 `session_id` 列出同工作区最近会话（标题、时间、轮数、最后回复摘录）；带 `session_id` 仍校验工作区归属，再返回最近的用户 / 助手对话摘录。
- `/fork` → `fork_native_session`：从最新已提交边界新建已结束会话（标题加「（分叉）」，`resume_session_id` 指向源会话），活动分支引用源历史而不是复制消息行。不回滚文件；带检查点参数会拒绝，文件回滚要先预览再应用。消息菜单还可以从某条用户消息之前或之后分叉，或在当前会话回退：回退封存原分支、建立新的活动分支，不删除后面的历史。默认保留到所选消息。会拆开工具调用的位置不能选。工作中拒绝修改边界。计划正文留在历史里，旧审批和运行授权不继承；已完成目标仍保留来源分支，但不能当作新分支的完成证据。聊天展示从最后一条 `[分支]` 标记重新开始。实现见 [`history.rs`](../src-tauri/src/native/history.rs)。
- 消息上的「回滚文件」先调用 `preview_native_file_rollback`，列出新增、删除、修改、冲突和不能自动回滚的项，并写明不撤销外部 API、消息发送，以及 Bash / MCP 等无法归属的副作用。确认时按「仅对话 / 仅文件 / 对话和文件」调用 `apply_native_file_rollback`。凭据和分支修订号在应用前重算，不一致则不写任何文件；任一冲突也不写。文件成功之后才切换对话，仅文件不改分支。回滚直接写绑定的本地、SSH 或隔离 worktree 根，不改 Git HEAD 和用户 index，也不回退到工作区默认目录。执行目标在回滚期间拒绝应用内的受控写入。中途失败按已保存的备份补偿；补偿时若文件又被改过，状态变为 `needs_recovery` 并保留备份。恢复会话时先处理未完成的回滚，`needs_recovery` 会挡住继续执行。实现见 [`file_rollback.rs`](../src-tauri/src/native/file_rollback.rs)。
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
- 重试：`model_retry_*` 设置控制指数退避（默认 6 次、1 s 起、上限 30 s、倍数 2、带抖动），服务端 `Retry-After` 优先但不超过上限。不另起一套恢复重试。每次尝试写入 `native_api_call_logs.attempt`，并计入 `native_model_attempt_budgets`。进程在请求中途退出后，同一回合接着已用次数，不会把预算清零；这一次请求成功后计数归零，下一次请求重新计算。次数用尽时返回「重试次数已用尽」。首包等待、流空闲、请求总期限、网络中断和缺少终止事件的半截流可重试；用户取消、协议错误、上下文上限和 4xx 认证/参数错误不重试。重试前清空已流式显示的半截内容，已提交的续写正文保留。前端通过 `[重试]` 行看到原因、等待时间和 `第 n/6 次`。
- 流期限：普通模型请求总期限和首包等待是 120 秒，思考模型是 300 秒。流空闲默认 60 秒，只由有效协议事件刷新；SSE 注释和 `ping` 不刷新，也不能把请求延长过总期限。发送、读流和退避等待都可以被取消。分类是首包等待、流空闲、请求总期限、网络中断、无合法终止、协议错误、提供商错误和用户取消。

模型层通过 `ModelResponse` 返回消息、usage、提供商原始结束字段、规范化结束类型和 Responses ID；传输中断、无效响应、取消、提供商错误与上下文错误保留结构化分类。完整 JSON 或合法终止事件可兼容缺少结束原因的网关：OpenAI SSE 需要 `finish_reason` 或 `[DONE]`，Anthropic 需要 `message_stop`，Responses 需要完整响应级终止事件。未知原因的显式 incomplete 状态、无终止证据、错误事件和无效工具批不能作为成功响应；增量 SSE 解析失败不会再退回文本缓冲区绕过检查。

输出达到 token 上限时，主 Agent 与真实子 Agent 保留有效文本/思考内容，丢弃该响应的工具调用，再自动续接，**每个用户回合最多额外调用 3 次**。压缩和转向不重置次数；取消、预算不足或次数耗尽会停止并报告未完成。续接提示只属于当前待恢复请求，完成、转向和新用户回合后不残留；正常完成的工具/stop-hook 后续步骤不携带旧的部分文本前缀。上下文上限沿用被动压缩流程。摘要、记忆、Hook 和一次性生成不自动续接，遇到部分结果会报告失败。OutputLimit/ContextLimit/Refusal 会清除 Responses 服务端续接锚点，避免继续携带已被本地丢弃的工具调用。

助手输出使用 `assistant: { chain_id, part, subagent_tag? }` 标识片段，贯穿实时 delta、持久化事件及历史回放。同一文本链按片段序号精确拼接，跨 thinking/usage 事件仍保留完整 Markdown 与代码围栏，复制结果与显示一致；独立文本链保持段落分隔。旧的无身份历史保持原有显示，不通过文本相似度猜测拼接关系。

## 工具与 MCP 子进程环境

会话启动时把 `network-settings.json` 转成代理 / CA 环境变量：代理写入大小写 `HTTP_PROXY` / `HTTPS_PROXY`，不代理地址写入 `NO_PROXY`，自定义 CA 写入 `SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS`。本地 Bash 和本地 MCP 子进程会注入这些变量；MCP 自身的 `env` 随后应用。本地 stdio MCP 还会把 Homebrew / nvm / fnm / volta 等常见目录并入 PATH，并把 `npx` 这类裸命令解析成绝对路径后再 spawn，避免桌面进程 PATH 过短导致 `os error 2`。SSH 远端 Bash 与远端 MCP 不注入本机网络设置。

子 Agent 克隆父 Agent 的 `ToolCtx.extra_env`，因此本地 Bash 的网络环境在子 Agent 中保持一致。

`WebFetch` 始终从 Agent 所在本机请求，SSH 工作区不改变网络主机。只允许 HTTP/HTTPS，直连每一跳先解析并检查全部候选 IPv4/IPv6，再将连接固定到已检查地址，保留原始 Host 和 TLS 主机名校验；混合公网/非公网 DNS 也需要非公网授权。最多跟随 5 次重定向，每跳重新检查，不向重定向目标转发 URL 凭据。网络等待累计最多 30 秒，读取达到 512 KiB 即停止并标明内容可能不完整。

非公网、回环及特殊用途地址使用 `network_origin` 确认，可允许本次或当前会话的同一 origin（协议、主机、端口）。`WebFetch` 每次读取应用明确保存的代理、NO_PROXY 和自定义 CA，禁用隐式环境代理；NO_PROXY 命中走相同的严格直连检查。代理使用 `network_proxy` 单独确认，说明代理侧 DNS/目标 IP 无法由本机核验，信任绑定代理与网络配置，配置或 CA 内容变化后重新确认；代理失败直接报错，不回退直连。

这些确认不生成普通工具 allow 规则，也不能由网页或 `permission_request` Hook 授予；「当前会话允许」只保存对应网络信任，不开启全局完全访问。显式 deny 始终优先，`yolo` 按既有完全访问语义免确认。私网授权按会话/origin 保存，缓存读取仍检查当前访问链，不能借其他会话或旧代理配置绕过授权。实现见 [`web_access.rs`](../src-tauri/src/native/tools/web_access.rs)、[`web_dns.rs`](../src-tauri/src/native/tools/web_dns.rs) 和 [`web.rs`](../src-tauri/src/native/tools/web.rs)。

## 命令

会话：`start_native_session`、`stop_native_session`、`stop_native`、`restart_native_session`、`resume_native_session`、`send_native_input`、`submit_native_steer`、`get_native_steer_snapshot`、`finish_native_input`、`resolve_native_tool_permission`（决策含 `allow_always`）、`answer_native_plan_question`、`resolve_native_plan_approval`（可选 `ai_channel_id` / `model` / `reasoning_effort`，批准时热更换实施模型与思考等级）、`compact_native_session`。

工作区 / 历史：`list/create/update/delete_workspace`、`check_workspace_health`、`list_agent_sessions`、`get_agent_session_log_lines`、`prepare_agent_session_resume`、`set_agent_session_pinned`、`delete_agent_session`、`list_activity_logs`。

设置：`get/update_native_settings`、`list_native_global_skills`、`list_native_skills`、`open_native_skills_dir`、`create/delete/copy/import/scan/set_enabled` 技能命令、`list/create/generate/update/delete_native_subagent`、`get/update/reset_mcp_servers`、`export_mcp_servers_snippet`、`list/get_native_api_call_log`。

删除工作区前会 `clear_workspace_checkpoints`。删除会话前会 `delete_checkpoints_for_session`。运行中的渠道 / 工作区 / 会话拒绝删除。

## 事件

| 事件 | 载荷 |
| --- | --- |
| `native-session` | `AgentSessionStarted` |
| `native-input-queue` | `session_record_id` + `queue_id` + `revision` + `items(id/text/image_count/editing)`，待执行指令完整快照。 |
| `native-steer` | `session_record_id` + `instance_id` + `turn_id` + `revision` + `receipts` + 可选 `lifecycle`，当前回合与转向状态快照。 |
| `native-request-resolved` | `session_record_id` + `request_id` + `kind(permission/question/plan_approval)`，仅清除对应请求。 |
| `native-background-tasks` | `session_record_id` + `tasks`，后台任务完整快照。 |
| `native-stdout` | `AgentSessionOutput`（已写入 `agent_session_events`）。工具 start/result 带可选 `tool`（`call_id` / `name` / `title` / `ok` / `duration_ms` 等）和 live-only `images`；助手正文带可选 `assistant` 片段身份。落库 `message` 为 `{"nox":1,"line":"...","tool":{...},"assistant":{...}}` 信封（未使用字段省略），旧纯文本行仍可回放。 |
| `native-text-delta` | `NativeTextDelta`（仅展示，不落库）；携带运行实例及回合身份，可带 `assistant`，与已提交片段精确拼接，重试清空只影响未提交内容。 |
| `native-context-usage` | `NativeContextUsage`（`used` = 工具 schema + 消息；分类字段 + 上次调用 `prompt_tokens` / `cached_tokens`；仅父 Agent；同时写入 `agent_sessions.context_usage_json`） |
| `native-turn-state` | `session_record_id` + `instance_id` + `turn_id` + `steer_turn_id` + 单调 `revision` + `state`（`waiting_input` / `working`，不落库）；`steer_turn_id` 仅在信箱仍开放时非空，空闲压缩不开放转向。 |
| `native-plan-mode` | `NativePlanModeChanged`（`session_record_id` + 当前 `plan_mode`，不落库） |
| `native-permission-request` | 高风险工具确认（含 `suggested_rule`） |
| `native-plan-question` | `AskUserQuestion` 提问（所有模式可用） |
| `native-plan-approval-request` | `ExitPlanMode` 提交的计划，等待批准 / 退回 |
| `native-exit` | `AgentSessionExit`，含退出的 `instance_id`，不能清除同一会话的新运行实例。 |

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
