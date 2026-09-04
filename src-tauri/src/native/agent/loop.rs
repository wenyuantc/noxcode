use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::db::models::{NativeToolEvent, NativeToolPhase};
use crate::engine::UsageDelta;
use crate::native::artifacts::{bound_with_artifact, ArtifactStore};
use crate::native::manager::NativeFollowup;
use crate::native::model::call_log::{
    CALL_KIND_COMPACT, CALL_KIND_SUBAGENT, MODEL_ROLE_LITE, MODEL_ROLE_MAIN, OPERATION_COMPACT,
    OPERATION_SUBAGENT,
};
use crate::native::model::client::{ChatRequest, ModelClient};
use crate::native::model::types::{
    Message, NativeImage, Role, StreamDelta, ToolCall, ToolSpec, Usage,
};
use crate::native::model::usage_to_delta;
use crate::native::settings::DEFAULT_NATIVE_MAX_TURNS;
use crate::native::subagents::{
    custom_tools_are_read_only, effective_custom_tools, find_native_subagent, ChildModelSettings,
    NativeSubagent, MODEL_MODE_CHANNEL, TOOL_MODE_ALL,
};
use crate::native::tools::contract::ContractRegistry;
use crate::native::tools::hooks::{run_stop_hooks, run_user_prompt_submit_hooks};
use crate::native::tools::{
    execute_tool_call, read_only_tool_names, tool_contracts, tool_specs, LocalWorkspace,
    ToolContract, ToolCtx, ToolOutput,
};

use super::background::BackgroundTaskRegistry;
use super::compact::{
    compact_local, compact_with_summary, compaction_prompt_with_instructions,
    is_context_overflow_error, is_usable_compaction_summary, microcompact, reset_local,
    BudgetSnapshot, ChildQuota, CompactBoundary, CompactTrigger, ContextWindow, RolloutBudget,
};
use super::subagent::{
    child_system_prompt, custom_child_system_prompt, format_subagent_log_tag,
    format_subagent_result, parse_subagent_args_with, truncate_report, SubagentKind, SubagentSpec,
};
use super::truncate::{
    chars_to_tokens, context_usage_breakdown, message_tokens, total_message_tokens,
    total_tool_tokens, truncate_messages_tokens, truncate_tool_result,
    DEFAULT_TOOL_RESULT_TOKEN_LIMIT,
};
const DEFAULT_CONTEXT_CHARS: usize = 120_000;
/// A finite default prevents a runaway rollout when older settings files do
/// not have a budget field. `0` remains available for an explicit unlimited
/// setting through [`RolloutBudget`].
pub const DEFAULT_ROLLOUT_TOKEN_BUDGET: u64 =
    crate::native::settings::DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET as u64;
const DEFAULT_FINAL_OUTPUT_RESERVE: u64 = 1_024;
/// When the caller does not configure `max_output_tokens`, assume one
/// response stays within this bound. A finite rollout budget always sends
/// this (or the remaining budget, whichever is smaller) as a hard request cap.
const FALLBACK_OUTPUT_TOKEN_GUARD: u64 = 16_384;
const REPEAT_TOOL_LIMIT: u32 = 3;
/// 同一轮里并行执行只读工具的上限。
const MAX_PARALLEL_TOOL_CALLS: usize = 8;
/// stop 钩子在一个用户回合内最多要求继续的次数，防止死循环。
const MAX_STOP_HOOK_CONTINUES: u32 = 3;
const LAST_TURN_REMINDER: &str = "工具轮次已达上限。请立即给出最终结论，不要再调用工具。";
const LAST_TURN_FALLBACK: &str = "已达到最大工具轮次，已根据已有工具结果停止。";
const TOOL_RESULT_DISPLAY_MAX_LINES: usize = 2000;
const TOOL_RESULT_DISPLAY_MAX_CHARS: usize = 65_536;

#[derive(Clone)]
struct ModelTurnCfg {
    model: String,
    effort: Option<String>,
    max_output_tokens: Option<u32>,
    thinking_enabled: bool,
}

struct ModelCallBudget {
    max_output_tokens: Option<u32>,
}

type SubagentStub = Arc<dyn Fn(&SubagentSpec) -> String + Send + Sync>;
pub(crate) type CustomSubagentReloader = Arc<dyn Fn() -> Vec<NativeSubagent> + Send + Sync>;
pub(crate) type ChildModelLoader = Arc<
    dyn Fn(
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<ChildModelSettings, String>> + Send>>
        + Send
        + Sync,
>;
pub(crate) type TranscriptCheckpoint =
    Arc<dyn Fn(Vec<Message>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Debug, Default)]
pub struct AgentDiagnostics {
    tool_results_truncated: AtomicU64,
    subagents_started: AtomicU64,
    budget_stops: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentDiagnosticsSnapshot {
    pub tool_results_truncated: u64,
    pub subagents_started: u64,
    pub budget_stops: u64,
}

impl AgentDiagnostics {
    fn snapshot(&self) -> AgentDiagnosticsSnapshot {
        AgentDiagnosticsSnapshot {
            tool_results_truncated: self.tool_results_truncated.load(Ordering::Acquire),
            subagents_started: self.subagents_started.load(Ordering::Acquire),
            budget_stops: self.budget_stops.load(Ordering::Acquire),
        }
    }
}

pub struct AgentRunner {
    pub ctx: ToolCtx,
    pub messages: Vec<Message>,
    pub max_turns: u32,
    pub max_subagent_turns: u32,
    pub max_concurrent_subagents: u32,
    pub subagent_policy: String,
    pub context_char_limit: usize,
    /// Shared by the parent rollout and all child agents. A zero limit means
    /// unlimited for backwards-compatible programmatic callers.
    pub rollout_budget: Arc<RolloutBudget>,
    pub child_quota: Option<Arc<ChildQuota>>,
    pub subagent_budget_share_percent: u32,
    pub steer_rx: Option<Arc<Mutex<mpsc::Receiver<NativeFollowup>>>>,
    pub context_window: ContextWindow,
    pub tool_result_token_limit: usize,
    pub diagnostics: Arc<AgentDiagnostics>,
    pub on_event: Option<mpsc::UnboundedSender<NativeEvent>>,
    pub on_usage: Option<mpsc::UnboundedSender<UsageDelta>>,
    pub on_activity: Option<mpsc::UnboundedSender<(String, String)>>,
    pub on_checkpoint: Option<TranscriptCheckpoint>,
    pub subagent_stub: Option<SubagentStub>,
    pub custom_subagents: Vec<NativeSubagent>,
    pub reload_custom_subagents: Option<CustomSubagentReloader>,
    pub child_model_loader: Option<ChildModelLoader>,
    pub workspace_context: String,
    pub project_agents: String,
    pub required_subagent_type: Option<String>,
    extra_tools: Vec<ToolSpec>,
    /// MCP 等动态工具的契约；内置工具契约来自 catalog。
    extra_tool_contracts: Vec<ToolContract>,
    /// 大输出落盘用的 artifact 存储；父子 Agent 共享同一会话目录。
    pub artifacts: Option<Arc<ArtifactStore>>,
    /// 渠道配置的轻量模型：压缩摘要等内部调用优先使用。
    pub lite_model: Option<String>,
    /// 后台子 Agent 任务注册表（只有主 Agent 使用）。
    pub background: Arc<BackgroundTaskRegistry>,
    pub skills_prompt: String,
    last_usage: Option<Usage>,
    allowed_tools: Option<HashSet<String>>,
    disallowed_tools: Option<HashSet<String>>,
    turns: u32,
    last_tool_key: Option<String>,
    last_tool_repeat: u32,
    stop_hook_continues: u32,
    /// `/compact [指令]` 请求，下一次模型调用前执行。
    pending_manual_compact: Option<Option<String>>,
    /// 下一条用户消息末尾要附加的文本（记忆回忆等），用后即清。
    turn_suffix: Option<String>,
    /// 会话恢复到更小窗口的模型时置位，超阈值即以 downshift 触发压缩。
    pending_downshift_compact: bool,
    /// 全量压缩前先尝试微压缩（替换旧工具结果）。
    microcompact_enabled: bool,
    /// 本回合内因供应商溢出而做的被动压缩次数（上限 2）。
    reactive_compactions: u32,
    depth: u8,
    event_prefix: String,
    subagent_seq: u32,
    model_turn: Option<ModelTurnCfg>,
    pending_budget_reservation: u64,
    pending_child_reservation: u64,
    pending_steer_finish: bool,
    budget_exhausted: bool,
    streaming: bool,
    call_started_ms: Arc<AtomicU64>,
    reasoning_started_ms: Arc<AtomicU64>,
    started_tool_ids: HashSet<String>,
    tool_started_ms: HashMap<String, u64>,
    tool_seq: u32,
}

enum TurnControl {
    Continue,
    Stop(String),
}

/// Terminal output of one native run. Lines are complete and get persisted as
/// session events; deltas are live-only fragments of the answer being
/// generated. Both share one channel so the frontend always sees a fragment
/// cleared before the matching line lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextUsageSnapshot {
    pub used_tokens: usize,
    pub limit_tokens: usize,
    pub generation: u32,
    pub compactions: u32,
    pub mcp_tokens: usize,
    pub system_tool_tokens: usize,
    pub skill_tokens: usize,
    pub system_prompt_tokens: usize,
    pub other_tokens: usize,
    pub message_tokens: usize,
    pub prompt_tokens: usize,
    pub cached_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeEvent {
    Line(String),
    Delta(StreamDelta),
    ContextUsage(ContextUsageSnapshot),
    Tool {
        line: String,
        event: NativeToolEvent,
        images: Vec<NativeImage>,
    },
}

impl AgentRunner {
    pub fn new(workspace: LocalWorkspace) -> Self {
        let background = Arc::new(BackgroundTaskRegistry::new());
        let mut ctx = ToolCtx::new(workspace);
        ctx.background = Some(background.clone());
        Self {
            ctx,
            background,
            messages: Vec::new(),
            max_turns: DEFAULT_NATIVE_MAX_TURNS as u32,
            max_subagent_turns: crate::native::settings::DEFAULT_NATIVE_MAX_SUBAGENT_TURNS as u32,
            max_concurrent_subagents: crate::native::agent::subagent::MAX_CONCURRENT_SUBAGENTS
                as u32,
            subagent_policy: crate::native::settings::DEFAULT_NATIVE_SUBAGENT_POLICY.to_string(),
            context_char_limit: DEFAULT_CONTEXT_CHARS,
            rollout_budget: RolloutBudget::shared(DEFAULT_ROLLOUT_TOKEN_BUDGET),
            child_quota: None,
            subagent_budget_share_percent:
                crate::native::settings::DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT as u32,
            steer_rx: None,
            context_window: ContextWindow::new(
                crate::native::settings::DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as usize,
            ),
            tool_result_token_limit: DEFAULT_TOOL_RESULT_TOKEN_LIMIT,
            diagnostics: Arc::new(AgentDiagnostics::default()),
            on_event: None,
            on_usage: None,
            on_activity: None,
            on_checkpoint: None,
            subagent_stub: None,
            custom_subagents: Vec::new(),
            reload_custom_subagents: None,
            child_model_loader: None,
            workspace_context: String::new(),
            project_agents: String::new(),
            required_subagent_type: None,
            extra_tools: Vec::new(),
            extra_tool_contracts: Vec::new(),
            artifacts: None,
            lite_model: None,
            skills_prompt: String::new(),
            last_usage: None,
            allowed_tools: None,
            disallowed_tools: None,
            turns: 0,
            last_tool_key: None,
            last_tool_repeat: 0,
            stop_hook_continues: 0,
            pending_manual_compact: None,
            turn_suffix: None,
            pending_downshift_compact: false,
            microcompact_enabled: true,
            reactive_compactions: 0,
            depth: 0,
            event_prefix: String::new(),
            subagent_seq: 0,
            model_turn: None,
            pending_budget_reservation: 0,
            pending_child_reservation: 0,
            pending_steer_finish: false,
            budget_exhausted: false,
            streaming: false,
            call_started_ms: Arc::new(AtomicU64::new(0)),
            reasoning_started_ms: Arc::new(AtomicU64::new(0)),
            started_tool_ids: HashSet::new(),
            tool_started_ms: HashMap::new(),
            tool_seq: 0,
        }
    }

    pub fn cancel(&self) {
        self.ctx.cancel.cancel();
    }

    pub fn set_extra_tools(&mut self, tools: Vec<ToolSpec>) {
        self.extra_tools = tools;
    }

    /// 注册动态工具（MCP）的契约，用于并行判定与结果预算。
    pub fn set_extra_tool_contracts(&mut self, contracts: Vec<ToolContract>) {
        self.extra_tool_contracts = contracts;
    }

    pub fn set_artifact_store(&mut self, store: Arc<ArtifactStore>) {
        self.artifacts = Some(store);
    }

    pub fn set_allowed_tools<S: AsRef<str>>(&mut self, names: &[S]) {
        self.allowed_tools = Some(names.iter().map(|name| name.as_ref().to_string()).collect());
    }

    /// 子 Agent 档案的 `disallowedTools`：从可见工具里剔除。
    pub fn set_disallowed_tools<S: AsRef<str>>(&mut self, names: &[S]) {
        self.disallowed_tools = Some(names.iter().map(|name| name.as_ref().to_string()).collect());
    }

    fn contract_registry(&self) -> ContractRegistry {
        let mut registry = ContractRegistry::new(tool_contracts());
        registry.extend(self.extra_tool_contracts.iter().cloned());
        registry
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.ctx.set_read_only(read_only);
    }

    pub fn set_plan_mode(&mut self, plan_mode: bool) {
        self.ctx.set_plan_mode(plan_mode);
    }

    /// 模型可能已通过 ExitPlanMode 结束计划模式，会话层据此决定是否还要自动实施。
    pub fn is_plan_mode(&self) -> bool {
        self.ctx.is_plan_mode()
    }

    pub fn take_steer_finish(&mut self) -> bool {
        std::mem::take(&mut self.pending_steer_finish)
    }

    fn inject_steer_messages(&mut self) -> bool {
        let Some(rx) = self.steer_rx.clone() else {
            return false;
        };
        let Ok(mut guard) = rx.try_lock() else {
            return false;
        };
        let mut injected = false;
        while let Ok(item) = guard.try_recv() {
            match item {
                NativeFollowup::Input(text) => {
                    self.emit(format!("[USER_INPUT] {text}"));
                    self.messages.push(Message::user(text));
                    injected = true;
                }
                NativeFollowup::Compact(instructions) => {
                    self.pending_manual_compact = Some(instructions);
                }
                NativeFollowup::Finish => {
                    self.pending_steer_finish = true;
                }
            }
        }
        injected
    }

    /// `/compact [指令]`：下一次模型调用前压缩。
    pub fn request_manual_compaction(&mut self, instructions: Option<String>) {
        self.pending_manual_compact = Some(instructions);
    }

    /// 给下一条用户消息附加文本（记忆回忆）；不影响事件流里的 `[USER_INPUT]` 行。
    pub fn set_turn_suffix(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.turn_suffix = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
    }

    /// 会话恢复到更小窗口模型时调用；只有超阈值才真正压缩。
    pub fn request_downshift_compaction(&mut self) {
        self.pending_downshift_compact = true;
    }

