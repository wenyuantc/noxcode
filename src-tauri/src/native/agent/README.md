# native/agent 模块

内置 Agent 的主循环与上下文管理。`session.rs` 负责启动 / 停止 / 续聊，`manager.rs` 管理运行中会话，本模块只做一件事：把「用户输入 + 工具 + 模型」跑成一个受控的回合循环。数据流仍是 `React → Tauri IPC → Rust → SQLite`，本模块不直接碰数据库，只通过事件通道把输出交给上层。

> 本文档以当前代码为准；模块演进后如有出入，以代码为准。

## 文件职责

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 模块声明（`#![allow(dead_code)]`） |
| `loop.rs` | `AgentRunner` 主循环：模型回合、工具批处理、steer 注入、transcript 检查点、预算预留 |
| `compact.rs` | 上下文压缩（4 种触发 × 4 种来源）与 token 预算（`RolloutBudget` / `ChildQuota`） |
| `truncate.rs` | 保守 token 估算、工具结果截断、请求前消息缩短、孤立 tool pair 清洗 |
| `subagent.rs` | 子 Agent 参数解析、派生 runner、系统提示组装、结果截断 |
| `background.rs` | 后台子 Agent 任务注册表（`task_id` / 状态 / 留言） |

## 主循环

`AgentRunner::run_with_client` 是主入口（`loop.rs:960`），每个用户回合的流程：

1. `begin_user_turn`：写入用户消息（可带图片），检查取消。
2. `checkpoint_transcript`：把当前消息同步到 `native_session_transcripts`（子 Agent 跳过）。
3. `inject_steer_messages`：排空 `steer_rx`——追加输入（`Input`）、挂起 `/compact`（`Compact`）、标记收尾（`Finish`）。
4. `prepare_model_call`：轮次上限检查 → 后台任务完成 / 子 Agent 留言提醒注入 → 压缩触发判定（manual / auto / downshift）→ 工具 schema 预算检查（`tools_fit`）→ `truncate_messages_tokens` 兜底 → 预算耗尽检查 → 判定 `last_turn`（无工具直接作答）。
5. 模型调用：`client.chat(...)`，流式结果经 `NativeEvent::Delta` 转发；供应商报上下文溢出时 `try_reactive_compaction` 被动压缩后重试（一个回合最多 2 次）。
6. `consume_assistant`：思考行与文本 emit → 无工具调用则跑 stop 钩子（最多要求继续 3 次）后结束回合；有工具调用则 `execute_tool_calls`。
7. 每轮结束再 `checkpoint_transcript`，循环直到模型给出最终文本。

`run_scripted`（`loop.rs:1170`）是测试入口：从预置 `replies` 队列取 assistant 消息，不调模型。`run_child_with_client` 是子 Agent 专用入口，工具串行执行，`Agent` 调用直接拒绝（防嵌套）。

### 工具批处理（`execute_tool_calls`）

- 连续 `Agent` 调用成批交给 `run_agent_batch`（并行，受信号量限制）。
- 连续 `concurrent_safe` 只读工具成批并行（`run_parallel_batch`，上限 `MAX_PARALLEL_TOOL_CALLS = 8`），结果按模型给出的顺序回填。
- 其余工具串行执行；重复调用受 `REPEAT_TOOL_LIMIT = 3` 限制。

## 上下文管理

### 压缩（compact.rs）

统一入口 `run_compaction(trigger, instructions)`（`loop.rs:427`），来源链：**microcompact → model 摘要 → local 摘要 → reset**，取第一个能缩减的。

| 触发 | 场景 |
| --- | --- |
| `auto` | `total_tokens ≥ 窗口 × threshold_percent`（默认 85%） |
| `manual` | `/compact [指令]`，等待输入时立即执行，工作中在下一次模型调用前执行 |
| `reactive` | 模型返回上下文溢出错误，被动压缩后重试（≤2 次/回合） |
| `downshift` | 会话恢复到更小窗口的模型，首轮调用前压缩 |

每次压缩写一行 `[COMPACT_BOUNDARY] {trigger, source, pre/post_tokens, pre/post_messages, instructions}` 到事件流，前端渲染为分隔线。microcompact 只把最近 6 条之外、超过 400 字的工具结果替换成一行占位。

### 截断（truncate.rs）

