use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::db::models::{NativeAssistantFragment, NativeToolEvent, NativeToolPhase};
use crate::engine::UsageDelta;
use crate::native::artifacts::{bound_with_artifact, ArtifactStore};
use crate::native::live_model::{LiveModelSnapshot, SharedLiveModel};
use crate::native::manager::{NativeCompactionRequest, NativeFollowup};
use crate::native::model::call_log::{
    CALL_KIND_COMPACT, CALL_KIND_SUBAGENT, MODEL_ROLE_LITE, MODEL_ROLE_MAIN, OPERATION_COMPACT,
    OPERATION_SUBAGENT,
};
use crate::native::model::client::{ChatRequest, ModelClient};
use crate::native::model::response::{FinishReason, ModelError, ModelErrorKind, ModelResponse};
use crate::native::model::types::{
    Message, NativeImage, Role, StreamDelta, ToolCall, ToolSpec, Usage,
};
use crate::native::model::usage_to_delta;
use crate::native::recovery::{PlannedCall, RecoveryState, RecoveryStep, UNKNOWN_RESULT};
use crate::native::settings::DEFAULT_NATIVE_MAX_TURNS;
use crate::native::subagents::{
    custom_tools_are_read_only, effective_custom_tools, find_native_subagent, ChildModelSettings,
    NativeSubagent, MODEL_MODE_CHANNEL, TOOL_MODE_ALL,
};
use crate::native::tools::contract::ContractRegistry;
use crate::native::tools::dispatch::{finalize_tool, preflight_tool, PreparedTool};
use crate::native::tools::hooks::{run_stop_hooks, run_user_prompt_submit_hooks};
use crate::native::tools::{
    execute_tool_call, read_only_tool_names_with_bash, tool_contracts, tool_specs, LocalWorkspace,
    ToolContract, ToolCtx, ToolOutput,
};

use self::format::*;

use super::background::BackgroundTaskRegistry;
use super::compact::{
    compact_local, compact_with_summary, compaction_gained, compaction_prompt_with_instructions,
    dropped_messages, inject_preserved_context, is_usable_compaction_summary, microcompact,
    output_reserve_for, preserved_context_block, reset_local, BudgetSnapshot, ChildQuota,
    CompactBoundary, CompactOutcome, CompactTrigger, ContextWindow, PreservedState, RolloutBudget,
};
use super::subagent::{
    child_system_prompt, custom_child_system_prompt, format_subagent_log_tag,
    format_subagent_result, parse_subagent_args_with, truncate_report, SubagentKind, SubagentSpec,
};
use super::truncate::{
    chars_to_tokens, context_usage_breakdown, message_tokens, sanitize_committed_tool_pairs,
    total_message_tokens, total_tool_tokens, truncate_messages_tokens, truncate_tool_result,
    DEFAULT_TOOL_RESULT_TOKEN_LIMIT,
};
const DEFAULT_CONTEXT_CHARS: usize = 120_000;
/// 自动压缩连续失败或无收益的上限，达到后暂停自动压缩和摘要模型调用。
const MAX_COMPACT_FAILURES: u32 = 3;
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
const OUTPUT_RECOVERY_REMINDER: &str = "上一条模型输出达到输出 token 上限，尚未完成。从中断处继续，不要重复已有内容。未完成的工具调用已丢弃；如仍需要工具，重新提供完整调用。";
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

/// 最近一次主模型调用的服务端用量，以及当时请求的形状。
/// 请求前缀、模型或压缩代数变化后就不再可信。
#[derive(Debug, Clone)]
struct UsageBaseline {
    prompt_tokens: usize,
    message_count: usize,
    prefix_tokens: usize,
    tool_tokens: usize,
    generation: u32,
    model: String,
}

type SubagentStub = Arc<dyn Fn(&SubagentSpec) -> String + Send + Sync>;
pub(crate) type CustomSubagentReloader = Arc<dyn Fn() -> Vec<NativeSubagent> + Send + Sync>;
pub(crate) type ChildModelLoader = Arc<
    dyn Fn(
            String,
            String,
            Option<String>,
        ) -> Pin<Box<dyn Future<Output = Result<ChildModelSettings, String>> + Send>>
        + Send
        + Sync,
>;
pub(crate) type TranscriptCheckpoint = Arc<
    dyn Fn(Vec<Message>) -> Pin<Box<dyn Future<Output = Result<Vec<Message>, String>> + Send>>
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
    /// Initialized on first delegation so callers can configure the public cap before starting.
    subagent_semaphore: Option<Arc<Semaphore>>,
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
    /// 自定义子 Agent 持久记忆的根目录；会话层按工作区计算。
    pub agent_memory_roots: Option<crate::native::subagents::AgentMemoryRoots>,
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
    usage_baseline: Option<UsageBaseline>,
    allowed_tools: Option<HashSet<String>>,
    disallowed_tools: Option<HashSet<String>>,
    turns: u32,
    last_tool_key: Option<String>,
    last_tool_repeat: u32,
    stop_hook_continues: u32,
    output_continuations: u32,
    output_partial: String,
    output_pending: bool,
    output_fragment: Option<NativeAssistantFragment>,
    /// `/compact [指令]` 请求，下一次模型调用前执行。
    pending_manual_compact: Option<NativeCompactionRequest>,
    /// 下一条用户消息末尾要附加的文本（记忆回忆等），用后即清。
    turn_suffix: Option<String>,
    /// 会话恢复到更小窗口的模型时置位，超阈值即以 downshift 触发压缩。
    pending_downshift_compact: bool,
    /// 全量压缩前先尝试微压缩（替换旧工具结果）。
    microcompact_enabled: bool,
    /// 本回合内因供应商溢出而做的被动压缩次数（上限 2）。
    reactive_compactions: u32,
    /// 非手动压缩连续失败 / 无收益的次数。
    compact_failures: u32,
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
    pub live_model: Option<SharedLiveModel>,
    live_model_revision: u64,
    pub recovery: Option<Arc<RecoveryState>>,
    pending_recovery: Vec<RecoveryStep>,
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
    /// 压缩判断使用的估算值（含工具定义）。
    pub estimated_tokens: usize,
    /// `provider`：以服务端用量为基线；`estimate`：纯本地保守估算。
    pub estimate_source: &'static str,
    pub output_reserve_tokens: usize,
}

#[derive(Debug)]
pub enum NativeEvent {
    TurnIdentity {
        instance_id: String,
        turn_id: String,
    },
    Flush(tokio::sync::oneshot::Sender<()>),
    Line(String),
    Assistant {
        text: String,
        fragment: NativeAssistantFragment,
    },
    ModelCall(NativeAssistantFragment),
    UserInput {
        text: String,
        images: Vec<NativeImage>,
    },
    Delta(StreamDelta),
    ContextUsage(ContextUsageSnapshot),
    Tool {
        line: String,
        event: NativeToolEvent,
        images: Vec<NativeImage>,
    },
}

mod context;
mod events;
mod format;
mod recovery;
mod runner;
mod steer;
mod subagents;
#[cfg(test)]
mod tests;
mod tools;

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
        media: Vec::new(),
        history_id: String::new(),
    }
}