    pub fn set_compaction_options(&mut self, threshold_percent: u32, microcompact_enabled: bool) {
        self.context_window
            .set_threshold_percent(threshold_percent as usize);
        self.microcompact_enabled = microcompact_enabled;
    }

    /// 等待输入状态下的 `/compact`：立刻压缩并写边界记录。
    pub async fn compact_now(
        &mut self,
        client: &ModelClient,
        instructions: Option<String>,
    ) -> Option<CompactBoundary> {
        let client = self.observe_client(client);
        self.run_compaction(Some(&client), CompactTrigger::Manual, instructions)
            .await
    }

    /// 统一的压缩入口：微压缩 → 模型摘要 → 本地摘要 → 重置。返回边界记录（已写入事件流）。
    async fn run_compaction(
        &mut self,
        client: Option<&ModelClient>,
        trigger: CompactTrigger,
        instructions: Option<String>,
    ) -> Option<CompactBoundary> {
        let pre_tokens = total_message_tokens(&self.messages);
        let pre_messages = self.messages.len();
        let mut source: Option<&str> = None;
        // 自动 / 降级 / 被动触发时，先试便宜的微压缩；用户明确 /compact 时直接做摘要。
        if trigger != CompactTrigger::Manual && self.microcompact_enabled {
            let replaced = microcompact(&mut self.messages);
            if replaced > 0 && !self.context_window.should_compact(&self.messages) {
                source = Some("microcompact");
            }
        }
        if source.is_none() {
            let mut compacted = false;
            if let Some(client) = client {
                compacted = self
                    .compact_with_model(client, instructions.as_deref())
                    .await;
                if compacted {
                    source = Some("model");
                }
            }
            if !compacted && compact_local(&mut self.messages) {
                source = Some("local");
                compacted = true;
            }
            if !compacted && reset_local(&mut self.messages) {
                source = Some("reset");
                compacted = true;
            }
            if !compacted {
                return None;
            }
        }
        let source = source.unwrap_or("local");
        if source == "reset" {
            self.context_window.mark_reset();
        } else {
            self.context_window.mark_compacted();
        }
        let boundary = CompactBoundary {
            trigger,
            source: source.to_string(),
            pre_tokens,
            post_tokens: total_message_tokens(&self.messages),
            pre_messages,
            post_messages: self.messages.len(),
            instructions: instructions.filter(|item| !item.trim().is_empty()),
        };
        let label = match source {
            "microcompact" => "已微压缩旧工具结果",
            "model" => "已压缩上下文（模型摘要）",
            "local" => "已压缩上下文（本地摘要）",
            _ => "已重置上下文窗口（保留当前任务）",
        };
        self.emit(format!(
            "[工具] {label}：{} → {} token（{}）",
            boundary.pre_tokens,
            boundary.post_tokens,
            boundary.trigger.as_str()
        ));
        self.emit(boundary.line());
        self.emit_context_usage();
        self.checkpoint_transcript().await;
        Some(boundary)
    }

    async fn checkpoint_transcript(&self) {
        if self.depth > 0 {
            return;
        }
        let Some(hook) = &self.on_checkpoint else {
            return;
        };
        hook(self.messages.clone()).await;
    }

    pub fn set_rollout_budget(&mut self, budget: Arc<RolloutBudget>) {
        self.release_model_reservation();
        self.rollout_budget = budget;
        self.budget_exhausted = false;
    }

    pub fn set_rollout_budget_limit(&mut self, limit: u64) {
        self.release_model_reservation();
        self.rollout_budget = RolloutBudget::shared(limit);
        self.budget_exhausted = false;
    }

    pub fn budget_snapshot(&self) -> BudgetSnapshot {
        self.rollout_budget.snapshot()
    }

    pub fn diagnostics_snapshot(&self) -> AgentDiagnosticsSnapshot {
        self.diagnostics.snapshot()
    }