- 估算：ASCII 每 4 字符 ≈ 1 token，非 ASCII 每字符 ≈ 1 token（`estimate_text_tokens`），刻意保守以覆盖 CJK。
- 工具结果：超过 `DEFAULT_TOOL_RESULT_TOKEN_LIMIT = 4096` 时保留头 2/3 + 尾 1/3，中间用省略标记 + 续读提示（`truncate_tool_result`）；完整内容仍进事件流，模型只看到截断版。
- 请求前：`truncate_messages_tokens` 先逐条截断工具结果，仍超限则按比例缩短旧消息（保护 system 与最近一条 user），最后 `sanitize_tool_message_pairs` 清洗孤立 tool pair。

### 预算（compact.rs）

- `RolloutBudget`：父 rollout 与所有子 Agent 共享的 token 预算，`0` = 无限；每次模型调用先 `try_reserve`，失败则降级为无工具最终回答（`append_budget_reminder` / `finish_without_model`）。
- `ChildQuota`：子 Agent 按 `剩余预算 × subagent_budget_share_percent` 分到的独立配额。
- 预算耗尽后 `last_turn` 置位，模型不再拿到工具。

## 子 Agent 与后台任务

`Agent` 工具调用经 `run_agent_batch`（`loop.rs:1940`）处理：

- 类型：`general` / `explore` / `custom`（`subagent.rs` 的 `SubagentKind`）。`explore` 强制只读白名单；`custom` 按档案限工具、轮次、技能与权限模式。
- 派生（`spawn_child_with_quota`）：共享取消、权限放行、MCP 放行、代理环境与 artifact 存储；已读文件与待办独立（`fork_for_child`）；`depth + 1`，事件带 `[子 Agent N(kind) - 描述]` 前缀；子 Agent 不写父会话 transcript。
- 嵌套拒绝：`depth > 0` 时子 Agent 再调 `Agent` 直接返回错误。
- 后台任务（`run_in_background=true`）：立即返回 `task_id`，子 Agent 在独立任务里跑（`background.rs`）；父 Agent 用 `TaskOutput` 取结果、`TaskStop` 取消、`SendMessage` 追加指令，子 Agent 用 `RespondToCoordinator` 留言；完成 / 留言在父 Agent 下一次模型调用前以提醒注入（`prepare_model_call` 里的 `pending_notice`）。父会话取消时级联取消后台任务。
- 结果报告超过 `SUBAGENT_RESULT_CHARS = 16_000` 字符时截断。

## 事件与 transcript 同步

`NativeEvent`（`loop.rs:222`）：`Line`（完整行，落库）、`Delta`（live 片段，不落库）、`Tool`（工具 start/result + 图片）、`ContextUsage`、`UserInput`、`Flush`。`on_event` / `on_usage` / `on_activity` 通道由 `session.rs` 接线转发给前端。

`checkpoint_transcript` 在以下边界 UPSERT transcript（fingerprint 未变则跳过）：用户消息进入后、steer 注入后、每轮 assistant 文本或工具结果写完整后、`run_native_loop` 退出前。子 Agent 不写父会话 transcript。

## 关键常量

| 常量 | 值 | 位置 |
| --- | --- | --- |
| `DEFAULT_CONTEXT_CHARS` | 120_000 | loop.rs |
| `MAX_PARALLEL_TOOL_CALLS` | 8 | loop.rs |
| `REPEAT_TOOL_LIMIT` | 3 | loop.rs |
| `MAX_STOP_HOOK_CONTINUES` | 3 | loop.rs |
| `FALLBACK_OUTPUT_TOKEN_GUARD` | 16_384 | loop.rs |
| `DEFAULT_TOOL_RESULT_TOKEN_LIMIT` | 4_096 | truncate.rs |
| `MAX_CONCURRENT_SUBAGENTS` | 3 | subagent.rs |
| `SUBAGENT_RESULT_CHARS` | 16_000 | subagent.rs |
| `COMPACT_BOUNDARY_PREFIX` | `[COMPACT_BOUNDARY] ` | compact.rs |

## 相关模块与文档

- `../session.rs`：会话启动 / 停止 / 续聊，构造 `AgentRunner` 并消费其事件。
- `../manager.rs`：live 会话管理、`NativeFollowup`（输入 / 压缩 / 收尾）。
- `../tools/`：工具契约（`contract.rs`）、执行与权限（`dispatch.rs` / `permission.rs`）。
- `../model/`：`ModelClient` 与三协议客户端。
- `../prompt/`：系统提示组装（identity / 环境 / Git / 项目指令 / skills）。
- 完整会话生命周期与六条链路验证见 [`docs/native.md`](../../../docs/native.md)。