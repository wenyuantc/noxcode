# Native Agent 代码分析

noxcode 的 Agent 是**进程内 Native Agent**：模型调用、工具执行、权限、压缩、子 Agent 全部在 Rust 里跑。前端只通过 Tauri IPC 发命令、听事件，从不直接碰 SQLite。

```
React (Composer / EventStream / Zustand)
        │ invoke / listen
        ▼
session.rs  ←→  NativeAgentManager（live 会话表）
        │
        ▼
AgentRunner（主循环） → ModelClient（OpenAI / Anthropic / Codex）
        │                    ↕
        │              native_session_transcripts（模型续聊）
        ▼                    ↕
ToolCtx / dispatch           agent_session_events（UI 回放）
        │
        ▼
本地文件 / SSH / MCP / Git / SQLite
```

内核（`native/agent`、`native/tools`、`native/model`）从参考实现搬过来；外壳（`session.rs` / `manager.rs`）按「工作区 + 渠道 + 会话」重写，去掉了员工档案、任务队列那套业务骨架。

---

## 1. 模块地图

核心代码大约 **2.8 万行**，集中在 `src-tauri/src/native/`。

| 层 | 路径 | 体量 | 职责 |
| --- | --- | --- | --- |
| 会话外壳 | `session.rs` | ~5700 行 | 启动 / 停止 / 续聊 / 权限 IPC / 计划审批 / 事件转发 |
| live 表 | `manager.rs` | ~1900 行 | 同一工作区多 live、followup 通道、排队输入、待确认请求 |
| 主循环 | `agent/loop.rs` | ~5000 行 | 模型回合、工具批处理、压缩、子 Agent、transcript 检查点 |
| 压缩 / 截断 | `agent/compact.rs` + `truncate.rs` | ~2000 行 | 4 种压缩触发、token 预算、工具结果截断 |
| 子 Agent | `agent/subagent.rs` + `background.rs` | ~800 行 | 类型解析、派生 runner、后台任务注册表 |
| 工具调度 | `tools/dispatch.rs` | ~3700 行 | 权限预检、超时、本地 / SSH 分发 |
| 工具契约 | `tools/catalog.rs` + `contract.rs` | ~1500 行 | 每个工具的只读/风险/并行/预算/超时元数据 |
| 模型 | `model/client.rs` + 三协议 | ~2400 行 | HTTP、SSE、重试、prompt cache、call log |
| 提示词 | `prompt/` | ~400 行 | identity → 策略 → AGENTS.md → skills → 易变环境块 |
| 前端 | `sessionStore` + `useNativeEvents` + `EventStream` | — | 事件入 store，按行渲染 |

`AgentRunner` **不直接写库**。它通过 `on_event` / `on_usage` / `on_checkpoint` 把输出交给 `session.rs`，由会话层落库并 `emit` 给前端。

---

## 2. 会话是可多次激活的逻辑记录

`agent_sessions` 一行 = 一条可反复唤醒的会话，不是一次进程。

- **新开会话**：`start_native_session` 插入行，`status=running`。
- **live 续发**：runtime 还在 → 输入进 FIFO 队列（最多 8 条），当前回合结束后再跑。
- **冷恢复**：runtime 已死 → 原位重激活同一 `session_record_id`，从 `native_session_transcripts` 静默恢复，不清历史事件 / checkpoint / 累计 token。
- **`/fork`**：复制 transcript 到新行，可选先回滚到某个 Git checkpoint。

两张表刻意拆开：

- `agent_session_events`：UI 回放（带 `[USER_INPUT]`、工具行、压缩分隔线）
- `native_session_transcripts`：模型真正续聊的 messages JSON

没有数据库级同步约束。顶层 runner 在这些边界 UPSERT transcript（fingerprint 未变则跳过）：用户消息入列后、steer 注入后、每轮 assistant/tool 写完、loop 退出前。子 Agent **不写**父会话 transcript。

同一工作区可以同时有多个 live。删除工作区要求该工作区没有 live。

---

## 3. 一次用户回合怎么跑

`session.rs` 的 `run_native_loop` 是外壳；`AgentRunner::run_with_client` 是内核。

```
begin_user_turn
  └─ user_prompt_submit 钩子（可阻断 / 注入）
  └─ 记忆 recall 作为 turn_suffix（不进事件流）
checkpoint_transcript
loop:
  inject_steer（排队输入 / /compact / Finish）
  prepare_model_call
    ├─ 后台任务完成提醒
    ├─ 压缩（manual / auto / downshift）
    ├─ 工具 schema 是否撑爆窗口
    ├─ truncate_messages_tokens
    └─ 预算耗尽 → last_turn（去掉工具）
  client.chat(...)  流式 Delta
    └─ 上下文溢出 → reactive compact（最多 2 次）再重试
  consume_assistant
    ├─ 无工具 → stop 钩子（最多继续 3 次）→ 结束回合
    └─ 有工具 → execute_tool_calls
checkpoint_transcript
等待输入 / 配置热更新 / 下一轮
```