    fn combined_tools(&self) -> Vec<ToolSpec> {
        let mut tools = tool_specs();
        let read_only = self.ctx.is_read_only();
        let plan_mode = self.ctx.is_plan_mode();
        if self.depth > 0 || read_only {
            tools.retain(|tool| tool.name != "Agent");
        }
        // 子 Agent 没有用户交互通道：不给提问与计划模式工具；后台任务管理只给主 Agent。
        if self.depth > 0 {
            tools.retain(|tool| {
                !matches!(
                    tool.name.as_str(),
                    "AskUserQuestion"
                        | "EnterPlanMode"
                        | "ExitPlanMode"
                        | "TaskOutput"
                        | "TaskStop"
                        | "SendMessage"
                        | "CronCreate"
                        | "CronList"
                        | "CronDelete"
                        | "Goal"
                        | "GoalRead"
                )
            });
        }
        // RespondToCoordinator 只对后台子 Agent 可见。
        let is_background_child = self.ctx.coordinator.is_some();
        tools.retain(|tool| tool.name != "RespondToCoordinator" || is_background_child);
        if read_only {
            tools.retain(|tool| {
                !matches!(
                    tool.name.as_str(),
                    "TaskOutput" | "TaskStop" | "SendMessage"
                )
            });
        }
        // EnterPlanMode 只在执行模式可见，ExitPlanMode 只在计划模式可见。
        tools.retain(|tool| match tool.name.as_str() {
            "EnterPlanMode" => !plan_mode && !read_only,
            "ExitPlanMode" => plan_mode,
            _ => true,
        });
        let cap = self.max_concurrent_subagents.max(1);
        for tool in &mut tools {
            if tool.name == "Agent" {
                tool.description = crate::native::prompt::agent_tool_description(
                    cap,
                    &self.subagent_policy,
                    &self.custom_subagents,
                    self.required_subagent_type.as_deref(),
                );
            }
        }
        tools.extend(self.extra_tools.clone());
        if let Some(allowed) = &self.allowed_tools {
            tools.retain(|tool| allowed.contains(&tool.name));
        }
        if let Some(disallowed) = &self.disallowed_tools {
            tools.retain(|tool| !disallowed.contains(&tool.name));
        }
        if read_only {
            tools.retain(|tool| crate::native::tools::is_read_only_native_tool(&tool.name));
        }
        tools
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.combined_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    fn emit(&self, line: impl Into<String>) {
        Self::send_prefixed_event(&self.on_event, &self.event_prefix, line);
    }

    fn send_prefixed_event(
        on_event: &Option<mpsc::UnboundedSender<NativeEvent>>,
        prefix: &str,
        line: impl Into<String>,
    ) {
        if let Some(tx) = on_event {
            let line = line.into();
            let _ = tx.send(NativeEvent::Line(if prefix.is_empty() {
                line
            } else {
                format!("{prefix}{line}")
            }));
        }
    }

    fn subagent_tag(&self) -> Option<String> {
        let tag = self.event_prefix.trim();
        if tag.is_empty() {
            None
        } else {
            Some(tag.to_string())
        }
    }

    fn assign_call_id(&mut self, call: &mut ToolCall) {
        if call.id.trim().is_empty() {
            self.tool_seq = self.tool_seq.saturating_add(1);
            call.id = format!("anon-{}", self.tool_seq);
        }
    }

    fn send_tool(&self, line: String, event: NativeToolEvent, images: Vec<NativeImage>) {
        if let Some(tx) = &self.on_event {
            let line = if self.event_prefix.is_empty() {
                line
            } else {
                format!("{}{line}", self.event_prefix)
            };
            let _ = tx.send(NativeEvent::Tool {
                line,
                event,
                images,
            });
        }
    }

    async fn mcp_display(&self, name: &str) -> (Option<String>, Option<String>) {
        match self.ctx.mcp.display_for_tool(name).await {
            Some((server, tool)) => (Some(server), Some(tool)),
            None => (None, None),
        }
    }

    async fn emit_tool_start(&mut self, call: &ToolCall) {
        if self.started_tool_ids.contains(&call.id) {
            return;
        }
        let (mcp_server, mcp_tool) = self.mcp_display(&call.name).await;
        let line = tool_start_line_ex(
            &call.name,
            &call.arguments,
            mcp_server.as_deref(),
            mcp_tool.as_deref(),
        );
        let title = tool_event_title(&line);
        let args_summary = tool_args_summary(&call.name, &call.arguments);
        self.started_tool_ids.insert(call.id.clone());
        self.tool_started_ms.insert(call.id.clone(), unix_now_ms());
        self.send_tool(
            line,
            NativeToolEvent {
                phase: NativeToolPhase::Start,
                call_id: call.id.clone(),
                name: call.name.clone(),
                title,
                args_summary,
                ok: None,
                duration_ms: None,
                result_preview: None,
                subagent_tag: self.subagent_tag(),
                mcp_server,
                mcp_tool,
                image_names: Vec::new(),
            },
            Vec::new(),
        );
    }

    async fn emit_tool_result(&mut self, call: &ToolCall, output: &ToolOutput) {
        if !self.started_tool_ids.contains(&call.id) {
            self.emit_tool_start(call).await;
        }
        let duration_ms = self
            .tool_started_ms
            .get(&call.id)
            .map(|start| unix_now_ms().saturating_sub(*start));
        let (mcp_server, mcp_tool) = self.mcp_display(&call.name).await;
        let start_line = tool_start_line_ex(
            &call.name,
            &call.arguments,
            mcp_server.as_deref(),
            mcp_tool.as_deref(),
        );
        let line = tool_result_line(&call.name, &output.text);
        self.send_tool(
            line,
            NativeToolEvent {
                phase: NativeToolPhase::Result,
                call_id: call.id.clone(),
                name: call.name.clone(),
                title: tool_event_title(&start_line),
                args_summary: tool_args_summary(&call.name, &call.arguments),
                ok: Some(output.ok),
                duration_ms,
                result_preview: Some(cap_tool_result_display(&output.text)),
                subagent_tag: self.subagent_tag(),
                mcp_server,
                mcp_tool,
                image_names: output
                    .images
                    .iter()
                    .map(|image| image.name.clone())
                    .collect(),
            },
            output.images.clone(),
        );
    }

    /// Drop the fragments shown so far: either the complete line is about to
    /// arrive, or a retry is going to regenerate the answer.
    fn emit_delta_clear(&self) {
        if !self.streaming {
            return;
        }
        if let Some(tx) = &self.on_event {
            let _ = tx.send(NativeEvent::Delta(StreamDelta::Reset));
        }
    }

    fn begin_model_call(&self) {
        self.call_started_ms.store(unix_now_ms(), Ordering::Relaxed);
        self.reasoning_started_ms.store(0, Ordering::Relaxed);
    }

    fn thinking_elapsed_seconds(&self) -> u32 {
        let now = unix_now_ms();
        let reasoning = self.reasoning_started_ms.swap(0, Ordering::Relaxed);
        let call = self.call_started_ms.swap(0, Ordering::Relaxed);
        let start = if reasoning > 0 { reasoning } else { call };
        thinking_duration_seconds(now.saturating_sub(start))
    }

    fn observe_client(&self, client: &ModelClient) -> ModelClient {
        let on_event = self.on_event.clone();
        let prefix = self.event_prefix.clone();
        let delta_events = self.on_event.clone();
        let reasoning_started_ms = self.reasoning_started_ms.clone();
        client
            .clone_for_conversation()
            .with_cancel(self.ctx.cancel.clone())
            .with_retry_hook(Arc::new(move |line: &str| {
                Self::send_prefixed_event(&on_event, &prefix, line);
            }))
            // Only the top-level runner streams: concurrent child agents would
            // interleave their fragments into one unreadable line.
            .with_delta_hook(Arc::new(move |delta: StreamDelta| {
                if matches!(&delta, StreamDelta::Reasoning(text) if !text.is_empty()) {
                    let _ = reasoning_started_ms.compare_exchange(
                        0,
                        unix_now_ms(),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
                if let Some(tx) = &delta_events {
                    let _ = tx.send(NativeEvent::Delta(delta));
                }
            }))
    }

    fn observe_child_client(
        &self,
        parent: Option<&ModelClient>,
        client: &ModelClient,
        spec: Option<&SubagentSpec>,
    ) -> ModelClient {
        let on_event = self.on_event.clone();
        let prefix = self.event_prefix.clone();
        // Child agents must begin an independent Responses conversation even
        // when they inherit the parent's transport client.
        let mut observed = client
            .clone()
            .with_cancel(self.ctx.cancel.clone())
            .with_retry_hook(Arc::new(move |line: &str| {
                Self::send_prefixed_event(&on_event, &prefix, line);
            }));
        let parent_context = parent.and_then(ModelClient::call_log_context);
        let mut context = client
            .call_log_context()
            .cloned()
            .or_else(|| parent_context.cloned())
            .unwrap_or_default()
            .with_call_kind(CALL_KIND_SUBAGENT)
            .with_operation(OPERATION_SUBAGENT);
        if context.session_id.is_none() {
            context.session_id = parent_context.and_then(|item| item.session_id.clone());
        }
        if context.profile_id.is_none() {
            context.profile_id = parent_context.and_then(|item| item.profile_id.clone());
        }
        if context.workspace_id.is_none() {
            context.workspace_id = parent_context.and_then(|item| item.workspace_id.clone());
        }
        if context.execution_target.is_none() {
            context.execution_target =
                parent_context.and_then(|item| item.execution_target.clone());
        }
        if let Some(spec) = spec {
            context = context.with_subagent_id(spec.kind.as_str());
        }
        if let Some(sink) = client
            .call_log_sink()
            .or_else(|| parent.and_then(ModelClient::call_log_sink))
        {
            observed = observed.with_call_log(context, sink);
        } else {
            observed = observed.with_call_log_context(context);
        }
        observed
    }

    fn emit_activity(&self, action: &str, details: &str) {
        if let Some(tx) = &self.on_activity {
            let _ = tx.send((action.to_string(), details.to_string()));
        }
    }

    fn emit_usage(&mut self, usage: crate::native::model::types::Usage) {
        let Some(delta) = usage_to_delta(usage) else {
            return;
        };
        if let Some(line) = delta.format_terminal_line() {
            self.emit(line);
        }
        if let Some(tx) = &self.on_usage {
            let _ = tx.send(delta);
        }
        self.last_usage = Some(usage);
        self.emit_context_usage();
    }

    fn settle_model_usage(&mut self, usage: Usage, assistant: Option<&Message>) {
        let reserved = std::mem::take(&mut self.pending_budget_reservation);
        let reported =
            u64::from(usage.prompt_tokens).saturating_add(u64::from(usage.completion_tokens));
        let estimated = assistant
            .map(|message| message_tokens(message) as u64)
            .unwrap_or(0);
        let actual = if reported > 0 {
            reported
        } else {
            estimated.max(reserved)
        };
        let actual_opt = (actual > 0).then_some(actual);
        if reserved > 0 {
            self.rollout_budget.settle(reserved, actual_opt);
        } else {
            self.rollout_budget.record_usage(usage);
        }
        self.settle_child_reservation(if reserved > 0 { actual_opt } else { None });
        self.budget_exhausted = self.rollout_budget.is_exhausted();
    }

    fn release_model_reservation(&mut self) {
        let reserved = std::mem::take(&mut self.pending_budget_reservation);
        if reserved > 0 {
            self.rollout_budget.release(reserved);
        }
        self.release_child_reservation();
    }

    fn try_reserve_child(&mut self, tokens: u64) -> bool {
        let Some(quota) = &self.child_quota else {
            return true;
        };
        if quota.try_reserve(tokens) {
            self.pending_child_reservation = tokens;
            true
        } else {
            false
        }
    }

    fn settle_child_reservation(&mut self, actual: Option<u64>) {
        let reserved = std::mem::take(&mut self.pending_child_reservation);
        if reserved == 0 {
            return;
        }
        if let Some(quota) = &self.child_quota {
            quota.settle(reserved, actual);
        }
    }

    fn release_child_reservation(&mut self) {
        let reserved = std::mem::take(&mut self.pending_child_reservation);
        if reserved == 0 {
            return;
        }
        if let Some(quota) = &self.child_quota {
            quota.release(reserved);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_client(
        &mut self,
        client: &ModelClient,
        user: &str,
        model: &str,
        effort: Option<&str>,
        max_output_tokens: Option<u32>,
        thinking_enabled: bool,
        images: Vec<NativeImage>,
    ) -> Result<String, String> {
        self.model_turn = Some(ModelTurnCfg {
            model: model.to_string(),
            effort: effort.map(ToOwned::to_owned),
            max_output_tokens,
            thinking_enabled,
        });
        self.begin_user_turn(user, images).await?;
        self.checkpoint_transcript().await;
        let client = self.observe_client(client);
        self.streaming = true;
        loop {
            if self.inject_steer_messages() {
                self.checkpoint_transcript().await;
            }
            let mut last_turn = self.prepare_model_call(Some(&client)).await?;
            if self.pending_steer_finish {
                last_turn = true;
            }
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            let call_budget =
                if let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) {
                    budget
                } else {
                    // A request with tools may not fit the remaining shared
                    // budget. Retry once as a tool-free final answer, then stop
                    // locally if even that request cannot be reserved.
                    if !last_turn {
                        last_turn = true;
                        self.append_budget_reminder();
                    }
                    tools_now = &[];
                    let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) else {
                        return self.finish_without_model();
                    };
                    budget
                };
            self.begin_model_call();
            let result = client
                .chat(ChatRequest {
                    messages: &self.messages,
                    tools: tools_now,
                    model,
                    effort,
                    max_output_tokens: call_budget.max_output_tokens,
                    thinking_enabled,
                })
                .await;
            let (assistant, usage) = match result {
                Ok(value) => value,
                Err(error) => {
                    self.release_model_reservation();
                    self.emit_delta_clear();
                    if self.try_reactive_compaction(&client, &error).await {
                        continue;
                    }
                    return Err(error);
                }
            };
            let requested_with_tools = !tools_now.is_empty();
            self.settle_model_usage(usage, Some(&assistant));
            self.emit_usage(usage);
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self
                .consume_assistant(assistant, last_turn, Some(&client))
                .await?
            {
                TurnControl::Stop(text) => return Ok(text),
                TurnControl::Continue => {}
            }
        }
    }

    /// 供应商报上下文溢出时被动压缩再重试；连续两次仍溢出则放弃。
    async fn try_reactive_compaction(&mut self, client: &ModelClient, error: &str) -> bool {
        if !is_context_overflow_error(error) || self.reactive_compactions >= 2 {
            return false;
        }
        self.reactive_compactions += 1;
        self.emit(format!(
            "[工具] 模型报上下文溢出，尝试被动压缩后重试（{}/2）：{error}",
            self.reactive_compactions
        ));
        // 被动压缩不再依赖阈值判断，直接做全量摘要。
        let boundary = self
            .run_compaction(Some(client), CompactTrigger::Reactive, None)
            .await;
        if boundary.is_none() {
            self.emit("[工具] 被动压缩无法再缩减上下文，停止重试");
        }
        boundary.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_child_with_client(
        &mut self,
        parent: Option<&ModelClient>,
        client: &ModelClient,
        user: &str,
        model: &str,
        effort: Option<&str>,
        max_output_tokens: Option<u32>,
        thinking_enabled: bool,
        spec: Option<&SubagentSpec>,
    ) -> Result<String, String> {
        self.model_turn = Some(ModelTurnCfg {
            model: model.to_string(),
            effort: effort.map(ToOwned::to_owned),
            max_output_tokens,
            thinking_enabled,
        });
        self.begin_user_turn(user, Vec::new()).await?;
        self.checkpoint_transcript().await;
        let client = self.observe_child_client(parent, client, spec);
        loop {
            let mut last_turn = self.prepare_model_call(Some(&client)).await?;
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            let call_budget =
                if let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) {
                    budget
                } else {
                    if !last_turn {
                        last_turn = true;
                        self.append_budget_reminder();
                    }
                    tools_now = &[];
                    let Some(budget) = self.reserve_model_call(max_output_tokens, tools_now) else {
                        return self.finish_without_model();
                    };
                    budget
                };
            self.begin_model_call();
            let result = client
                .chat(ChatRequest {
                    messages: &self.messages,
                    tools: tools_now,
                    model,
                    effort,
                    max_output_tokens: call_budget.max_output_tokens,
                    thinking_enabled,
                })
                .await;
            let (assistant, usage) = match result {
                Ok(value) => value,
                Err(error) => {
                    self.release_model_reservation();
                    if self.try_reactive_compaction(&client, &error).await {
                        continue;
                    }
                    return Err(error);
                }
            };
            let requested_with_tools = !tools_now.is_empty();
            self.settle_model_usage(usage, Some(&assistant));
            self.emit_usage(usage);
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self.consume_assistant_serial(assistant, last_turn).await? {
                TurnControl::Stop(text) => return Ok(text),
                TurnControl::Continue => {}
            }
        }
    }

    pub async fn run_scripted(
        &mut self,
        user: &str,
        replies: Vec<Message>,
    ) -> Result<String, String> {
        self.begin_user_turn(user, Vec::new()).await?;
        self.checkpoint_transcript().await;
        let mut queue = VecDeque::from(replies);
        loop {
            let mut last_turn = self.prepare_model_call(None).await?;
            let tools = self.combined_tools();
            let mut tools_now: &[ToolSpec] = if last_turn { &[] } else { &tools };
            if self.reserve_model_call(None, tools_now).is_none() {
                if !last_turn {
                    last_turn = true;
                    self.append_budget_reminder();
                    tools_now = &[];
                    if self.reserve_model_call(None, tools_now).is_none() {
                        return self.finish_without_model();
                    }
                } else {
                    return self.finish_without_model();
                }
            }
            let requested_with_tools = !tools_now.is_empty();
            let Some(assistant) = queue.pop_front() else {
                self.release_model_reservation();
                return Err("scripted model exhausted".to_string());
            };
            self.settle_model_usage(Usage::default(), Some(&assistant));
            last_turn = last_turn_after_response(
                last_turn,
                requested_with_tools,
                &assistant,
                self.budget_exhausted,
            );
            match self.consume_assistant(assistant, last_turn, None).await? {
                TurnControl::Stop(text) => return Ok(text),
                TurnControl::Continue => {}
            }
        }
    }

    async fn begin_user_turn(
        &mut self,
        user: &str,
        images: Vec<NativeImage>,
    ) -> Result<(), String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        self.turns = 0;
        self.last_tool_key = None;
        self.last_tool_repeat = 0;
        self.stop_hook_continues = 0;
        self.reactive_compactions = 0;
        let mut text = user.to_string();
        if let Some(suffix) = self.turn_suffix.take() {
            text = format!("{text}\n\n{suffix}");
        }
        if self.depth == 0 && !self.ctx.hooks.is_empty() {
            match run_user_prompt_submit_hooks(&self.ctx.hook_runtime(), user).await {
                Ok(context) if !context.is_empty() => {
                    self.emit("[钩子] user_prompt_submit 注入了附加上下文");
                    text = format!("{text}\n\n[钩子上下文]\n{}", context.join("\n"));
                }
                Ok(_) => {}
                Err(reason) => return Err(format!("输入被钩子阻断：{reason}")),
            }
        }
        self.messages.push(Message::user_with_images(text, images));
        Ok(())
    }

    /// 回合结束前询问 stop 钩子；要求继续时把理由作为用户消息追加，最多 3 次。
    async fn stop_hooks_want_continue(&mut self, final_text: &str) -> bool {
        if self.depth > 0 || self.ctx.hooks.is_empty() {
            return false;
        }
        if self.stop_hook_continues >= MAX_STOP_HOOK_CONTINUES {
            return false;
        }
        let result = run_stop_hooks(&self.ctx.hook_runtime(), final_text).await;
        for warning in &result.warnings {
            self.emit(format!("[钩子警告] {warning}"));
        }
        let Some(reason) = result.continue_reason else {
            return false;
        };
        self.stop_hook_continues += 1;
        self.emit(format!(
            "[钩子] stop 钩子要求继续（{}/{}）：{reason}",
            self.stop_hook_continues, MAX_STOP_HOOK_CONTINUES
        ));
        self.messages
            .push(Message::user(format!("[Stop 钩子要求继续] {reason}")));
        true
    }

    async fn prepare_model_call(&mut self, client: Option<&ModelClient>) -> Result<bool, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        if self.max_turns > 0 && self.turns >= self.max_turns {
            return Err("达到最大模型轮次".to_string());
        }
        self.turns += 1;
        self.sync_context_window();
        // 后台任务完成 / 子 Agent 留言：在模型看到下一次请求前提醒。
        if self.depth == 0 {
            if let Some(notice) = self.background.pending_notice() {
                self.emit(notice.clone());
                append_user_note(&mut self.messages, &notice);
            }
        }
        if let Some(instructions) = self.pending_manual_compact.take() {
            if self
                .run_compaction(client, CompactTrigger::Manual, instructions)
                .await
                .is_none()
            {
                self.emit("[工具] 当前上下文太短，无需压缩");
            }
        } else if self.context_window.should_compact(&self.messages) {
            let trigger = if std::mem::take(&mut self.pending_downshift_compact) {
                CompactTrigger::Downshift
            } else {
                CompactTrigger::Auto
            };
            self.run_compaction(client, trigger, None).await;
        } else {
            self.pending_downshift_compact = false;
        }
        let tool_context_tokens = total_tool_tokens(&self.combined_tools());
        // Tool schemas are part of the request context too. If MCP/schema
        // definitions alone consume the entire configured window, sending
        // them would guarantee an oversized request. Fall back to a final
        // tool-free turn and retain the full window for the answer.
        let tools_fit = tool_context_tokens < self.context_window.token_limit;
        let message_context_limit = if tools_fit {
            self.context_window
                .token_limit
                .saturating_sub(tool_context_tokens)
                .max(1)
        } else {
            self.context_window.token_limit.max(1)
        };
        truncate_messages_tokens(
            &mut self.messages,
            message_context_limit,
            self.tool_result_token_limit,
        );
        // A provider may return a very large assistant message after local
        // compaction. Try a second reset before sending an oversized request.
        if total_message_tokens(&self.messages) > message_context_limit
            && reset_local(&mut self.messages)
        {
            self.context_window.mark_reset();
            truncate_messages_tokens(
                &mut self.messages,
                message_context_limit,
                self.tool_result_token_limit,
            );
            self.emit("[工具] 已重置上下文窗口（超出 token 上限）");
        }
        let budget_stop = self.budget_exhausted || self.rollout_budget.is_exhausted();
        if !tools_fit {
            self.emit("[工具] 工具定义已超过上下文窗口，停止调用工具并直接作答");
        }
        if budget_stop {
            let newly_exhausted = !self.budget_exhausted;
            self.budget_exhausted = true;
            if newly_exhausted {
                self.diagnostics.budget_stops.fetch_add(1, Ordering::AcqRel);
            }
            self.emit("[工具] rollout token 预算已用尽，停止调用工具并直接作答");
        }
        let last_turn =
            !tools_fit || budget_stop || (self.max_turns > 0 && self.turns >= self.max_turns);
        if last_turn {
            if !budget_stop {
                self.emit(format!(
                    "[工具] 第 {}/{} 轮，停止调用工具并直接作答",
                    self.turns, self.max_turns
                ));
            }
            append_last_turn_reminder(&mut self.messages);
        }
        self.emit_context_usage();
        Ok(last_turn)
    }

    fn emit_context_usage(&self) {
        if self.depth != 0 {
            return;
        }
        if let Some(tx) = &self.on_event {
            let breakdown = context_usage_breakdown(
                &self.messages,
                &self.combined_tools(),
                &self.skills_prompt,
            );
            let (prompt_tokens, cached_tokens) = self
                .last_usage
                .map(|usage| (usage.prompt_tokens as usize, usage.cached_tokens as usize))
                .unwrap_or((0, 0));
            let _ = tx.send(NativeEvent::ContextUsage(ContextUsageSnapshot {
                used_tokens: breakdown.used_tokens,
                limit_tokens: self.context_window.token_limit,
                generation: self.context_window.generation,
                compactions: self.context_window.compactions,
                mcp_tokens: breakdown.mcp_tokens,
                system_tool_tokens: breakdown.system_tool_tokens,
                skill_tokens: breakdown.skill_tokens,
                system_prompt_tokens: breakdown.system_prompt_tokens,
                other_tokens: breakdown.other_tokens,
                message_tokens: breakdown.message_tokens,
                prompt_tokens,
                cached_tokens,
            }));
        }
    }

    async fn compact_with_model(
        &mut self,
        client: &ModelClient,
        instructions: Option<&str>,
    ) -> bool {
        let Some(summary_messages) =
            compaction_prompt_with_instructions(&self.messages, instructions)
        else {
            return false;
        };
        let Some(summary) = self
            .request_compaction_summary(client, &summary_messages)
            .await
        else {
            return false;
        };
        if apply_usable_summary(&mut self.messages, &summary) {
            return true;
        }
        let mut retry_messages = summary_messages;
        retry_messages.push(Message::assistant_text(summary.content.clone()));
        retry_messages.push(Message::user(
            "Previous summary is unusable. Regenerate a factual structured handoff that includes at least two of these headings: User goal, Constraints, Completed work, Verification, Pending work.",
        ));
        if let Some(retry) = self
            .request_compaction_summary(client, &retry_messages)
            .await
        {
            if apply_usable_summary(&mut self.messages, &retry) {
                return true;
            }
            if retry.content.trim().is_empty() {
                self.emit("[工具] 模型摘要为空，改用本地摘要");
            } else {
                self.emit("[工具] 模型摘要不可用，改用本地摘要");
            }
        }
        false
    }

    async fn request_compaction_summary(
        &mut self,
        client: &ModelClient,
        messages: &[Message],
    ) -> Option<Message> {
        let summary_limit = 1_024u64;
        let request_tokens = total_message_tokens(messages) as u64;
        let reservation = request_tokens.saturating_add(summary_limit);
        if !self.rollout_budget.try_reserve(reservation) {
            self.emit("[工具] 上下文压缩预算不足，改用本地摘要");
            return None;
        }
        self.pending_budget_reservation = reservation;
        // 配置了轻量模型时用它做摘要，更便宜也更快。
        let main_model = self
            .model_turn
            .as_ref()
            .map(|turn| turn.model.clone())
            .unwrap_or_default();
        let (summary_model, model_role) = match self.lite_model.as_deref() {
            Some(lite) if !lite.trim().is_empty() && lite != main_model => {
                (lite.to_string(), MODEL_ROLE_LITE)
            }
            _ => (main_model, MODEL_ROLE_MAIN),
        };
        let compact_client = match client.call_log_context() {
            Some(context) => client.clone_for_conversation().with_call_log_context(
                context
                    .clone()
                    .with_call_kind(CALL_KIND_COMPACT)
                    .with_operation(OPERATION_COMPACT)
                    .with_model_role(model_role),
            ),
            None => client.clone_for_conversation(),
        };
        let result = compact_client
            .chat(ChatRequest {
                messages,
                tools: &[],
                model: &summary_model,
                effort: None,
                max_output_tokens: Some(summary_limit as u32),
                thinking_enabled: false,
            })
            .await;
        match result {
            Ok((summary, usage)) => {
                self.settle_model_usage(usage, Some(&summary));
                self.emit_usage(usage);
                Some(summary)
            }
            Err(error) => {
                self.release_model_reservation();
                self.emit(format!("[工具] 模型摘要失败，改用本地摘要：{error}"));
                None
            }
        }
    }

    fn sync_context_window(&mut self) {
        let configured = chars_to_tokens(self.context_char_limit).max(1);
        // Older callers only set `context_char_limit`; newer session wiring
        // sets the token field explicitly. Do not overwrite an explicit token
        // window when both fields are present.
        if self.context_char_limit != DEFAULT_CONTEXT_CHARS
            && self.context_window.token_limit
                == crate::native::settings::DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as usize
        {
            self.context_window.set_token_limit(configured);
        }
    }

    fn reserve_model_call(
        &mut self,
        max_output_tokens: Option<u32>,
        tools: &[ToolSpec],
    ) -> Option<ModelCallBudget> {
        if self.pending_budget_reservation > 0 {
            self.release_model_reservation();
        }
        let input_tokens = total_message_tokens(&self.messages) as u64;
        let requested_output_tokens =
            u64::from(max_output_tokens.unwrap_or(DEFAULT_FINAL_OUTPUT_RESERVE as u32));
        let tool_tokens = total_tool_tokens(tools) as u64;
        let fixed_tokens = input_tokens.saturating_add(tool_tokens);
        if self.rollout_budget.limit() == 0 {
            // Unlimited budgets track usage for diagnostics but never rewrite
            // the caller's provider settings.
            let estimate = fixed_tokens.saturating_add(requested_output_tokens);
            if !self.try_reserve_child(estimate) {
                self.mark_budget_stop();
                return None;
            }
            self.rollout_budget.try_reserve(estimate);
            self.pending_budget_reservation = estimate;
            return Some(ModelCallBudget { max_output_tokens });
        }
        let mut available = self.rollout_budget.remaining().saturating_sub(fixed_tokens);
        if let Some(quota) = &self.child_quota {
            available = available.min(quota.remaining().saturating_sub(fixed_tokens));
        }
        if available == 0 {
            self.mark_budget_stop();
            return None;
        }
        // The output reserve is only an estimate that `settle_model_usage`
        // replaces with actual usage. A finite budget always sends a hard
        // request cap so a provider that omits usage cannot overrun remaining
        // tokens. Unlimited rollouts still leave the caller's setting alone.
        let requested_output_tokens = match max_output_tokens {
            Some(value) => u64::from(value),
            None => FALLBACK_OUTPUT_TOKEN_GUARD,
        };
        let reserve_output_tokens = requested_output_tokens.min(available);
        let request_cap = Some(reserve_output_tokens as u32);
        let estimate = fixed_tokens.saturating_add(reserve_output_tokens);
        if !self.try_reserve_child(estimate) {
            self.mark_budget_stop();
            return None;
        }
        if self.rollout_budget.try_reserve(estimate) {
            self.pending_budget_reservation = estimate;
            Some(ModelCallBudget {
                max_output_tokens: request_cap,
            })
        } else {
            self.release_child_reservation();
            self.mark_budget_stop();
            None
        }
    }

    fn mark_budget_stop(&mut self) {
        let newly_exhausted = !self.budget_exhausted;
        self.budget_exhausted = true;
        if newly_exhausted {
            self.diagnostics.budget_stops.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn append_budget_reminder(&mut self) {
        self.emit("[工具] rollout token 预算不足，当前请求仅允许直接作答");
        append_last_turn_reminder(&mut self.messages);
    }

    fn finish_without_model(&mut self) -> Result<String, String> {
        self.release_model_reservation();
        self.emit(LAST_TURN_FALLBACK.to_string());
        Ok(LAST_TURN_FALLBACK.to_string())
    }

    async fn consume_assistant(
        &mut self,
        mut assistant: Message,
        last_turn: bool,
        client: Option<&ModelClient>,
    ) -> Result<TurnControl, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        assistant
            .tool_calls
            .retain(|call| !call.name.trim().is_empty());
        if last_turn {
            assistant.tool_calls.clear();
        }
        // The persisted lines below supersede whatever streamed live.
        self.emit_delta_clear();
        if let Some(line) = thinking_start_line(
            &assistant.reasoning_content,
            self.thinking_elapsed_seconds(),
        ) {
            self.emit(line);
        }
        let text = assistant.content.clone();
        let tool_calls = assistant.tool_calls.clone();
        if !text.trim().is_empty() {
            self.emit(text.clone());
        }
        self.messages.push(assistant);
        if tool_calls.is_empty() {
            let text = if text.trim().is_empty() && last_turn {
                self.emit(LAST_TURN_FALLBACK.to_string());
                LAST_TURN_FALLBACK.to_string()
            } else {
                text
            };
            if !last_turn && self.stop_hooks_want_continue(&text).await {
                self.checkpoint_transcript().await;
                return Ok(TurnControl::Continue);
            }
            self.checkpoint_transcript().await;
            return Ok(TurnControl::Stop(text));
        }
        self.execute_tool_calls(tool_calls, client).await?;
        self.checkpoint_transcript().await;
        Ok(TurnControl::Continue)
    }

    async fn consume_assistant_serial(
        &mut self,
        mut assistant: Message,
        last_turn: bool,
    ) -> Result<TurnControl, String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        assistant
            .tool_calls
            .retain(|call| !call.name.trim().is_empty());
        if last_turn {
            assistant.tool_calls.clear();
        }
        if let Some(line) = thinking_start_line(
            &assistant.reasoning_content,
            self.thinking_elapsed_seconds(),
        ) {
            self.emit(line);
        }
        let text = assistant.content.clone();
        let tool_calls = assistant.tool_calls.clone();
        if !text.trim().is_empty() {
            self.emit(text.clone());
        }
        self.messages.push(assistant);
        if tool_calls.is_empty() {
            let text = if text.trim().is_empty() && last_turn {
                self.emit(LAST_TURN_FALLBACK.to_string());
                LAST_TURN_FALLBACK.to_string()
            } else {
                text
            };
            return Ok(TurnControl::Stop(text));
        }
        for mut call in tool_calls {
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            self.assign_call_id(&mut call);
            if call.name == "Agent" {
                self.push_tool_output(&call, ToolOutput::text("子 Agent 不能再委派子 Agent"))
                    .await;
                continue;
            }
            self.emit_tool_start(&call).await;
            let output = self.execute_logged_tool(&call).await;
            self.emit_tool_result(&call, &output).await;
            self.append_tool_message(&call, output);
        }
        Ok(TurnControl::Continue)
    }

    /// 连续相同参数的调用计数；达到上限时返回拒绝文案。
    fn repeat_guard(&mut self, call: &ToolCall) -> Option<String> {
        let key = format!("{}\n{}", call.name, call.arguments);
        if self.last_tool_key.as_deref() == Some(key.as_str()) {
            self.last_tool_repeat = self.last_tool_repeat.saturating_add(1);
        } else {
            self.last_tool_key = Some(key);
            self.last_tool_repeat = 1;
        }
        if self.last_tool_repeat >= REPEAT_TOOL_LIMIT {
            return Some(format!(
                "重复调用被拒绝：你已用相同参数连续调用 {} {} 次。请改用其他工具或直接给出最终结论。",
                call.name, self.last_tool_repeat
            ));
        }
        None
    }

    async fn execute_logged_tool(&mut self, call: &ToolCall) -> ToolOutput {
        if let Some(rejection) = self.repeat_guard(call) {
            return ToolOutput::error(rejection);
        }
        match execute_tool_call(&self.ctx, call).await {
            Ok(value) => value,
            Err(error) => ToolOutput::error(error),
        }
    }

    /// 同一轮工具调用的调度：连续的 `Agent` 调用成批并行；连续的
    /// `concurrent_safe` 只读工具并行（上限 [`MAX_PARALLEL_TOOL_CALLS`]）；
    /// 其余按顺序执行。结果始终按模型给出的顺序回填。
    async fn execute_tool_calls(
        &mut self,
        mut calls: Vec<ToolCall>,
        client: Option<&ModelClient>,
    ) -> Result<(), String> {
        for call in &mut calls {
            self.assign_call_id(call);
        }
        let registry = self.contract_registry();
        let mut index = 0;
        while index < calls.len() {
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            if calls[index].name == "Agent" {
                let mut end = index + 1;
                while end < calls.len() && calls[end].name == "Agent" {
                    end += 1;
                }
                self.run_agent_batch(&calls[index..end], client).await?;
                index = end;
                continue;
            }
            let parallel_ok = |call: &ToolCall| {
                call.name != "Agent" && registry.resolve(&call.name).can_run_concurrently()
            };
            if parallel_ok(&calls[index]) {
                let mut end = index + 1;
                while end < calls.len()
                    && end - index < MAX_PARALLEL_TOOL_CALLS
                    && parallel_ok(&calls[end])
                {
                    end += 1;
                }
                if end - index > 1 {
                    self.run_parallel_batch(&calls[index..end]).await?;
                    index = end;
                    continue;
                }
            }
            let call = &calls[index];
            self.emit_tool_start(call).await;
            let output = self.execute_logged_tool(call).await;
            self.emit_tool_result(call, &output).await;
            self.append_tool_message(call, output);
            index += 1;
        }
        Ok(())
    }

    /// 并行执行一批只读工具；`ToolCtx` 克隆共享已读文件与待办状态。
    async fn run_parallel_batch(&mut self, calls: &[ToolCall]) -> Result<(), String> {
        let mut slot: Vec<Option<ToolOutput>> = vec![None; calls.len()];
        let mut join_set = JoinSet::new();
        for (pos, call) in calls.iter().enumerate() {
            self.emit_tool_start(call).await;
            if let Some(rejection) = self.repeat_guard(call) {
                slot[pos] = Some(ToolOutput::error(rejection));
                continue;
            }
            let ctx = self.ctx.clone();
            let call = call.clone();
            join_set.spawn(async move {
                let output = match execute_tool_call(&ctx, &call).await {
                    Ok(value) => value,
                    Err(error) => ToolOutput::error(error),
                };
                (pos, output)
            });
        }
        while let Some(joined) = join_set.join_next().await {
            let (pos, output) = joined.map_err(|error| format!("并行工具任务失败: {error}"))?;
            slot[pos] = Some(output);
        }
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        for (call, output) in calls.iter().zip(slot) {
            let output = output.unwrap_or_else(|| ToolOutput::error("工具未返回结果"));
            self.emit_tool_result(call, &output).await;
            self.append_tool_message(call, output);
        }
        Ok(())
    }

    fn spawn_child_runner(&self, spec: &SubagentSpec, index: u32) -> AgentRunner {
        self.spawn_child_with_quota(spec, index, self.child_quota_for_share())
    }

    fn child_quota_for_share(&self) -> Option<Arc<ChildQuota>> {
        if self.rollout_budget.limit() == 0 {
            None
        } else {
            let share = u64::from(self.subagent_budget_share_percent.clamp(5, 100));
            let limit = self.rollout_budget.remaining().saturating_mul(share) / 100;
            Some(ChildQuota::shared(limit))
        }
    }

    fn spawn_child_with_quota(
        &self,
        spec: &SubagentSpec,
        index: u32,
        child_quota: Option<Arc<ChildQuota>>,
    ) -> AgentRunner {
        let mut child = AgentRunner::new(self.ctx.workspace.clone());
        // 共享取消 / 权限放行 / MCP 服务器放行 / 代理环境；已读文件与待办独立。
        child.ctx = self.ctx.fork_for_child();
        child.artifacts = self.artifacts.clone();
        child.lite_model = self.lite_model.clone();
        child.extra_tool_contracts = self.extra_tool_contracts.clone();
        child.skills_prompt = self.skills_prompt.clone();
        child.depth = self.depth.saturating_add(1);
        child.event_prefix = format!(
            "{} ",
            format_subagent_log_tag(index, &spec.kind, &spec.description)
        );
        child.max_turns = self.max_subagent_turns;
        child.max_subagent_turns = self.max_subagent_turns;
        child.max_concurrent_subagents = self.max_concurrent_subagents;
        child.subagent_policy = self.subagent_policy.clone();
        child.context_char_limit = self.context_char_limit;
        child.rollout_budget = self.rollout_budget.clone();
        child.subagent_budget_share_percent = self.subagent_budget_share_percent;
        child.child_quota = child_quota;
        child.context_window = ContextWindow::new(self.context_window.token_limit);
        child.tool_result_token_limit = self.tool_result_token_limit;
        child.diagnostics = self.diagnostics.clone();
        child.on_event = self.on_event.clone();
        child.on_usage = self.on_usage.clone();
        child.workspace_context = self.workspace_context.clone();
        child.project_agents = self.project_agents.clone();
        let custom_def = match &spec.kind {
            SubagentKind::Custom(name) => {
                find_native_subagent(&self.custom_subagents, name).cloned()
            }
            _ => None,
        };
        match &spec.kind {
            SubagentKind::Explore => {
                child.ctx.set_read_only(true);
                child.set_allowed_tools(&read_only_tool_names());
            }
            SubagentKind::General => {
                child.ctx.mcp = self.ctx.mcp.clone();
                child.set_extra_tools(self.extra_tools.clone());
            }
            SubagentKind::Custom(_) => {
                if let Some(def) = custom_def.as_ref() {
                    if def.tool_mode == TOOL_MODE_ALL {
                        child.ctx.mcp = self.ctx.mcp.clone();
                        child.set_extra_tools(self.extra_tools.clone());
                    } else {
                        let tools = effective_custom_tools(&def.tools);
                        let names: Vec<&str> = tools.iter().map(String::as_str).collect();
                        child.set_allowed_tools(&names);
                        if custom_tools_are_read_only(&tools) {
                            child.ctx.set_read_only(true);
                        }
                    }
                    if !def.disallowed_tools.is_empty() {
                        child.set_disallowed_tools(&def.disallowed_tools);
                    }
                    if let Some(max_turns) = def.max_turns {
                        child.max_turns = max_turns.max(1) as u32;
                    }
                    // 档案限定了技能时，子 Agent 只看到这些技能。
                    if !def.skills.is_empty() {
                        child.ctx.skills.retain(|skill| {
                            def.skills
                                .iter()
                                .any(|name| name.eq_ignore_ascii_case(&skill.name))
                        });
                        child.skills_prompt =
                            crate::native::skills::format_skills_prompt(&child.ctx.skills);
                    }
                    // 档案自带权限模式时，子 Agent 用自己的放行开关，不再共享父会话的。
                    if let Some(mode) = def.permission_mode.as_deref() {
                        use crate::native::settings::{
                            permission_mode_auto_approves_build,
                            permission_mode_auto_approves_edits, permission_mode_is_yolo,
                        };
                        child.ctx.allow_all_high_risk = Arc::new(
                            std::sync::atomic::AtomicBool::new(permission_mode_is_yolo(mode)),
                        );
                        child.ctx.auto_approve_overwrite =
                            permission_mode_auto_approves_edits(mode);
                        child.ctx.auto_approve_opaque_bash =
                            permission_mode_auto_approves_build(mode);
                        child.ctx.auto_approve_readonly_mcp =
                            permission_mode_auto_approves_build(mode);
                    }
                }
            }
        }
        let parent_system = self
            .messages
            .iter()
            .find(|message| message.role == Role::System)
            .map(|message| message.content.clone());
        let system = if let Some(def) = custom_def.as_ref() {
            custom_child_system_prompt(spec, def, &self.workspace_context, &self.project_agents)
        } else {
            child_system_prompt(parent_system.as_deref(), spec)
        };
        child.messages.push(Message::system(system));
        child
    }

    async fn run_agent_batch(
        &mut self,
        calls: &[ToolCall],
        client: Option<&ModelClient>,
    ) -> Result<(), String> {
        if self.depth > 0 {
            for call in calls {
                self.push_tool_output(call, ToolOutput::text("子 Agent 不能再委派子 Agent"))
                    .await;
            }
            return Ok(());
        }
        if let Some(reload) = &self.reload_custom_subagents {
            self.custom_subagents = reload();
        }
        let mut slot: Vec<Option<(ToolCall, String)>> = vec![None; calls.len()];
        struct Job {
            call: ToolCall,
            spec: SubagentSpec,
            index: u32,
        }
        let mut jobs = Vec::new();
        for (pos, call) in calls.iter().enumerate() {
            match parse_subagent_args_with(&call.arguments, &self.custom_subagents) {
                Ok(spec) => {
                    self.subagent_seq = self.subagent_seq.saturating_add(1);
                    jobs.push((
                        pos,
                        Job {
                            call: call.clone(),
                            spec,
                            index: self.subagent_seq,
                        },
                    ));
                }
                Err(error) => slot[pos] = Some((call.clone(), error)),
            }
        }
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent_subagents.max(1) as usize));
        let mut join_set = JoinSet::new();
        let stub = self.subagent_stub.clone();
        let model_turn = self.model_turn.clone();
        let client_owned = client.cloned();
        let batch_quota = self.child_quota_for_share();
        for (pos, job) in jobs {
            self.emit_tool_start(&job.call).await;
            self.emit(format!(
                "{} 启动（{}）",
                format_subagent_log_tag(job.index, &job.spec.kind, &job.spec.description),
                job.spec.kind.as_str()
            ));
            self.emit_activity(
                "native_subagent_started",
                &format!("{}（{}）", job.spec.description, job.spec.kind.as_str()),
            );
            let permit = semaphore.clone();
            let stub = stub.clone();
            let client_owned = client_owned.clone();
            let model_turn = model_turn.clone();
            let child_model_loader = self.child_model_loader.clone();
            let custom_override = match &job.spec.kind {
                SubagentKind::Custom(name) => find_native_subagent(&self.custom_subagents, name)
                    .filter(|item| item.model_mode == MODEL_MODE_CHANNEL)
                    .and_then(|item| Some((item.channel_id.clone()?, item.model.clone()?))),
                _ => None,
            };
            let mut child = self.spawn_child_with_quota(&job.spec, job.index, batch_quota.clone());
            self.diagnostics
                .subagents_started
                .fetch_add(1, Ordering::AcqRel);
            if job.spec.run_in_background {
                // 后台任务：立刻返回 task_id，子 Agent 在独立任务里跑，结果进注册表。
                let (task, steer_rx) = self
                    .background
                    .register(&job.spec.description, job.spec.kind.as_str());
                let parent_cancel = self.ctx.cancel.clone();
                child.ctx.cancel = task.cancel.clone();
                child.steer_rx = Some(Arc::new(Mutex::new(steer_rx)));
                child.ctx.coordinator = Some((self.background.clone(), task.id.clone()));
                let registry = self.background.clone();
                let task_id = task.id.clone();
                let on_event = self.on_event.clone();
                let prefix = self.event_prefix.clone();
                let tag = format_subagent_log_tag(job.index, &job.spec.kind, &job.spec.description);
                let spec = job.spec.clone();
                let run = run_child_job(
                    child,
                    spec.clone(),
                    None,
                    stub,
                    custom_override,
                    child_model_loader,
                    client_owned,
                    model_turn,
                );
                let cancel_flag = task.cancel.clone();
                tokio::spawn(async move {
                    let outcome = tokio::select! {
                        result = run => result,
                        _ = async {
                            loop {
                                if parent_cancel.is_cancelled() {
                                    cancel_flag.cancel();
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        } => Err("父会话已取消".to_string()),
                    };
                    let status = if outcome.is_ok() { "成功" } else { "失败" };
                    Self::send_prefixed_event(
                        &on_event,
                        &prefix,
                        format!("{tag} 后台任务 {task_id} 结束 {status}"),
                    );
                    registry.finish(&task_id, outcome.map(|report| truncate_report(&report)));
                });
                slot[pos] = Some((
                    job.call,
                    format!(
                        "后台任务已启动：task_id={}（{} / {}）。用 TaskOutput 读取结果、SendMessage 追加指令、TaskStop 停止；任务完成时会收到提醒。",
                        task.id,
                        spec.kind.as_str(),
                        spec.description
                    ),
                ));
                continue;
            }
            let run = run_child_job(
                child,
                job.spec.clone(),
                Some(permit),
                stub,
                custom_override,
                child_model_loader,
                client_owned,
                model_turn,
            );
            join_set.spawn(async move {
                let outcome = run.await;
                (pos, job, outcome)
            });
        }
        while let Some(joined) = join_set.join_next().await {
            let (pos, job, outcome) =
                joined.map_err(|error| format!("子 Agent 任务失败: {error}"))?;
            let output = match &outcome {
                Ok(report) => format_subagent_result(&job.spec, Ok(report)),
                Err(error) => format_subagent_result(&job.spec, Err(error)),
            };
            let status = if outcome.is_ok() { "成功" } else { "失败" };
            self.emit(format!(
                "{} 结束 {status}",
                format_subagent_log_tag(job.index, &job.spec.kind, &job.spec.description)
            ));
            self.emit_activity(
                "native_subagent_finished",
                &format!(
                    "{}（{}）{status}",
                    job.spec.description,
                    job.spec.kind.as_str()
                ),
            );
            slot[pos] = Some((job.call, output));
        }
        for item in slot.into_iter().flatten() {
            self.push_tool_output(&item.0, ToolOutput::text(item.1))
                .await;
        }
        Ok(())
    }

    async fn push_tool_output(&mut self, call: &ToolCall, output: ToolOutput) {
        self.emit_tool_result(call, &output).await;
        self.append_tool_message(call, output);
    }

    /// 先按契约结果预算裁决（超预算的 Artifact 策略落盘、只留预览），再按会话的
    /// token 上限截断；图片附件以紧随其后的用户消息形式交给模型。
    fn append_tool_message(&mut self, call: &ToolCall, output: ToolOutput) {
        let contract = self.contract_registry().resolve(&call.name);
        let budgeted =
            bound_with_artifact(self.artifacts.as_deref(), &contract, call, &output.text);
        let bounded = truncate_tool_result(
            &call.name,
            &call.arguments,
            &budgeted,
            self.tool_result_token_limit,
        );
        if bounded != output.text {
            self.diagnostics
                .tool_results_truncated
                .fetch_add(1, Ordering::AcqRel);
        }
        let mut message = Message::tool_result(&call.id, bounded);
        message.name = call.name.clone();
        self.messages.push(message);
        if !output.images.is_empty() {
            let names: Vec<&str> = output
                .images
                .iter()
                .map(|image| image.name.as_str())
                .collect();
            self.messages.push(Message::user_with_images(
                format!("（{} 工具返回的图片：{}）", call.name, names.join("、")),
                output.images,
            ));
        }
    }
}

/// 跑一个子 Agent：可选并发许可、测试桩、自定义渠道模型或继承父模型。
#[allow(clippy::too_many_arguments)]
fn run_child_job(
    mut child: AgentRunner,
    spec: SubagentSpec,
    permit: Option<Arc<Semaphore>>,
    stub: Option<SubagentStub>,
    custom_override: Option<(String, String)>,
    child_model_loader: Option<ChildModelLoader>,
    client_owned: Option<ModelClient>,
    model_turn: Option<ModelTurnCfg>,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
    Box::pin(async move {
        let _permit = match permit {
            Some(semaphore) => Some(
                semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| "子 Agent 并发许可已关闭".to_string())?,
            ),
            None => None,
        };
        if let Some(stub) = stub {
            Ok(stub(&spec))
        } else if let Some((channel_id, model)) = custom_override {
            let Some(loader) = child_model_loader else {
                return Err("子 Agent 需要模型客户端".to_string());
            };
            let settings = loader(channel_id, model).await?;
            child
                .run_child_with_client(
                    client_owned.as_ref(),
                    &settings.client,
                    &spec.prompt,
                    &settings.model,
                    settings.effort.as_deref(),
                    settings.max_output_tokens,
                    settings.thinking_enabled,
                    Some(&spec),
                )
                .await
        } else if let (Some(client), Some(cfg)) = (client_owned.as_ref(), model_turn) {
            child
                .run_child_with_client(
                    Some(client),
                    client,
                    &spec.prompt,
                    &cfg.model,
                    cfg.effort.as_deref(),
                    cfg.max_output_tokens,
                    cfg.thinking_enabled,
                    Some(&spec),
                )
                .await
        } else {
            Err("子 Agent 需要模型客户端".to_string())
        }
    })
}

fn last_turn_after_response(
    planned_last_turn: bool,
    requested_with_tools: bool,
    assistant: &Message,
    budget_exhausted: bool,
) -> bool {
    let honor_tools = requested_with_tools
        && assistant
            .tool_calls
            .iter()
            .any(|call| !call.name.trim().is_empty());
    (planned_last_turn || budget_exhausted) && !honor_tools
}

fn apply_usable_summary(messages: &mut Vec<Message>, summary: &Message) -> bool {
    summary.tool_calls.is_empty()
        && is_usable_compaction_summary(&summary.content)
        && compact_with_summary(messages, summary.content.trim())
}

fn append_last_turn_reminder(messages: &mut Vec<Message>) {
    append_user_note(messages, LAST_TURN_REMINDER);
}

/// 把提示追加到最后一条用户 / 工具消息末尾（保持角色交替），否则新起一条用户消息。
fn append_user_note(messages: &mut Vec<Message>, note: &str) {
    if let Some(last) = messages.last_mut() {
        if last.role == Role::User || last.role == Role::Tool {
            if !last.content.is_empty() {
                last.content.push_str("\n\n");
            }
            last.content.push_str(note);
            return;
        }
    }
    messages.push(Message::user(note.to_string()));
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|item| item.as_millis() as u64)
        .unwrap_or(0)
}

