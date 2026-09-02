use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::engine::UsageDelta;
use crate::native::manager::NativeFollowup;
use crate::native::model::call_log::{CALL_KIND_COMPACT, CALL_KIND_SUBAGENT};
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
use crate::native::tools::{
    ask_question_spec, execute_tool, tool_specs, CancelFlag, LocalWorkspace, SharedMcp, ToolCtx,
};

use super::compact::{
    compact_local, compact_with_summary, compaction_prompt, is_usable_compaction_summary,
    reset_local, BudgetSnapshot, ChildQuota, ContextWindow, RolloutBudget,
};
use super::subagent::{
    child_system_prompt, custom_child_system_prompt, format_subagent_log_tag,
    format_subagent_result, parse_subagent_args_with, SubagentKind, SubagentSpec,
};
use super::truncate::{
    chars_to_tokens, message_tokens, total_message_tokens, total_tool_tokens,
    truncate_messages_tokens, truncate_tool_result, DEFAULT_TOOL_RESULT_TOKEN_LIMIT,
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
    pub subagent_stub: Option<SubagentStub>,
    pub custom_subagents: Vec<NativeSubagent>,
    pub reload_custom_subagents: Option<CustomSubagentReloader>,
    pub child_model_loader: Option<ChildModelLoader>,
    pub workspace_context: String,
    pub project_agents: String,
    pub required_subagent_type: Option<String>,
    extra_tools: Vec<ToolSpec>,
    allowed_tools: Option<HashSet<String>>,
    plan_mode: bool,
    turns: u32,
    last_tool_key: Option<String>,
    last_tool_repeat: u32,
    depth: u8,
    event_prefix: String,
    subagent_seq: u32,
    model_turn: Option<ModelTurnCfg>,
    pending_budget_reservation: u64,
    pending_child_reservation: u64,
    pending_steer_finish: bool,
    budget_exhausted: bool,
    streaming: bool,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeEvent {
    Line(String),
    Delta(StreamDelta),
    ContextUsage(ContextUsageSnapshot),
}

impl AgentRunner {
    pub fn new(workspace: LocalWorkspace) -> Self {
        Self {
            ctx: ToolCtx {
                workspace,
                ssh: None,
                cancel: CancelFlag::new(),
                read_files: HashSet::new(),
                todos: Vec::new(),
                mcp: SharedMcp::empty(),
                allow_all_high_risk: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                allowed_mcp_servers: std::sync::Arc::new(std::sync::Mutex::new(HashSet::new())),
                request_permission: None,
                expire_permission: None,
                permission_timeout: std::time::Duration::ZERO,
                request_question: None,
                read_only: false,
                skills: Vec::new(),
                hooks: Vec::new(),
                on_mutation: None,
            },
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
            subagent_stub: None,
            custom_subagents: Vec::new(),
            reload_custom_subagents: None,
            child_model_loader: None,
            workspace_context: String::new(),
            project_agents: String::new(),
            required_subagent_type: None,
            extra_tools: Vec::new(),
            allowed_tools: None,
            plan_mode: false,
            turns: 0,
            last_tool_key: None,
            last_tool_repeat: 0,
            depth: 0,
            event_prefix: String::new(),
            subagent_seq: 0,
            model_turn: None,
            pending_budget_reservation: 0,
            pending_child_reservation: 0,
            pending_steer_finish: false,
            budget_exhausted: false,
            streaming: false,
        }
    }

    pub fn cancel(&self) {
        self.ctx.cancel.cancel();
    }

    pub fn set_extra_tools(&mut self, tools: Vec<ToolSpec>) {
        self.extra_tools = tools;
    }

    pub fn set_allowed_tools(&mut self, names: &[&str]) {
        self.allowed_tools = Some(names.iter().map(|name| (*name).to_string()).collect());
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.ctx.read_only = read_only;
    }

    pub fn set_plan_mode(&mut self, plan_mode: bool) {
        self.plan_mode = plan_mode;
    }

    pub fn take_steer_finish(&mut self) -> bool {
        std::mem::take(&mut self.pending_steer_finish)
    }

    fn inject_steer_messages(&mut self) {
        let Some(rx) = self.steer_rx.clone() else {
            return;
        };
        let Ok(mut guard) = rx.try_lock() else {
            return;
        };
        while let Ok(item) = guard.try_recv() {
            match item {
                NativeFollowup::Input(text) => {
                    self.emit(format!("[USER_INPUT] {text}"));
                    self.messages.push(Message::user(text));
                }
                NativeFollowup::Finish => {
                    self.pending_steer_finish = true;
                }
            }
        }
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
        if self.depth > 0 || self.ctx.read_only {
            tools.retain(|tool| tool.name != "Agent");
        }
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
        if self.ctx.read_only {
            tools.retain(|tool| crate::native::tools::is_read_only_native_tool(&tool.name));
        }
        if self.plan_mode && !tools.iter().any(|tool| tool.name == "AskQuestion") {
            tools.push(ask_question_spec());
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

    fn observe_client(&self, client: &ModelClient) -> ModelClient {
        let on_event = self.on_event.clone();
        let prefix = self.event_prefix.clone();
        let delta_events = self.on_event.clone();
        client
            .clone_for_conversation()
            .with_cancel(self.ctx.cancel.clone())
            .with_retry_hook(Arc::new(move |line: &str| {
                Self::send_prefixed_event(&on_event, &prefix, line);
            }))
            // Only the top-level runner streams: concurrent child agents would
            // interleave their fragments into one unreadable line.
            .with_delta_hook(Arc::new(move |delta: StreamDelta| {
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
            .with_call_kind(CALL_KIND_SUBAGENT);
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

    fn emit_usage(&self, usage: crate::native::model::types::Usage) {
        let Some(delta) = usage_to_delta(usage) else {
            return;
        };
        if let Some(line) = delta.format_terminal_line() {
            self.emit(line);
        }
        if let Some(tx) = &self.on_usage {
            let _ = tx.send(delta);
        }
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
        self.begin_user_turn(user, images)?;
        let client = self.observe_client(client);
        self.streaming = true;
        loop {
            self.inject_steer_messages();
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
        self.begin_user_turn(user, Vec::new())?;
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
        self.begin_user_turn(user, Vec::new())?;
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

    fn begin_user_turn(&mut self, user: &str, images: Vec<NativeImage>) -> Result<(), String> {
        if self.ctx.cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        self.turns = 0;
        self.last_tool_key = None;
        self.last_tool_repeat = 0;
        self.messages.push(Message::user_with_images(user, images));
        Ok(())
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
        if self.context_window.should_compact(&self.messages) {
            let mut compacted = false;
            if let Some(client) = client {
                compacted = self.compact_with_model(client).await;
                if compacted {
                    self.context_window.mark_compacted();
                    self.emit("[工具] 已压缩上下文（模型摘要）");
                }
            }
            if !compacted && compact_local(&mut self.messages) {
                self.context_window.mark_compacted();
                self.emit("[工具] 已压缩上下文（本地摘要）");
                compacted = true;
            }
            if !compacted && reset_local(&mut self.messages) {
                self.context_window.mark_reset();
                self.emit("[工具] 已重置上下文窗口（保留当前任务）");
            }
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
        if let Some(tx) = &self.on_event {
            let _ = tx.send(NativeEvent::ContextUsage(ContextUsageSnapshot {
                used_tokens: total_message_tokens(&self.messages),
                limit_tokens: self.context_window.token_limit,
                generation: self.context_window.generation,
                compactions: self.context_window.compactions,
            }));
        }
    }

    async fn compact_with_model(&mut self, client: &ModelClient) -> bool {
        let Some(summary_messages) = compaction_prompt(&self.messages) else {
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
        let summary_model = self
            .model_turn
            .as_ref()
            .map(|turn| turn.model.clone())
            .unwrap_or_default();
        let compact_client = match client.call_log_context() {
            Some(context) => client
                .clone_for_conversation()
                .with_call_log_context(context.clone().with_call_kind(CALL_KIND_COMPACT)),
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
        if !assistant.reasoning_content.is_empty() {
            let chars = assistant.reasoning_content.chars().count();
            self.emit(format!("[思考] 已生成 {chars} 字"));
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
        self.execute_tool_calls(tool_calls, client).await?;
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
        if !assistant.reasoning_content.is_empty() {
            let chars = assistant.reasoning_content.chars().count();
            self.emit(format!("[思考] 已生成 {chars} 字"));
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
        for call in tool_calls {
            if self.ctx.cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            if call.name == "Agent" {
                self.push_tool_output(&call, "子 Agent 不能再委派子 Agent".to_string());
                continue;
            }
            self.emit(tool_start_line(&call.name, &call.arguments));
            let output = self.execute_logged_tool(&call).await;
            self.emit(tool_result_line(&call.name, &output));
            self.append_tool_message(&call, output);
        }
        Ok(TurnControl::Continue)
    }

    async fn execute_logged_tool(&mut self, call: &ToolCall) -> String {
        let key = format!("{}\n{}", call.name, call.arguments);
        if self.last_tool_key.as_deref() == Some(key.as_str()) {
            self.last_tool_repeat = self.last_tool_repeat.saturating_add(1);
        } else {
            self.last_tool_key = Some(key);
            self.last_tool_repeat = 1;
        }
        if self.last_tool_repeat >= REPEAT_TOOL_LIMIT {
            return format!(
                "重复调用被拒绝：你已用相同参数连续调用 {} {} 次。请改用其他工具或直接给出最终结论。",
                call.name, self.last_tool_repeat
            );
        }
        match execute_tool(&mut self.ctx, &call.name, &call.arguments).await {
            Ok(value) => value,
            Err(error) => error,
        }
    }

    async fn execute_tool_calls(
        &mut self,
        calls: Vec<ToolCall>,
        client: Option<&ModelClient>,
    ) -> Result<(), String> {
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
            } else {
                let call = &calls[index];
                self.emit(tool_start_line(&call.name, &call.arguments));
                let output = self.execute_logged_tool(call).await;
                self.emit(tool_result_line(&call.name, &output));
                self.append_tool_message(call, output);
                index += 1;
            }
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
        child.ctx.ssh = self.ctx.ssh.clone();
        child.ctx.cancel = self.ctx.cancel.clone();
        child.ctx.allow_all_high_risk = self.ctx.allow_all_high_risk.clone();
        child.ctx.allowed_mcp_servers = self.ctx.allowed_mcp_servers.clone();
        child.ctx.request_permission = self.ctx.request_permission.clone();
        child.ctx.expire_permission = self.ctx.expire_permission.clone();
        child.ctx.permission_timeout = self.ctx.permission_timeout;
        child.ctx.skills = self.ctx.skills.clone();
        child.ctx.hooks = self.ctx.hooks.clone();
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
                child.ctx.read_only = true;
                child.set_allowed_tools(crate::native::tools::READ_ONLY_NATIVE_TOOL_NAMES);
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
                            child.ctx.read_only = true;
                        }
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
                let output = "子 Agent 不能再委派子 Agent".to_string();
                self.push_tool_output(call, output);
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
            self.emit(tool_start_line("Agent", &job.call.arguments));
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
            join_set.spawn(async move {
                let outcome = async {
                    let _permit = permit
                        .acquire_owned()
                        .await
                        .map_err(|_| "子 Agent 并发许可已关闭".to_string())?;
                    if let Some(stub) = stub {
                        Ok(stub(&job.spec))
                    } else if let Some((channel_id, model)) = custom_override {
                        let Some(loader) = child_model_loader else {
                            return Err("子 Agent 需要模型客户端".to_string());
                        };
                        let settings = loader(channel_id, model).await?;
                        child
                            .run_child_with_client(
                                client_owned.as_ref(),
                                &settings.client,
                                &job.spec.prompt,
                                &settings.model,
                                settings.effort.as_deref(),
                                settings.max_output_tokens,
                                settings.thinking_enabled,
                                Some(&job.spec),
                            )
                            .await
                    } else if let (Some(client), Some(cfg)) = (client_owned.as_ref(), model_turn) {
                        child
                            .run_child_with_client(
                                Some(client),
                                client,
                                &job.spec.prompt,
                                &cfg.model,
                                cfg.effort.as_deref(),
                                cfg.max_output_tokens,
                                cfg.thinking_enabled,
                                Some(&job.spec),
                            )
                            .await
                    } else {
                        Err("子 Agent 需要模型客户端".to_string())
                    }
                }
                .await;
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
            self.push_tool_output(&item.0, item.1);
        }
        Ok(())
    }

    fn push_tool_output(&mut self, call: &ToolCall, output: String) {
        if !output.starts_with("子 Agent（") {
            self.emit(tool_start_line(&call.name, &call.arguments));
        }
        self.emit(tool_result_line(&call.name, &output));
        self.append_tool_message(call, output);
    }

    fn append_tool_message(&mut self, call: &ToolCall, output: String) {
        let bounded = truncate_tool_result(
            &call.name,
            &call.arguments,
            &output,
            self.tool_result_token_limit,
        );
        if bounded != output {
            self.diagnostics
                .tool_results_truncated
                .fetch_add(1, Ordering::AcqRel);
        }
        let mut message = Message::tool_result(&call.id, bounded);
        message.name = call.name.clone();
        self.messages.push(message);
    }
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
    if let Some(last) = messages.last_mut() {
        if last.role == Role::User || last.role == Role::Tool {
            if !last.content.is_empty() {
                last.content.push_str("\n\n");
            }
            last.content.push_str(LAST_TURN_REMINDER);
            return;
        }
    }
    messages.push(Message::user(LAST_TURN_REMINDER.to_string()));
}

fn tool_start_line(name: &str, arguments: &str) -> String {
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
        other => format!("[工具] {other}"),
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
            if let NativeEvent::Line(line) = event {
                lines.push(line);
            }
        }
        lines
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
        runner.set_allowed_tools(crate::native::tools::READ_ONLY_NATIVE_TOOL_NAMES);
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
        let (runner, root) = temp_runner();
        assert!(runner.tool_names().iter().any(|name| name == "Agent"));
        let general = parse_subagent_args(r#"{"prompt":"go","description":"改文件"}"#).unwrap();
        let child = runner.spawn_child_runner(&general, 1);
        assert!(!child.tool_names().iter().any(|name| name == "Agent"));
        assert!(!child.ctx.read_only);
        assert_eq!(child.event_prefix, "[子 Agent 1(general) - 改文件] ");
        let explore = parse_subagent_args(r#"{"prompt":"go","subagent_type":"explore"}"#).unwrap();
        let explore_child = runner.spawn_child_runner(&explore, 2);
        assert!(explore_child.ctx.read_only);
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
        }];
        custom_runner.workspace_context = "Working directory: /repo".to_string();
        custom_runner.project_agents = "secret agents".to_string();
        let custom = parse_subagent_args_with(
            r#"{"prompt":"go","subagent_type":"reviewer","description":"审"}"#,
            &custom_runner.custom_subagents,
        )
        .unwrap();
        let custom_child = custom_runner.spawn_child_runner(&custom, 3);
        assert!(custom_child.ctx.read_only);
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
        readonly.set_allowed_tools(crate::native::tools::READ_ONLY_NATIVE_TOOL_NAMES);
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
        assert!(plan_names.iter().any(|name| name == "AskQuestion"));
        assert!(plan_names.iter().any(|name| name == "Skill"));
        assert!(!plan_names.iter().any(|name| name == "Write"));
        assert!(!plan_names.iter().any(|name| name == "ApplyPatch"));
        plan.set_plan_mode(false);
        assert!(!plan.tool_names().iter().any(|name| name == "AskQuestion"));
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
        runner.append_tool_message(&call, "line\n".repeat(500));
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
        let mut child = runner.spawn_child_runner(&spec, 1);
        child.ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        let error = crate::native::tools::execute_tool(
            &mut child.ctx,
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