空闲时 `recv_idle_wait` 三路 `select`：followup 通道、配置通道、输入队列。配置变更（渠道 / 模型 / 思考等级）可以在空闲时热切换；live 续聊若配置对不上会拒绝，要求先正常结束再开。

工作中追加的消息**不是**即时 steer：等当前工具和子 Agent 全部结束、输出排空后，才按 FIFO 出队。队首正在编辑时暂停出队。队列只属于当前 runtime，停止或退出即清空。

---

## 4. 工具系统：契约驱动，不是硬编码白名单

每个内置工具在 `catalog.rs` 声明一份 `ToolContract`：

- `read_only` / `destructive` / `concurrent_safe`
- `side_effect_scope` / `risk_level` / `needs_approval`
- `allowed_in_plan_mode`
- `permission`（能力：read / edit / bash / mcp / …）
- `result_budget`（Truncate 或 Artifact）
- `timeout`

MCP 工具按 `tools/list` 的 `readOnlyHint` / `destructiveHint` 动态生成契约，缺省视为需审批、串行。

**同一轮调度**（`execute_tool_calls`）：

1. 连续 `Agent` → 成批并行（信号量限制）
2. 连续 `concurrent_safe && !destructive && !needs_approval` → 并行，上限 8
3. 其余（Write / Edit / Bash / MCP 写）→ 串行
4. 相同参数连续 3 次 → 拒绝（`REPEAT_TOOL_LIMIT`）

内置工具大致分四组：

| 组 | 工具 |
| --- | --- |
| 文件 | Read / Write / Edit / ApplyPatch / Glob / Grep / SQLiteQuery |
| 执行 | Bash（可后台）/ ProcessList / ProcessOutput / ProcessStop / Monitor |
| 认知 | Lsp / WebFetch / WebSearch / Skill / TodoRead / TodoWrite |
| 会话 | Agent / AskUserQuestion / EnterPlanMode / ExitPlanMode / Goal / Cron* / ReadSessionContext / EnterWorktree / ExitWorktree |

SSH 会话去掉 `SQLiteQuery`，不启动 LSP，不套本机 Bash 沙箱，后台 Bash 也不可用。

工具结果超预算时：完整内容落到 `$APPCONFIG/artifacts/<session>/<id>.txt`，模型只看到头（Glob/Grep/WebFetch）或尾（Bash）预览。`Read` 可以读 artifact 目录。之后还有 `max_tool_output_tokens`（默认 4096）截断兜底。

---

## 5. 权限是独立裁决层

规则在风险分类**之前**裁决：`deny → allow → ask → 未命中`。

四档 `permission_mode`：

| 模式 | 行为 |
| --- | --- |
| `default` | 变更前确认 |
| `edit` | 自动放行覆盖（Overwrite） |
| `build` | 再放行不透明 shell 与只读 MCP |
| `yolo` | 完全访问，不弹确认；**deny 仍拒绝** |

计划模式是**会话态**，不是第五档权限。Composer 启动时可进，模型也可 `EnterPlanMode`。解除必须用户批准 `ExitPlanMode`；拒绝 / 取消 / 无审批通道都保持计划模式。

计划模式里：

- 只读工具按契约 `allowed_in_plan_mode` 放行
- Bash：已审计只读命令直接跑；写入 / 不透明 / 高风险需授权（本次 / 始终 / 当前会话全部）
- `Write / Edit / ApplyPatch` 和写入型 MCP 禁止
- `Agent` 仍出现在工具列表里，但执行层只接受显式 `explore`

待批准计划写入 `agent_sessions.pending_plan_json`。**停止会话和应用退出都不清**，所以重开后前端还能还原一张 detached 审批卡：批准会以 `plan_mode=false` 续聊并带上完整计划正文（因为 transcript 里未完成的工具对可能已被清洗）。

「当前会话允许所有命令」是内存态：不写权限文件、不切 yolo、不影响其他会话和非 Bash 工具，会话结束即失效。

---

## 6. 子 Agent：派生 runner，不是另起进程

`Agent(prompt, description, subagent_type, run_in_background?)`

三种类型：

- `explore`：强制只读 + 只读工具白名单（含只读 Bash）
- `general`：继承父 MCP / 额外工具
- `custom`：来自 `.noxcode/agents/*.md` / `.claude/agents` / 设置页 JSON；可限工具、轮次、技能、独立 `permission_mode`

派生时（`spawn_child_with_quota`）：