fn thinking_duration_seconds(elapsed_ms: u64) -> u32 {
    u32::try_from(elapsed_ms.saturating_add(500) / 1000)
        .unwrap_or(u32::MAX)
        .max(1)
}

fn thinking_start_line(content: &str, seconds: u32) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("[思考] {seconds}秒\n{trimmed}"))
}

fn tool_start_line(name: &str, arguments: &str) -> String {
    tool_start_line_ex(name, arguments, None, None)
}

fn tool_start_line_ex(
    name: &str,
    arguments: &str,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    match name {
        "Read" => format!("[读取] {}", json_string(&args, "file_path")),
        "Write" => format!("[写入] {}", json_string(&args, "file_path")),
        "Edit" => format!("[编辑] {}", json_string(&args, "file_path")),
        "Bash" => format!("[命令] {}", json_string(&args, "command")),
        "Glob" => format!("[工具] Glob {}", json_string(&args, "pattern")),
        "Grep" => format!("[工具] Grep {}", json_string(&args, "pattern")),
        "TodoRead" => "[待办] 读取任务清单".to_string(),
        "TodoWrite" => format_todo_write_start(&args),
        "ApplyPatch" => "[补丁] 应用多文件补丁".to_string(),
        "Skill" => format!("[技能] {}", json_string(&args, "name")),
        "Agent" => format!("[子 Agent] {}", json_string(&args, "description")),
        "WebFetch" => format!("[工具] WebFetch {}", json_string(&args, "url")),
        "WebSearch" => format!("[工具] WebSearch {}", json_string(&args, "query")),
        "AskUserQuestion" | "AskQuestion" => {
            format!("[工具] 提问 {}", first_question_prompt(&args))
        }
        "EnterPlanMode" => "[工具] EnterPlanMode".to_string(),
        "ExitPlanMode" => "[工具] ExitPlanMode".to_string(),
        "TaskOutput" => format!("[工具] TaskOutput {}", json_string(&args, "task_id")),
        "TaskStop" => format!("[工具] TaskStop {}", json_string(&args, "task_id")),
        "SendMessage" => format!("[工具] SendMessage {}", json_string(&args, "task_id")),
        "RespondToCoordinator" => "[工具] RespondToCoordinator".to_string(),
        "CronCreate" => format!(
            "[工具] CronCreate {}",
            first_of(&args, &["name", "cron", "expression"])
        ),
        "CronList" => "[工具] CronList".to_string(),
        "CronDelete" => format!("[工具] CronDelete {}", first_of(&args, &["id"])),
        "Goal" => format!("[工具] Goal {}", first_of(&args, &["title", "action"])),
        "GoalRead" => "[工具] GoalRead".to_string(),
        "ReadSessionContext" => format!(
            "[工具] ReadSessionContext {}",
            first_of(&args, &["session_id", "query"])
        ),
        other => format_generic_tool_line(other, &args, mcp_server, mcp_tool),
    }
}

fn format_generic_tool_line(
    name: &str,
    args: &Value,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> String {
    let extra = compact_args(args);
    if let (Some(server), Some(tool)) = (mcp_server, mcp_tool) {
        return if extra.is_empty() {
            format!("[MCP工具] {server} / {tool}")
        } else {
            format!("[MCP工具] {server} / {tool} {extra}")
        };
    }
    if name.starts_with("mcp_") {
        return if extra.is_empty() {
            format!("[MCP工具] {name}")
        } else {
            format!("[MCP工具] {name} {extra}")
        };
    }
    if extra.is_empty() {
        format!("[工具] {name}")
    } else {
        format!("[工具] {name} {extra}")
    }
}

fn tool_event_title(line: &str) -> String {
    let first = line.split('\n').next().unwrap_or(line).trim();
    let Some(rest) = first.strip_prefix('[') else {
        return first.to_string();
    };
    let Some(end) = rest.find(']') else {
        return first.to_string();
    };
    let label = rest[..end].trim();
    let after = rest[end + 1..].trim();
    if after.is_empty() {
        label.to_string()
    } else {
        format!("{label} {after}")
    }
}

fn tool_args_summary(name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    match name {
        "Read" | "Write" | "Edit" => json_opt(&args, "file_path").unwrap_or_default(),
        "Bash" => json_opt(&args, "command").unwrap_or_default(),
        "Glob" | "Grep" => json_opt(&args, "pattern").unwrap_or_default(),
        "Skill" => json_opt(&args, "name").unwrap_or_default(),
        "Agent" => json_opt(&args, "description").unwrap_or_default(),
        "WebFetch" => json_opt(&args, "url").unwrap_or_default(),
        "WebSearch" => json_opt(&args, "query").unwrap_or_default(),
        "TaskOutput" | "TaskStop" | "SendMessage" => json_opt(&args, "task_id").unwrap_or_default(),
        _ => compact_args(&args),
    }
}

fn first_question_prompt(args: &Value) -> String {
    args.get("questions")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("prompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, 80))
        .unwrap_or_else(|| "(unknown)".to_string())
}

fn first_of(args: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = json_opt(args, key) {
            return value;
        }
    }
    "(unknown)".to_string()
}

fn json_opt(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
}

fn compact_args(args: &Value) -> String {
    match args {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        Value::Object(map) => {
            if let Some((_, Value::String(text))) = map
                .iter()
                .find(|(_, value)| value.as_str().is_some_and(|item| !item.trim().is_empty()))
            {
                return truncate_chars(text.trim(), 120);
            }
            truncate_chars(&args.to_string(), 120)
        }
        other => truncate_chars(&other.to_string(), 120),
    }
}

fn format_todo_write_start(args: &Value) -> String {
    let Some(todos) = args.get("todos").and_then(Value::as_array) else {
        return "[待办] 更新任务清单".to_string();
    };
    if todos.is_empty() {
        return "[待办] (空)".to_string();
    }
    let lines: Vec<String> = todos.iter().filter_map(format_todo_item_line).collect();
    if lines.is_empty() {
        return "[待办] 更新任务清单".to_string();
    }
    format!("[待办]\n{}", lines.join("\n"))
}

fn format_todo_item_line(item: &Value) -> Option<String> {
    let content = item
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("pending");
    let priority = item
        .get("priority")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("medium");
    Some(format!(
        "- [{}] {} ({})",
        status,
        truncate_chars(content, 200),
        priority
    ))
}

fn tool_result_line(name: &str, output: &str) -> String {
    match name {
        "TodoWrite" if is_todo_list_output(output) => {
            format!("[工具结果] 已更新 {} 项", count_todo_item_lines(output))
        }
        _ => format!("[工具结果]\n{}", cap_tool_result_display(output)),
    }
}

fn cap_tool_result_display(output: &str) -> String {
    let trimmed = output.trim_end();
    let line_count = trimmed.lines().count();
    let char_count = trimmed.chars().count();
    if line_count <= TOOL_RESULT_DISPLAY_MAX_LINES && char_count <= TOOL_RESULT_DISPLAY_MAX_CHARS {
        return trimmed.to_string();
    }
    let mut prefix = String::new();
    let mut used_chars = 0usize;
    for line in trimmed.lines().take(TOOL_RESULT_DISPLAY_MAX_LINES) {
        let extra = usize::from(!prefix.is_empty());
        let line_chars = line.chars().count();
        if used_chars + extra + line_chars > TOOL_RESULT_DISPLAY_MAX_CHARS {
            let remaining = TOOL_RESULT_DISPLAY_MAX_CHARS.saturating_sub(used_chars + extra);
            if remaining > 0 {
                if extra == 1 {
                    prefix.push('\n');
                }
                prefix.extend(line.chars().take(remaining));
            }
            break;
        }
        if extra == 1 {
            prefix.push('\n');
        }
        prefix.push_str(line);
        used_chars += extra + line_chars;
    }
    format!("{prefix}\n…（已截断，共 {line_count} 行 / {char_count} 字）")
}

fn is_todo_list_output(output: &str) -> bool {
    let trimmed = output.trim();
    trimmed == "(no todos)" || count_todo_item_lines(output) > 0
}

fn count_todo_item_lines(output: &str) -> usize {
    output
        .lines()
        .filter(|line| line.trim_start().starts_with("- ["))
        .count()
}

fn json_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(unknown)".to_string())
}