- 共享：取消、权限规则、MCP 放行、代理环境、artifact、rollout 预算
- 独立：已读文件指纹、待办、`depth+1`、自己的 `ChildQuota`（剩余预算 × `subagent_budget_share_percent`）
- 事件带 `[子 Agent N(kind) - 描述]` 前缀
- **禁止嵌套**：`depth > 0` 再调 `Agent` 直接报错
- 子 Agent 不写父 transcript，也没有 AskUser / 计划模式 / Cron / Goal

后台任务：`run_in_background=true` 立刻返回 `task_id`，独立 tokio 任务 + 独立 CancelFlag，父取消会级联。父用 `TaskOutput` / `TaskStop` / `SendMessage`，子用 `RespondToCoordinator`。完成和留言在父下一次模型调用前以 `[后台任务提醒]` 注入。

---

## 7. 上下文：压缩、截断、预算三道闸

**压缩**统一入口 `run_compaction`，来源链：

微压缩（旧工具结果换成一行占位）→ 模型摘要（优先 `lite_model`）→ 本地摘要 → 重置

触发：

- `auto`：用量 ≥ 窗口 × 阈值（默认 85%）
- `manual`：`/compact [指令]`，空闲立刻做，工作中等到下次模型调用前
- `reactive`：供应商报溢出，最多 2 次/回合
- `downshift`：恢复到更小窗口模型，首轮前压缩

每次写 `[COMPACT_BOUNDARY] {...}`，前端渲染成分隔线。

**截断**：ASCII 每 4 字符 ≈ 1 token，CJK 每字符 ≈ 1 token（刻意保守）。工具结果超限保留头 2/3 + 尾 1/3。请求前再按比例缩短旧消息，保护 system 和最近一条 user，最后清洗孤立 tool pair。

**预算**：`RolloutBudget` 父子共享；耗尽后去掉工具、要求直接作答。预留失败会再试一次无工具请求，再失败就本地结束。

系统提示把日期 / Git / 权限模式放到**最后**，静态前缀才能打中 prompt cache。

---

## 8. 前端只做投影

提交路径很薄：

```ts
submitSessionPrompt → startNativeSession | resumeNativeSession
```

`useNativeEvents` 把所有 `native-*` 事件灌进 `sessionStore`：

- `native-stdout` → 行（落库回放）
- `native-text-delta` → live 碎片（不落库）
- `native-turn-state` → working / waiting_input
- `native-permission-request` / `plan-question` / `plan-approval-request` → 弹卡
- `native-input-queue` / `native-background-tasks` → 完整快照（带 revision，丢弃过期）

`EventStream` 把行聚成 turn block：思考、计划、工具摘要、子 Agent 行、压缩边界、Goal、用户气泡。虚拟列表在 24 行以后启用。

前端**没有**自己的 agent 循环。斜杠命令里 `/mode` `/model` `/compact` 等由前端拦截；`/init` `/goal` `/review` `/create-skill` 展开成提示词后走普通回合。

---

## 9. 设计上真正立住的几件事

1. **UI 与模型上下文分离**。事件流可以丢、可以信封化（`{"nox":1,"line":"...","tool":{...}}`），模型续聊只认 transcript。硬中断至少能恢复当前用户任务和已完成的工具轮。
2. **契约比白名单稳**。计划模式、并行、超时、artifact、权限能力都从同一份 `ToolContract` 推导，MCP 也能套进去。
3. **排队输入不是打断**。工作中的消息等到回合边界才执行，避免半截工具结果被新指令搅乱。
4. **计划模式是运行时状态，不是权限档**。退出必须用户批准；停止后 pending plan 仍能 detached 续上。
5. **子 Agent 是同一套 loop 的派生**，不是第二套运行时。只读 / 工具集 / 权限 / 预算用 fork 收紧。
6. **本地和 SSH 共用 ToolCtx**。文件边界、权限规则、Bash 风险判断走同一套；SSH 只关掉本机才有的能力（LSP、SQLiteQuery、sandbox、后台进程）。

---

## 10. 复杂度与阅读入口

`loop.rs`（5000 行）和 `session.rs`（5700 行）是两个重心：前者是「怎么跑一轮」，后者是「怎么把一轮接到桌面应用」。`dispatch.rs` 是第三块，几乎所有工具副作用都从这里出去。

建议阅读顺序：

1. `docs/native.md` — 产品语义和生命周期
2. `agent/README.md` — 循环、压缩、子 Agent 常量
3. `AgentRunner::run_with_client` → `prepare_model_call` → `consume_assistant` → `execute_tool_calls`
4. `start_native_session_locked` → `run_native_loop` 的 idle wait
5. `ToolCtx` + `enforce_permissions` + `catalog.rs` 的契约表
6. 前端 `sessionSubmission.ts` → `useNativeEvents` → `EventStream`

这套代码的本质是：**一个带权限、压缩、子 Agent 和持久化的 in-process tool-calling loop**，外面包了一层可多开会话的桌面运行时。模型协议是可替换的；会话、权限、工作区才是这个项目自己的产品内核。