fn truncate_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{prefix}…")
}

pub fn assistant_tool_call(id: &str, name: &str, arguments: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        }],
        tool_call_id: String::new(),
        name: String::new(),
        reasoning_content: String::new(),
        images: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::subagent::parse_subagent_args;
    use super::*;
    use crate::native::model::types::Message;
    use std::fs;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_runner() -> (AgentRunner, std::path::PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("codex-ai-agent-{stamp}-{seq}"));
        fs::create_dir_all(&root).expect("mkdir");
        fs::write(root.join("hello.txt"), "hello world\n").expect("write");
        let runner = AgentRunner::new(LocalWorkspace::new(root.clone()));
        runner
            .ctx
            .allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        (runner, root)
    }

    fn drain_events(rx: &mut mpsc::UnboundedReceiver<NativeEvent>) -> Vec<String> {
        let mut lines = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                NativeEvent::Line(line) | NativeEvent::Tool { line, .. } => lines.push(line),
                _ => {}
            }
        }
        lines
    }

    fn capture_checkpoints(runner: &mut AgentRunner) -> Arc<tokio::sync::Mutex<Vec<Vec<Message>>>> {
        let snapshots = Arc::new(tokio::sync::Mutex::new(Vec::<Vec<Message>>::new()));
        let captured = snapshots.clone();
        runner.on_checkpoint = Some(Arc::new(move |messages| {
            let captured = captured.clone();
            Box::pin(async move {
                captured.lock().await.push(messages);
            })
        }));
        snapshots
    }

    fn assistant_tool_calls(calls: &[(&str, &str, &str)]) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: calls
                .iter()
                .map(|(id, name, arguments)| ToolCall {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    arguments: (*arguments).to_string(),
                })
                .collect(),
            tool_call_id: String::new(),
            name: String::new(),
            reasoning_content: String::new(),
            images: Vec::new(),
        }
    }

    #[tokio::test]
    async fn parallel_read_only_batch_keeps_result_order_and_serializes_writes() {
        let (mut runner, root) = temp_runner();
        fs::write(root.join("second.txt"), "second file\n").expect("write");
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let text = runner
            .run_scripted(
                "并行读取",
                vec![
                    assistant_tool_calls(&[
                        ("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                        ("c2", "Glob", r#"{"pattern":"*.txt"}"#),
                        ("c3", "Read", r#"{"file_path":"second.txt"}"#),
                        (
                            "c4",
                            "Write",
                            r#"{"file_path":"hello.txt","content":"rewritten"}"#,
                        ),
                        ("c5", "Read", r#"{"file_path":"hello.txt"}"#),
                    ]),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        assert_eq!(text, "done");
        let tool_messages: Vec<&Message> = runner
            .messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .collect();
        assert_eq!(
            tool_messages
                .iter()
                .map(|message| message.tool_call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1", "c2", "c3", "c4", "c5"]
        );
        assert!(tool_messages[0].content.contains("hello world"));
        assert!(tool_messages[1].content.contains("second.txt"));
        assert!(tool_messages[2].content.contains("second file"));
        assert!(tool_messages[3].content.contains("Wrote"));
        // 写入排在并行批之后串行执行，之后的 Read 必须看到新内容。
        assert!(tool_messages[4].content.contains("rewritten"));
        let lines = drain_events(&mut rx);
        let starts: Vec<&String> = lines
            .iter()
            .filter(|line| line.starts_with("[读取]") || line.starts_with("[工具] Glob"))
            .collect();
        assert_eq!(starts.len(), 4);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn manual_compaction_writes_boundary_and_summarizes_history() {
        let (mut runner, root) = temp_runner();
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        // 先跑两个回合积累历史，再请求手动压缩。
        runner
            .run_scripted("第一件事", vec![Message::assistant_text("做完第一件")])
            .await
            .expect("first");
        runner
            .run_scripted("第二件事", vec![Message::assistant_text("做完第二件")])
            .await
            .expect("second");
        runner.request_manual_compaction(Some("保留失败堆栈".to_string()));
        let text = runner
            .run_scripted("第三件事", vec![Message::assistant_text("继续")])
            .await
            .expect("third");
        assert_eq!(text, "继续");
        let lines = drain_events(&mut rx);
        let boundary_line = lines
            .iter()
            .find(|line| line.starts_with("[COMPACT_BOUNDARY]"))
            .expect("boundary line");
        let boundary = CompactBoundary::parse_line(boundary_line).expect("parse");
        assert_eq!(boundary.trigger, CompactTrigger::Manual);
        assert_eq!(boundary.source, "local");
        assert_eq!(boundary.instructions.as_deref(), Some("保留失败堆栈"));
        assert!(boundary.post_messages <= boundary.pre_messages);
        assert_eq!(runner.context_window.compactions, 1);
        assert!(runner
            .messages
            .iter()
            .any(|message| message.content.contains("第三件事")));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn background_agent_does_not_emit_duplicate_start() {
        let (mut runner, root) = temp_runner();
        runner.subagent_stub = Some(Arc::new(|spec| format!("stub:{}", spec.description)));
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call(
                        "c1",
                        "Agent",
                        r#"{"description":"bg","prompt":"do it","run_in_background":true}"#,
                    ),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        let lines = drain_events(&mut rx);
        let starts = lines
            .iter()
            .filter(|line| line.starts_with("[子 Agent]"))
            .count();
        assert_eq!(starts, 1, "{lines:?}");
        let results = lines
            .iter()
            .filter(|line| line.starts_with("[工具结果]"))
            .count();
        assert_eq!(results, 1, "{lines:?}");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn background_agent_returns_task_id_and_task_output_waits() {
        let (mut runner, root) = temp_runner();
        runner.subagent_stub = Some(Arc::new(|spec| format!("stub:{}", spec.description)));
        let text = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call(
                        "c1",
                        "Agent",
                        r#"{"description":"bg","prompt":"do it","run_in_background":true}"#,
                    ),
                    assistant_tool_call(
                        "c2",
                        "TaskOutput",
                        r#"{"task_id":"task-1","wait":true,"timeout_ms":5000}"#,
                    ),
                    assistant_tool_call("c3", "TaskStop", r#"{"task_id":"task-1"}"#),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        assert_eq!(text, "done");
        let tools: Vec<&Message> = runner
            .messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .collect();
        assert!(
            tools[0].content.contains("task_id=task-1"),
            "{}",
            tools[0].content
        );
        assert!(tools[1].content.contains("stub:bg"), "{}", tools[1].content);
        assert!(
            tools[2].content.contains("早已结束"),
            "{}",
            tools[2].content
        );
        // 主 Agent 能看到后台任务工具，子 Agent 看不到。
        assert!(runner.tool_names().iter().any(|name| name == "TaskOutput"));
        let spec = parse_subagent_args(r#"{"prompt":"x","subagent_type":"general"}"#).unwrap();
        let child = runner.spawn_child_runner(&spec, 9);
        assert!(!child.tool_names().iter().any(|name| name == "TaskOutput"));
        assert!(!child
            .tool_names()
            .iter()
            .any(|name| name == "RespondToCoordinator"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compaction_options_apply_threshold_and_microcompact_flag() {
        let (mut runner, root) = temp_runner();
        runner.set_compaction_options(60, false);
        assert_eq!(runner.context_window.threshold_percent, 60);
        assert!(!runner.microcompact_enabled);
        runner.set_compaction_options(5, true);
        assert_eq!(runner.context_window.threshold_percent, 30);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn checkpoints_user_message_before_model_and_after_tool_round() {
        let (mut runner, root) = temp_runner();
        let snapshots = capture_checkpoints(&mut runner);
        let text = runner
            .run_scripted(
                "分析项目",
                vec![
                    assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        assert_eq!(text, "done");
        let snaps = snapshots.lock().await;
        assert!(
            snaps.len() >= 3,
            "expected user, tool-round, and final checkpoints: {}",
            snaps.len()
        );
        assert_eq!(
            snaps[0]
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>(),
            vec![(Role::User, "分析项目")]
        );
        assert!(!snaps[0]
            .iter()
            .any(|message| message.role == Role::Assistant));
        let after_tools = snaps
            .iter()
            .find(|snapshot| snapshot.iter().any(|message| message.role == Role::Tool))
            .expect("tool-round checkpoint");
        assert!(after_tools
            .iter()
            .any(|message| { message.role == Role::Assistant && !message.tool_calls.is_empty() }));
        assert!(after_tools
            .iter()
            .any(|message| message.role == Role::Tool && message.content.contains("hello world")));
        let last = snaps.last().expect("final checkpoint");
        assert!(last
            .iter()
            .any(|message| message.role == Role::Assistant && message.content == "done"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn checkpoints_injected_steer_before_next_model_call() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(1);
        let (steer_tx, steer_rx) = mpsc::channel(1);
        runner.steer_rx = Some(Arc::new(Mutex::new(steer_rx)));
        steer_tx
            .send(NativeFollowup::Input("补充约束".to_string()))
            .await
            .expect("steer");
        let snapshots = capture_checkpoints(&mut runner);
        let client = ModelClient::new(crate::native::model::client::ModelClientConfig {
            protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: "test".to_string(),
            extra_headers: std::collections::HashMap::new(),
            retry: crate::native::model::RetryConfig::none(),
            timeout: Duration::from_millis(50),
            network: crate::app::network_settings::NetworkSettings::default(),
        })
        .expect("client");
        let text = runner
            .run_with_client(&client, "go", "test-model", None, None, false, Vec::new())
            .await
            .expect("budget stop");
        assert_eq!(text, LAST_TURN_FALLBACK);
        let snaps = snapshots.lock().await;
        assert!(
            snaps.iter().any(|snapshot| snapshot
                .iter()
                .any(|message| { message.role == Role::User && message.content == "go" })),
            "missing first user checkpoint: {snaps:?}"
        );
        let with_steer = snaps
            .iter()
            .find(|snapshot| {
                snapshot
                    .iter()
                    .any(|message| message.role == Role::User && message.content == "补充约束")
            })
            .expect("steer checkpoint");
        assert_eq!(
            with_steer
                .iter()
                .filter(|message| message.role == Role::User)
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["go", "补充约束"]
        );
        assert!(!with_steer
            .iter()
            .any(|message| message.role == Role::Assistant));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn reads_then_edits_file() {
        let (mut runner, root) = temp_runner();
        let replies = vec![
            assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
            assistant_tool_call(
                "c2",
                "Edit",
                r#"{"file_path":"hello.txt","old_string":"hello world","new_string":"goodbye world"}"#,
            ),
            Message::assistant_text("done"),
        ];
        let text = runner
            .run_scripted("fix the greeting", replies)
            .await
            .expect("run");
        assert_eq!(text, "done");
        let content = fs::read_to_string(root.join("hello.txt")).expect("read result");
        assert_eq!(content, "goodbye world\n");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cancel_stops_before_next_model_call() {
        let (mut runner, root) = temp_runner();
        runner.max_turns = 8;
        runner.cancel();
        let error = runner
            .run_scripted(
                "go",
                vec![assistant_tool_call(
                    "c1",
                    "Read",
                    r#"{"file_path":"hello.txt"}"#,
                )],
            )
            .await
            .unwrap_err();
        assert_eq!(error, "已取消");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn last_turn_stops_with_fallback_instead_of_error() {
        let (mut runner, root) = temp_runner();
        runner.max_turns = 1;
        let text = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    Message::assistant_text("should not run"),
                ],
            )
            .await
            .expect("last turn");
        assert_eq!(text, LAST_TURN_FALLBACK);
        let original = fs::read_to_string(root.join("hello.txt")).expect("read");
        assert_eq!(original, "hello world\n");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn last_turn_keeps_model_text() {
        let (mut runner, root) = temp_runner();
        runner.max_turns = 2;
        let text = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    Message::assistant_text("审查通过"),
                ],
            )
            .await
            .expect("run");
        assert_eq!(text, "审查通过");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn budget_exhaustion_after_tool_response_still_runs_tool_calls() {
        let (mut probe, probe_root) = temp_runner();
        probe.set_allowed_tools(&["Read"]);
        probe.set_rollout_budget_limit(1_000_000);
        probe.messages.push(Message::user("go"));
        let tools = probe.combined_tools();
        assert!(probe.reserve_model_call(None, &tools).is_some());
        let spent = probe.budget_snapshot().spent;
        probe.release_model_reservation();
        let _ = fs::remove_dir_all(probe_root);

        let (mut runner, root) = temp_runner();
        runner.set_allowed_tools(&["Read"]);
        runner.set_rollout_budget_limit(spent);
        let _ = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    Message::assistant_text("已读完"),
                ],
            )
            .await
            .expect("run");
        assert!(
            runner.messages.iter().any(|message| {
                message.role == Role::Tool && message.content.contains("hello world")
            }),
            "tool call from a tools-enabled request must still execute: {:?}",
            runner.messages
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn emits_tool_progress_lines() {
        let (mut runner, root) = temp_runner();
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let _ = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        let lines = drain_events(&mut rx);
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("[读取] hello.txt")),
            "missing read start: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("[工具结果]\n") && line.contains("hello world")),
            "missing full tool result: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line == "done"),
            "missing final: {lines:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn read_only_emits_read_and_blocks_write() {
        let (mut runner, root) = temp_runner();
        runner.set_read_only(true);
        runner.set_allowed_tools(&crate::native::tools::read_only_tool_names());
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let replies = vec![
            assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
            assistant_tool_call(
                "c2",
                "Write",
                r#"{"file_path":"hello.txt","content":"changed"}"#,
            ),
            Message::assistant_text("plan ready"),
        ];
        let text = runner
            .run_scripted("plan the work", replies)
            .await
            .expect("run");
        assert_eq!(text, "plan ready");
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).expect("read"),
            "hello world\n"
        );
        let lines = drain_events(&mut rx);
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("[读取] hello.txt")),
            "missing read: {lines:?}"
        );
        assert!(
            runner
                .messages
                .iter()
                .any(|message| message.content.contains("只读规划模式禁止调用工具 Write")),
            "expected write rejection in tool results: {:?}",
            runner
                .messages
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejects_repeated_identical_tool() {
        let (mut runner, root) = temp_runner();
        let replies = vec![
            assistant_tool_call("c1", "Read", r#"{"file_path":"hello.txt"}"#),
            assistant_tool_call("c2", "Read", r#"{"file_path":"hello.txt"}"#),
            assistant_tool_call("c3", "Read", r#"{"file_path":"hello.txt"}"#),
            Message::assistant_text("ok"),
        ];
        let text = runner.run_scripted("go", replies).await.expect("run");
        assert_eq!(text, "ok");
        let refused = runner
            .messages
            .iter()
            .any(|message| message.content.contains("重复调用被拒绝"));
        assert!(refused, "expected repeat rejection in tool results");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn empty_tool_names_do_not_continue() {
        let (mut runner, root) = temp_runner();
        let mut dummy = Message::assistant_text("hello");
        dummy.tool_calls = vec![ToolCall {
            id: "empty".to_string(),
            name: String::new(),
            arguments: "{}".to_string(),
        }];
        let text = runner.run_scripted("go", vec![dummy]).await.expect("run");
        assert_eq!(text, "hello");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn thinking_start_line_keeps_full_content() {
        assert_eq!(thinking_start_line("   ", 3), None);
        assert_eq!(
            thinking_start_line("先看入口再改 Composer", 8),
            Some("[思考] 8秒\n先看入口再改 Composer".to_string())
        );
        assert_eq!(thinking_duration_seconds(0), 1);
        assert_eq!(thinking_duration_seconds(499), 1);
        assert_eq!(thinking_duration_seconds(500), 1);
        assert_eq!(thinking_duration_seconds(1499), 1);
        assert_eq!(thinking_duration_seconds(1500), 2);
    }

    #[test]
    fn todo_write_start_line_lists_all_items() {
        let line = tool_start_line(
            "TodoWrite",
            r#"{"todos":[
                {"id":"1","content":"定位 TestController","status":"in_progress","priority":"high"},
                {"id":"2","content":"实现 ok 接口","status":"pending"},
                {"id":"3","content":"补测试","status":"pending","priority":"low"}
            ]}"#,
        );
        assert_eq!(
            line,
            "[待办]\n- [in_progress] 定位 TestController (high)\n- [pending] 实现 ok 接口 (medium)\n- [pending] 补测试 (low)"
        );
        assert!(!line.contains("TodoWrite"));
    }

    #[test]
    fn todo_write_start_line_empty_and_invalid() {
        assert_eq!(
            tool_start_line("TodoWrite", r#"{"todos":[]}"#),
            "[待办] (空)"
        );
        assert_eq!(
            tool_start_line("TodoWrite", "not-json"),
            "[待办] 更新任务清单"
        );
        assert_eq!(tool_start_line("TodoWrite", "{}"), "[待办] 更新任务清单");
    }

    #[test]
    fn todo_read_start_line_is_label() {
        assert_eq!(tool_start_line("TodoRead", "{}"), "[待办] 读取任务清单");
    }

    #[test]
    fn tool_start_line_covers_unmatched_builtins_and_mcp() {
        assert_eq!(
            tool_start_line("WebFetch", r#"{"url":"https://example.com"}"#),
            "[工具] WebFetch https://example.com"
        );
        assert_eq!(
            tool_start_line("WebSearch", r#"{"query":"tokio runtime"}"#),
            "[工具] WebSearch tokio runtime"
        );
        assert_eq!(
            tool_start_line(
                "AskUserQuestion",
                r#"{"questions":[{"prompt":"用哪种方案？"}]}"#
            ),
            "[工具] 提问 用哪种方案？"
        );
        assert_eq!(
            tool_start_line("mcp_fs_tools_list_files", r#"{"path":"/tmp"}"#),
            "[MCP工具] mcp_fs_tools_list_files /tmp"
        );
        assert_eq!(
            tool_start_line_ex(
                "mcp_fs_tools_list_files",
                r#"{"path":"/tmp"}"#,
                Some("fs.tools"),
                Some("list-files"),
            ),
            "[MCP工具] fs.tools / list-files /tmp"
        );
        assert_eq!(tool_event_title("[读取] src/main.ts"), "读取 src/main.ts");
    }

    #[test]
    fn todo_write_result_summarizes_count() {
        let output =
            "- [in_progress] 定位 TestController (medium)\n- [pending] 实现 ok 接口 (medium)";
        assert_eq!(
            tool_result_line("TodoWrite", output),
            "[工具结果] 已更新 2 项"
        );
        assert_eq!(
            tool_result_line("TodoWrite", "(no todos)"),
            "[工具结果] 已更新 0 项"
        );
        assert_eq!(
            tool_result_line("TodoWrite", "todos 必须是数组"),
            "[工具结果]\ntodos 必须是数组"
        );
    }

    #[test]
    fn todo_read_result_keeps_full_list() {
        let output =
            "- [completed] 定位 TestController (medium)\n- [in_progress] 实现 ok 接口 (medium)";
        assert_eq!(
            tool_result_line("TodoRead", output),
            "[工具结果]\n- [completed] 定位 TestController (medium)\n- [in_progress] 实现 ok 接口 (medium)"
        );
    }

    #[test]
    fn other_tool_result_keeps_full_output() {
        assert_eq!(
            tool_result_line("Read", "line1\nline2"),
            "[工具结果]\nline1\nline2"
        );
        assert_eq!(
            tool_result_line("Grep", "a.rs:3:hit\nb.rs:9:hit"),
            "[工具结果]\na.rs:3:hit\nb.rs:9:hit"
        );
    }

    #[test]
    fn tool_result_display_caps_lines_and_chars() {
        let many_lines = (0..2001)
            .map(|index| format!("L{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let line_result = tool_result_line("Read", &many_lines);
        assert!(line_result.starts_with("[工具结果]\nL0\n"));
        assert!(line_result.contains("L1999"));
        assert!(!line_result.contains("L2000"));
        assert!(line_result.contains("…（已截断，共 2001 行 / "));

        let huge = "a".repeat(TOOL_RESULT_DISPLAY_MAX_CHARS + 1);
        let char_result = tool_result_line("Bash", &huge);
        assert!(char_result.starts_with("[工具结果]\n"));
        assert!(
            char_result.contains("…（已截断，共 1 行 / 65537 字）"),
            "missing char cap notice: {}",
            char_result.chars().rev().take(40).collect::<String>()
        );
        let body = char_result
            .strip_prefix("[工具结果]\n")
            .and_then(|text| text.strip_suffix("\n…（已截断，共 1 行 / 65537 字）"))
            .expect("display wrapper");
        assert_eq!(body.chars().count(), TOOL_RESULT_DISPLAY_MAX_CHARS);
        assert!(body.chars().all(|ch| ch == 'a'));
    }

    #[tokio::test]
    async fn emits_todo_write_list() {
        let (mut runner, root) = temp_runner();
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let _ = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call(
                        "t1",
                        "TodoWrite",
                        r#"{"todos":[
                            {"content":"定位 TestController","status":"in_progress"},
                            {"content":"实现 ok 接口","status":"pending"},
                            {"content":"补测试","status":"pending"}
                        ]}"#,
                    ),
                    Message::assistant_text("done"),
                ],
            )
            .await
            .expect("run");
        let lines = drain_events(&mut rx);
        let start = lines
            .iter()
            .find(|line| line.starts_with("[待办]"))
            .expect("missing todo start");
        assert!(
            start.contains("- [in_progress] 定位 TestController (medium)")
                && start.contains("- [pending] 实现 ok 接口 (medium)")
                && start.contains("- [pending] 补测试 (medium)"),
            "todo list missing items: {start}"
        );
        assert!(
            lines.iter().any(|line| line == "[工具结果] 已更新 3 项"),
            "missing todo result count: {lines:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn child_uses_dedicated_subagent_turns() {
        let (mut runner, root) = temp_runner();
        let spec = parse_subagent_args(r#"{"prompt":"go"}"#).unwrap();
        let default_child = runner.spawn_child_runner(&spec, 1);
        assert_eq!(default_child.max_turns, 20);
        assert_eq!(default_child.max_subagent_turns, 20);

        runner.max_turns = 40;
        runner.max_subagent_turns = 80;
        let child = runner.spawn_child_runner(&spec, 2);
        assert_eq!(child.max_turns, 80);
        assert_eq!(child.max_subagent_turns, 80);

        runner.max_turns = 0;
        runner.max_subagent_turns = 20;
        let limited = runner.spawn_child_runner(&spec, 3);
        assert_eq!(limited.max_turns, 20);

        runner.max_subagent_turns = 0;
        let unlimited = runner.spawn_child_runner(&spec, 4);
        assert_eq!(unlimited.max_turns, 0);
        assert_eq!(unlimited.max_subagent_turns, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parent_has_agent_child_and_readonly_do_not() {
        let (mut runner, root) = temp_runner();
        runner.ctx.extra_env = vec![("HTTPS_PROXY".to_string(), "http://proxy".to_string())];
        assert!(runner.tool_names().iter().any(|name| name == "Agent"));
        let general = parse_subagent_args(r#"{"prompt":"go","description":"改文件"}"#).unwrap();
        let child = runner.spawn_child_runner(&general, 1);
        assert!(!child.tool_names().iter().any(|name| name == "Agent"));
        assert!(!child.ctx.is_read_only());
        assert_eq!(child.ctx.extra_env, runner.ctx.extra_env);
        assert_eq!(child.event_prefix, "[子 Agent 1(general) - 改文件] ");
        let explore = parse_subagent_args(r#"{"prompt":"go","subagent_type":"explore"}"#).unwrap();
        let explore_child = runner.spawn_child_runner(&explore, 2);
        assert!(explore_child.ctx.is_read_only());
        assert!(!explore_child
            .tool_names()
            .iter()
            .any(|name| name == "Agent"));
        assert!(!explore_child
            .tool_names()
            .iter()
            .any(|name| name == "Write"));
        let mut custom_runner = AgentRunner::new(LocalWorkspace::new(root.clone()));
        custom_runner.custom_subagents = vec![NativeSubagent {
            id: "1".to_string(),
            name: "reviewer".to_string(),
            description: "review".to_string(),
            model_mode: "inherit".to_string(),
            channel_id: None,
            model: None,
            tool_mode: "custom".to_string(),
            tools: vec!["Read".to_string(), "Grep".to_string()],
            system_prompt: "你是审查员".to_string(),
            inject_agents_md: false,
            scope: "all".to_string(),
            workspace_ids: Vec::new(),
            permission_mode: None,
            disallowed_tools: Vec::new(),
            source: "json".to_string(),
            path: None,
            max_turns: None,
            skills: Vec::new(),
        }];
        custom_runner.workspace_context = "Working directory: /repo".to_string();
        custom_runner.project_agents = "secret agents".to_string();
        let custom = parse_subagent_args_with(
            r#"{"prompt":"go","subagent_type":"reviewer","description":"审"}"#,
            &custom_runner.custom_subagents,
        )
        .unwrap();
        let custom_child = custom_runner.spawn_child_runner(&custom, 3);
        assert!(custom_child.ctx.is_read_only());
        assert!(custom_child.tool_names().iter().any(|name| name == "Read"));
        assert!(!custom_child.tool_names().iter().any(|name| name == "Write"));
        assert!(!custom_child.tool_names().iter().any(|name| name == "Agent"));
        let system = custom_child
            .messages
            .iter()
            .find(|message| message.role == Role::System)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        assert!(system.contains("你是审查员"));
        assert!(system.contains("Working directory: /repo"));
        assert!(!system.contains("secret agents"));
        let mut readonly = AgentRunner::new(LocalWorkspace::new(root.clone()));
        readonly.set_read_only(true);
        readonly.set_allowed_tools(&crate::native::tools::read_only_tool_names());
        assert!(!readonly.tool_names().iter().any(|name| name == "Agent"));
        let mut extra = AgentRunner::new(LocalWorkspace::new(root.clone()));
        extra.set_read_only(true);
        extra.set_extra_tools(vec![ToolSpec {
            name: "mcp_fs_write".to_string(),
            description: "mcp".to_string(),
            parameters: serde_json::json!({}),
        }]);
        let extra_names = extra.tool_names();
        assert!(extra_names.iter().any(|name| name == "Read"));
        assert!(!extra_names.iter().any(|name| name == "Write"));
        assert!(!extra_names.iter().any(|name| name == "Agent"));
        assert!(!extra_names.iter().any(|name| name == "mcp_fs_write"));
        extra.set_read_only(false);
        assert!(extra.tool_names().iter().any(|name| name == "Write"));
        let mut plan = AgentRunner::new(LocalWorkspace::new(root.clone()));
        plan.set_read_only(true);
        plan.set_plan_mode(true);
        let plan_names = plan.tool_names();
        assert!(plan_names.iter().any(|name| name == "AskUserQuestion"));
        assert!(plan_names.iter().any(|name| name == "ExitPlanMode"));
        assert!(!plan_names.iter().any(|name| name == "EnterPlanMode"));
        assert!(plan_names.iter().any(|name| name == "Skill"));
        assert!(!plan_names.iter().any(|name| name == "Write"));
        assert!(!plan_names.iter().any(|name| name == "ApplyPatch"));
        plan.set_plan_mode(false);
        plan.set_read_only(false);
        let exec_names = plan.tool_names();
        // 执行模式：提问工具始终可用，EnterPlanMode 可见，ExitPlanMode 隐藏。
        assert!(exec_names.iter().any(|name| name == "AskUserQuestion"));
        assert!(exec_names.iter().any(|name| name == "EnterPlanMode"));
        assert!(!exec_names.iter().any(|name| name == "ExitPlanMode"));
        // 子 Agent 没有交互通道，看不到提问与计划模式工具。
        let child_names = child.tool_names();
        assert!(!child_names.iter().any(|name| name == "AskUserQuestion"));
        assert!(!child_names.iter().any(|name| name == "EnterPlanMode"));
        runner.cancel();
        assert!(child.ctx.cancel.is_cancelled());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn child_runner_shares_rollout_budget() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(10_000);
        let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
        let child = runner.spawn_child_runner(&spec, 1);
        assert!(Arc::ptr_eq(&runner.rollout_budget, &child.rollout_budget));
        assert_eq!(child.budget_snapshot().limit, 10_000);
        assert_eq!(
            child.child_quota.as_ref().map(|quota| quota.limit()),
            Some(4_000)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn child_quota_stops_before_parent_budget() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(10_000);
        runner.subagent_budget_share_percent = 40;
        let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
        let mut child = runner.spawn_child_runner(&spec, 1);
        child.child_quota = Some(ChildQuota::shared(1));
        child.messages.push(Message::user("hello world"));
        assert!(child.reserve_model_call(Some(1_000), &[]).is_none());
        assert_eq!(runner.budget_snapshot().remaining, 10_000);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn batch_children_share_one_quota_pool() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(10_000);
        runner.subagent_budget_share_percent = 40;
        let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
        let pool = runner.child_quota_for_share();
        let first = runner.spawn_child_with_quota(&spec, 1, pool.clone());
        let second = runner.spawn_child_with_quota(&spec, 2, pool.clone());
        let third = runner.spawn_child_with_quota(&spec, 3, pool.clone());
        assert!(Arc::ptr_eq(
            first.child_quota.as_ref().unwrap(),
            second.child_quota.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            second.child_quota.as_ref().unwrap(),
            third.child_quota.as_ref().unwrap()
        ));
        assert_eq!(
            first.child_quota.as_ref().map(|quota| quota.limit()),
            Some(4_000)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn budget_exhaustion_forces_tool_free_final_turn() {
        let (mut runner, root) = temp_runner();
        // The first request estimate is deliberately larger than this budget,
        // so the scripted model must receive a tool-free final turn.
        runner.set_rollout_budget_limit(8);
        let text = runner
            .run_scripted(
                "go",
                vec![assistant_tool_call(
                    "c1",
                    "Read",
                    r#"{"file_path":"hello.txt"}"#,
                )],
            )
            .await
            .expect("budget final turn");
        assert_eq!(text, LAST_TURN_FALLBACK);
        assert!(runner.budget_snapshot().limit == 8);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn exhausted_budget_stops_without_an_extra_model_request() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(1);
        let client = ModelClient::new(crate::native::model::client::ModelClientConfig {
            protocol: crate::native::protocol::PROTOCOL_OPENAI.to_string(),
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: "test".to_string(),
            extra_headers: std::collections::HashMap::new(),
            retry: crate::native::model::RetryConfig::none(),
            timeout: Duration::from_millis(50),
            network: crate::app::network_settings::NetworkSettings::default(),
        })
        .expect("client");
        let text = runner
            .run_with_client(&client, "go", "test-model", None, None, false, Vec::new())
            .await
            .expect("budget stop");
        assert_eq!(text, LAST_TURN_FALLBACK);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn final_request_output_is_clamped_to_remaining_budget() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(300);
        runner.messages.push(Message::user("task"));
        let budget = runner
            .reserve_model_call(Some(10_000), &[])
            .expect("reservation");
        assert!(budget.max_output_tokens.expect("output cap") < 10_000);
        runner.release_model_reservation();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ample_budget_caps_unconfigured_output_to_fallback_guard() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(1_000_000);
        runner.messages.push(Message::user("task"));
        let budget = runner.reserve_model_call(None, &[]).expect("reservation");
        assert_eq!(
            budget.max_output_tokens,
            Some(FALLBACK_OUTPUT_TOKEN_GUARD as u32)
        );
        runner.release_model_reservation();
        let budget = runner
            .reserve_model_call(Some(60_000), &[])
            .expect("reservation");
        assert_eq!(budget.max_output_tokens, Some(60_000));
        runner.release_model_reservation();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_usage_settles_using_response_text() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(100_000);
        runner.messages.push(Message::user("task"));
        let _ = runner.reserve_model_call(None, &[]).expect("reserve");
        let reserved = runner.budget_snapshot().spent;
        let assistant = Message::assistant_text("x".repeat(80_000));
        runner.settle_model_usage(Usage::default(), Some(&assistant));
        assert!(
            runner.budget_snapshot().spent > reserved,
            "missing usage must charge the response text, spent={} reserved={reserved}",
            runner.budget_snapshot().spent
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn near_exhaustion_caps_unconfigured_output_to_remaining_budget() {
        let (mut runner, root) = temp_runner();
        runner.set_rollout_budget_limit(2_000);
        runner.messages.push(Message::user("task"));
        let budget = runner.reserve_model_call(None, &[]).expect("reservation");
        let cap = u64::from(budget.max_output_tokens.expect("near-exhaustion cap"));
        assert!(cap < 2_000);
        runner.release_model_reservation();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn oversized_tool_schemas_force_tool_free_final_turn() {
        let (mut runner, root) = temp_runner();
        runner.context_window.set_token_limit(32);
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        runner.messages.push(Message::system("rules"));
        runner.messages.push(Message::user("task"));
        runner.set_extra_tools(vec![ToolSpec {
            name: "mcp_large".to_string(),
            description: "schema ".repeat(512),
            parameters: serde_json::json!({"type":"object"}),
        }]);

        let last_turn = runner
            .prepare_model_call(None)
            .await
            .expect("prepare model call");
        assert!(last_turn);
        assert!(drain_events(&mut rx)
            .iter()
            .any(|line| line.contains("工具定义已超过上下文窗口")));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn large_tool_result_is_bounded_in_model_history() {
        let (mut runner, root) = temp_runner();
        runner.tool_result_token_limit = 40;
        let mut assistant = Message::assistant_text("");
        assistant.content.clear();
        assistant.tool_calls = vec![ToolCall {
            id: "c1".to_string(),
            name: "Bash".to_string(),
            arguments: r#"{"command":"printf huge"}"#.to_string(),
        }];
        // Use a scripted tool call that returns the normal shell output path;
        // direct helper coverage in truncate.rs covers the large payload.
        let _ = runner
            .run_scripted("go", vec![assistant, Message::assistant_text("done")])
            .await
            .expect("run");
        let tool_messages = runner
            .messages
            .iter()
            .filter(|message| message.role == Role::Tool)
            .collect::<Vec<_>>();
        assert!(tool_messages
            .iter()
            .all(|message| message.content.chars().count() < 200));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostics_count_tool_result_truncation() {
        let (mut runner, root) = temp_runner();
        runner.tool_result_token_limit = 16;
        let call = ToolCall {
            id: "c1".to_string(),
            name: "Read".to_string(),
            arguments: r#"{"file_path":"hello.txt"}"#.to_string(),
        };
        runner.append_tool_message(&call, ToolOutput::text("line\n".repeat(500)));
        assert_eq!(runner.diagnostics_snapshot().tool_results_truncated, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn agent_missing_prompt_keeps_loop_going() {
        let (mut runner, root) = temp_runner();
        let text = runner
            .run_scripted(
                "go",
                vec![
                    assistant_tool_call("c1", "Agent", r#"{"description":"x"}"#),
                    Message::assistant_text("ok"),
                ],
            )
            .await
            .expect("run");
        assert_eq!(text, "ok");
        assert!(runner
            .messages
            .iter()
            .any(|message| message.content.contains("prompt 不能为空")));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parallel_agent_stubs_overlap() {
        let (mut runner, root) = temp_runner();
        let (tx, mut rx) = mpsc::unbounded_channel();
        runner.on_event = Some(tx);
        let live = Arc::new(AtomicU32::new(0));
        let max = Arc::new(AtomicU32::new(0));
        let live_clone = live.clone();
        let max_clone = max.clone();
        runner.subagent_stub = Some(Arc::new(move |spec: &SubagentSpec| {
            let now = live_clone.fetch_add(1, Ordering::SeqCst) + 1;
            max_clone.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(80));
            live_clone.fetch_sub(1, Ordering::SeqCst);
            format!("done {}", spec.description)
        }));
        let mut assistant = Message::assistant_text("");
        assistant.content.clear();
        assistant.tool_calls = vec![
            ToolCall {
                id: "a1".to_string(),
                name: "Agent".to_string(),
                arguments: r#"{"description":"one","prompt":"p1"}"#.to_string(),
            },
            ToolCall {
                id: "a2".to_string(),
                name: "Agent".to_string(),
                arguments: r#"{"description":"two","prompt":"p2"}"#.to_string(),
            },
        ];
        let text = runner
            .run_scripted(
                "go",
                vec![assistant, Message::assistant_text("parent done")],
            )
            .await
            .expect("run");
        assert_eq!(text, "parent done");
        assert!(
            max.load(Ordering::SeqCst) >= 2,
            "expected overlapping stubs, max={}",
            max.load(Ordering::SeqCst)
        );
        let lines = drain_events(&mut rx);
        assert!(lines
            .iter()
            .any(|line| line.contains("[子 Agent 1(general) - one] 启动")));
        assert!(lines
            .iter()
            .any(|line| line.contains("[子 Agent 2(general) - two] 启动")));
        assert!(runner
            .messages
            .iter()
            .any(|message| message.content.contains("done one")));
        assert!(runner
            .messages
            .iter()
            .any(|message| message.content.contains("done two")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn agent_tool_description_uses_runner_cap() {
        let (mut runner, root) = temp_runner();
        runner.max_concurrent_subagents = 4;
        runner.subagent_policy = "aggressive".to_string();
        let agent = runner
            .combined_tools()
            .into_iter()
            .find(|tool| tool.name == "Agent")
            .expect("Agent tool");
        assert!(
            agent.description.contains("max 4"),
            "description should include cap: {}",
            agent.description
        );
        assert!(agent.description.contains("Policy aggressive"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subagent_cap_one_does_not_overlap() {
        let (mut runner, root) = temp_runner();
        runner.max_concurrent_subagents = 1;
        let live = Arc::new(AtomicU32::new(0));
        let max = Arc::new(AtomicU32::new(0));
        let live_clone = live.clone();
        let max_clone = max.clone();
        runner.subagent_stub = Some(Arc::new(move |spec: &SubagentSpec| {
            let now = live_clone.fetch_add(1, Ordering::SeqCst) + 1;
            max_clone.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            live_clone.fetch_sub(1, Ordering::SeqCst);
            format!("done {}", spec.description)
        }));
        let mut assistant = Message::assistant_text("");
        assistant.content.clear();
        assistant.tool_calls = vec![
            ToolCall {
                id: "a1".to_string(),
                name: "Agent".to_string(),
                arguments: r#"{"description":"one","prompt":"p1"}"#.to_string(),
            },
            ToolCall {
                id: "a2".to_string(),
                name: "Agent".to_string(),
                arguments: r#"{"description":"two","prompt":"p2"}"#.to_string(),
            },
        ];
        let _ = runner
            .run_scripted(
                "go",
                vec![assistant, Message::assistant_text("parent done")],
            )
            .await
            .expect("run");
        assert_eq!(max.load(Ordering::SeqCst), 1, "cap 1 should not overlap");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn explore_child_rejects_write() {
        let (runner, root) = temp_runner();
        let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
        let child = runner.spawn_child_runner(&spec, 1);
        child.ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        let error = crate::native::tools::execute_tool(
            &child.ctx,
            "Write",
            r#"{"file_path":"hello.txt","content":"changed"}"#,
        )
        .await
        .expect_err("explore write");
        assert!(error.contains("只读规划模式禁止调用工具 Write"));
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).expect("read"),
            "hello world\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compact_and_child_clients_inherit_session_scope() {
        let (runner, root) = temp_runner();
        let parent = ModelClient::new(crate::native::model::ModelClientConfig {
            protocol: "openai".to_string(),
            base_url: "http://127.0.0.1".to_string(),
            api_key: "sk-test".to_string(),
            extra_headers: std::collections::HashMap::new(),
            retry: crate::native::model::RetryConfig::none(),
            timeout: Duration::from_secs(1),
            network: crate::app::network_settings::NetworkSettings::default(),
        })
        .expect("client")
        .with_call_log_context(
            crate::native::model::call_log::CallLogContext::for_session(
                Some("ch-1".to_string()),
                Some("OpenAI".to_string()),
                Some("sess-1".to_string()),
                Some("emp-1".to_string()),
                Some("proj-ssh".to_string()),
                crate::native::model::call_log::CALL_KIND_CHAT,
                Some("ssh".to_string()),
            ),
        );
        let compact = parent.clone_for_conversation().with_call_log_context(
            parent
                .call_log_context()
                .cloned()
                .unwrap_or_default()
                .with_call_kind(CALL_KIND_COMPACT),
        );
        let compact_ctx = compact.call_log_context().expect("compact context");
        assert_eq!(compact_ctx.call_kind.as_deref(), Some(CALL_KIND_COMPACT));
        assert_eq!(compact_ctx.session_id.as_deref(), Some("sess-1"));
        assert_eq!(compact_ctx.workspace_id.as_deref(), Some("proj-ssh"));
        assert_eq!(compact_ctx.execution_target.as_deref(), Some("ssh"));

        let spec = parse_subagent_args(r#"{"prompt":"look","subagent_type":"explore"}"#).unwrap();
        let child = runner.observe_child_client(Some(&parent), &parent, Some(&spec));
        let child_ctx = child.call_log_context().expect("child context");
        assert_eq!(child_ctx.call_kind.as_deref(), Some(CALL_KIND_SUBAGENT));
        assert_eq!(child_ctx.session_id.as_deref(), Some("sess-1"));
        assert_eq!(child_ctx.workspace_id.as_deref(), Some("proj-ssh"));
        assert_eq!(child_ctx.execution_target.as_deref(), Some("ssh"));
        assert_eq!(child_ctx.subagent_id.as_deref(), Some("explore"));
        let _ = fs::remove_dir_all(root);
    }
}
