use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::native::model::types::{Message, Role, Usage};

use super::truncate::{message_chars, total_message_tokens};

const COMPACT_THRESHOLD_PERCENT: usize = 85;
const PREVIEW_CHARS: usize = 800;
const SUMMARY_KEEP: usize = 12;
const HANDOFF_CHARS: usize = 2_000;
const HANDOFF_TOTAL_CHARS: usize = 24_000;
const RESET_KEEP_MESSAGES: usize = 4;

/// A shared token budget for a parent rollout and all of its child agents.
/// `0` means unlimited, which preserves compatibility with callers that did
/// not configure a budget before this feature was introduced.
#[derive(Debug)]
pub struct RolloutBudget {
    limit: AtomicU64,
    spent: AtomicU64,
    active_reservations: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub limit: u64,
    pub spent: u64,
    pub remaining: u64,
    pub active_reservations: u64,
}

impl RolloutBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit: AtomicU64::new(limit),
            spent: AtomicU64::new(0),
            active_reservations: AtomicU64::new(0),
        }
    }

    pub fn shared(limit: u64) -> Arc<Self> {
        Arc::new(Self::new(limit))
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Acquire)
    }

    pub fn set_limit(&self, limit: u64) {
        self.limit.store(limit, Ordering::Release);
    }

    pub fn spent(&self) -> u64 {
        self.spent.load(Ordering::Acquire)
    }

    pub fn remaining(&self) -> u64 {
        let limit = self.limit();
        if limit == 0 {
            return u64::MAX;
        }
        limit.saturating_sub(self.spent())
    }

    pub fn is_exhausted(&self) -> bool {
        let limit = self.limit();
        limit > 0 && self.spent() >= limit
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        let limit = self.limit();
        BudgetSnapshot {
            limit,
            spent: self.spent(),
            remaining: self.remaining(),
            active_reservations: self.active_reservations.load(Ordering::Acquire),
        }
    }

    /// Reserve an estimated request cost. Reservations are atomic so parallel
    /// child agents cannot all pass a stale remaining-budget check.
    pub fn try_reserve(&self, tokens: u64) -> bool {
        if tokens == 0 {
            return true;
        }
        let limit = self.limit();
        if limit == 0 {
            // Unlimited rollouts still accumulate usage for diagnostics. Keep
            // the estimate in `spent` until the request is settled so a child
            // reservation is visible while it is in flight.
            self.spent.fetch_add(tokens, Ordering::AcqRel);
            self.active_reservations.fetch_add(tokens, Ordering::AcqRel);
            return true;
        }
        let mut current = self.spent.load(Ordering::Acquire);
        loop {
            if tokens > limit || current > limit.saturating_sub(tokens) {
                return false;
            }
            match self.spent.compare_exchange_weak(
                current,
                current.saturating_add(tokens),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.active_reservations.fetch_add(tokens, Ordering::AcqRel);
                    return true;
                }
                Err(next) => current = next,
            }
        }
    }

    /// Replace a request reservation with actual usage. If the provider does
    /// not return usage, the estimate remains charged as a conservative guard.
    pub fn settle(&self, reserved: u64, actual: Option<u64>) {
        if reserved == 0 {
            return;
        }
        let actual = actual.unwrap_or(reserved);
        decrement_atomic(&self.active_reservations, reserved);
        let mut current = self.spent.load(Ordering::Acquire);
        loop {
            let next = current.saturating_sub(reserved).saturating_add(actual);
            match self.spent.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(value) => current = value,
            }
        }
    }

    pub fn release(&self, reserved: u64) {
        if reserved == 0 {
            return;
        }
        decrement_atomic(&self.active_reservations, reserved);
        self.spent
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_sub(reserved))
            })
            .ok();
    }

    pub fn record_usage(&self, usage: Usage) {
        let tokens =
            u64::from(usage.prompt_tokens).saturating_add(u64::from(usage.completion_tokens));
        if tokens == 0 {
            return;
        }
        self.spent.fetch_add(tokens, Ordering::AcqRel);
    }
}

/// Per-child cap on a shared parent rollout budget. Limit `0` means the child
/// cannot spend any further tokens (unlike [`RolloutBudget`], where `0` is
/// unlimited).
#[derive(Debug)]
pub struct ChildQuota {
    limit: u64,
    spent: AtomicU64,
}

impl ChildQuota {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            spent: AtomicU64::new(0),
        }
    }

    pub fn shared(limit: u64) -> Arc<Self> {
        Arc::new(Self::new(limit))
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn remaining(&self) -> u64 {
        self.limit
            .saturating_sub(self.spent.load(Ordering::Acquire))
    }

    pub fn try_reserve(&self, tokens: u64) -> bool {
        if tokens == 0 {
            return true;
        }
        if self.limit == 0 {
            return false;
        }
        let mut current = self.spent.load(Ordering::Acquire);
        loop {
            if tokens > self.limit || current > self.limit.saturating_sub(tokens) {
                return false;
            }
            match self.spent.compare_exchange_weak(
                current,
                current.saturating_add(tokens),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }

    pub fn settle(&self, reserved: u64, actual: Option<u64>) {
        if reserved == 0 {
            return;
        }
        let actual = actual.unwrap_or(reserved);
        let mut current = self.spent.load(Ordering::Acquire);
        loop {
            let next = current.saturating_sub(reserved).saturating_add(actual);
            match self.spent.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(value) => current = value,
            }
        }
    }

    pub fn release(&self, reserved: u64) {
        if reserved == 0 {
            return;
        }
        self.spent
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(value.saturating_sub(reserved))
            })
            .ok();
    }
}

fn decrement_atomic(value: &AtomicU64, amount: u64) {
    if amount == 0 {
        return;
    }
    value
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_sub(amount))
        })
        .ok();
}

/// 触发压缩的原因，写入压缩边界记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    /// 超过阈值自动压缩。
    Auto,
    /// 用户 `/compact`。
    Manual,
    /// 供应商报上下文溢出后被动压缩再重试。
    Reactive,
    /// 会话恢复到更小上下文窗口的模型。
    Downshift,
}

impl CompactTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Reactive => "reactive",
            Self::Downshift => "downshift",
        }
    }
}

/// 一次压缩的结果：替换了消息但没有明显缩减时记为 `NoGain`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactOutcome {
    #[default]
    Success,
    NoGain,
    Failed,
}

/// 压缩后仍保留超过 95% 的 token 视为无收益。
pub fn compaction_gained(pre_tokens: usize, post_tokens: usize) -> bool {
    post_tokens.saturating_mul(100) < pre_tokens.saturating_mul(95)
}

/// 一次压缩的结果记录（对齐 ZCode 的 compactBoundary）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactBoundary {
    pub trigger: CompactTrigger,
    /// `microcompact` / `model` / `local` / `reset`。
    pub source: String,
    /// 旧记录没有该字段，按成功读取。
    #[serde(default)]
    pub outcome: CompactOutcome,
    pub pre_tokens: usize,
    pub post_tokens: usize,
    pub pre_messages: usize,
    pub post_messages: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

pub const COMPACT_BOUNDARY_PREFIX: &str = "[COMPACT_BOUNDARY] ";

impl CompactBoundary {
    /// 写进事件流的一行：前端解析成压缩边界行。
    pub fn line(&self) -> String {
        format!(
            "{COMPACT_BOUNDARY_PREFIX}{}",
            serde_json::to_string(self).unwrap_or_default()
        )
    }

    pub fn parse_line(line: &str) -> Option<Self> {
        let json = line
            .trim()
            .strip_prefix(COMPACT_BOUNDARY_PREFIX.trim_end())?;
        serde_json::from_str(json.trim()).ok()
    }
}

/// 供应商上下文溢出错误的启发式识别；实现放在模型层供错误分类共用。
pub fn is_context_overflow_error(error: &str) -> bool {
    crate::native::model::response::is_context_overflow_message(error)
}

/// State for a logical model context window. A new generation is used after
/// local compaction/reset, making it possible to diagnose repeated input
/// growth without retaining the old messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextWindow {
    pub generation: u32,
    pub token_limit: usize,
    pub compactions: u32,
    pub resets: u32,
    /// 自动压缩阈值（占窗口的百分比）。
    pub threshold_percent: usize,
    /// 为模型回答预留的 token，触发线不会高于 `token_limit - output_reserve`。
    pub output_reserve: usize,
}

impl ContextWindow {
    pub fn new(token_limit: usize) -> Self {
        Self {
            generation: 0,
            token_limit,
            compactions: 0,
            resets: 0,
            threshold_percent: COMPACT_THRESHOLD_PERCENT,
            output_reserve: 0,
        }
    }

    pub fn set_token_limit(&mut self, token_limit: usize) {
        self.token_limit = token_limit;
    }

    pub fn set_threshold_percent(&mut self, percent: usize) {
        self.threshold_percent = percent.clamp(30, 99);
    }

    pub fn set_output_reserve(&mut self, tokens: usize) {
        self.output_reserve = tokens;
    }

    /// 只用本地估算判断；运行中的 Agent 应使用带用量基线的估算。
    pub fn should_compact(&self, messages: &[Message]) -> bool {
        self.should_compact_used(total_message_tokens(messages))
    }

    pub fn should_compact_used(&self, used_tokens: usize) -> bool {
        self.token_limit > 0 && used_tokens >= self.trigger_tokens()
    }

    pub fn threshold_tokens(&self) -> usize {
        self.token_limit.saturating_mul(self.threshold_percent) / 100
    }

    /// 实际触发线：阈值线与「窗口减输出预留」取较低者。
    pub fn trigger_tokens(&self) -> usize {
        self.threshold_tokens()
            .min(self.token_limit.saturating_sub(self.output_reserve))
            .max(1)
    }

    pub fn mark_compacted(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.compactions = self.compactions.saturating_add(1);
    }

    pub fn mark_reset(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.resets = self.resets.saturating_add(1);
    }
}

const MIN_OUTPUT_RESERVE: usize = 4_096;

/// 按窗口大小给回答预留空间：约 15%（至少 4K），不超过模型最大输出和窗口的 1/4。
pub fn output_reserve_for(window: usize, max_output: Option<usize>) -> usize {
    let proportional = (window.saturating_mul(15) / 100).max(MIN_OUTPUT_RESERVE);
    let reserve = match max_output.filter(|tokens| *tokens > 0) {
        Some(max_output) => max_output.min(proportional),
        None => proportional,
    };
    reserve.min(window / 4)
}

pub fn total_chars(messages: &[Message]) -> usize {
    messages.iter().map(message_chars).sum()
}

/// Legacy character-based threshold retained for API compatibility.
pub fn should_compact(messages: &[Message], limit: usize) -> bool {
    if limit == 0 {
        return false;
    }
    total_chars(messages).saturating_mul(100) >= limit.saturating_mul(COMPACT_THRESHOLD_PERCENT)
}

pub fn should_compact_tokens(messages: &[Message], limit: usize) -> bool {
    should_compact_tokens_with(messages, limit, COMPACT_THRESHOLD_PERCENT)
}

pub fn should_compact_tokens_with(messages: &[Message], limit: usize, percent: usize) -> bool {
    if limit == 0 {
        return false;
    }
    total_message_tokens(messages).saturating_mul(100) >= limit.saturating_mul(percent.max(1))
}

const MICROCOMPACT_KEEP_RECENT_TOOL_RESULTS: usize = 6;
const MICROCOMPACT_MIN_CHARS: usize = 400;
const MICROCOMPACT_STUB_PREFIX: &str = "[已微压缩]";

/// 微压缩：把较早的、较长的工具结果替换成一行占位，保留调用结构与最近几条完整结果。
/// 返回被替换的条数。比全量摘要便宜，先于全量压缩尝试。
pub fn microcompact(messages: &mut [Message]) -> usize {
    let tool_indexes: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == Role::Tool)
        .map(|(index, _)| index)
        .collect();
    if tool_indexes.len() <= MICROCOMPACT_KEEP_RECENT_TOOL_RESULTS {
        return 0;
    }
    let cutoff = tool_indexes.len() - MICROCOMPACT_KEEP_RECENT_TOOL_RESULTS;
    let mut replaced = 0;
    for index in &tool_indexes[..cutoff] {
        let message = &mut messages[*index];
        if message.content.starts_with(MICROCOMPACT_STUB_PREFIX)
            || message.content.chars().count() < MICROCOMPACT_MIN_CHARS
        {
            continue;
        }
        let chars = message.content.chars().count();
        let label = if message.name.is_empty() {
            "工具".to_string()
        } else {
            message.name.clone()
        };
        let first_line: String = message
            .content
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(120)
            .collect();
        message.content = format!(
            "{MICROCOMPACT_STUB_PREFIX} {label} 结果共 {chars} 字符已省略；首行：{first_line}。需要时请重新调用。"
        );
        message.images.clear();
        replaced += 1;
    }
    replaced
}

/// Compact old user turns while keeping the system constraints and the latest
/// turn verbatim. This is deterministic fallback behavior when a remote
/// summary is unavailable or too expensive.
pub fn compact_local(messages: &mut Vec<Message>) -> bool {
    let sys_len = system_prefix_len(messages);
    let rest = &messages[sys_len..];
    let groups = group_user_turns(rest);
    if groups.len() < 2 {
        return false;
    }
    let (old, recent) = groups.split_at(groups.len() - 1);
    // A summary is itself a user message. If the summary is the only old
    // group, compacting it again would slowly grow the prompt on every turn.
    if old.len() == 1 && old[0].first().is_some_and(is_context_summary) {
        return false;
    }
    let to_summarize: Vec<Message> = old.iter().flatten().cloned().collect();
    let preserved: Vec<Message> = recent.iter().flatten().cloned().collect();
    if to_summarize.is_empty() || preserved.is_empty() {
        return false;
    }
    let summary = local_summary(&to_summarize);
    replace_with_summary(messages, &summary, &preserved, sys_len)
}

/// Hard reset used when there is only one user group (for example a task that
/// produced many tool calls). It preserves the current request and a small
/// recent tail while summarizing the rest.
pub fn reset_local(messages: &mut Vec<Message>) -> bool {
    let sys_len = system_prefix_len(messages);
    if messages.len().saturating_sub(sys_len) <= RESET_KEEP_MESSAGES {
        return false;
    }
    let rest = &messages[sys_len..];
    let keep_from = rest.len().saturating_sub(RESET_KEEP_MESSAGES);
    let to_summarize = rest[..keep_from].to_vec();
    let mut preserved = rest[keep_from..].to_vec();
    if let Some(last_user) = messages.iter().rposition(|message| {
        message.role == Role::User && !is_context_summary(message) && !is_synthetic_user(message)
    }) {
        if !preserved.iter().any(|message| {
            message.role == Role::User && message.content == messages[last_user].content
        }) {
            preserved.insert(0, messages[last_user].clone());
        }
    }
    if to_summarize.is_empty() || preserved.is_empty() {
        return false;
    }
    let summary = local_summary(&to_summarize);
    replace_with_summary(messages, &summary, &preserved, sys_len)
}

/// Replace history with a caller-provided (possibly remote-generated)
/// summary. Keeping this operation separate lets the model runner use a
/// provider's summarization endpoint without duplicating message surgery.
pub fn compact_with_summary(messages: &mut Vec<Message>, summary: &str) -> bool {
    let Some((sys_len, to_summarize, preserved)) = compaction_segments(messages) else {
        return false;
    };
    if to_summarize.is_empty() {
        return false;
    }
    replace_with_summary(messages, summary, &preserved, sys_len)
}

/// Build a tool-free request for a model-generated compaction summary.
/// The user turn includes a richer `handoff_transcript` (not the 800-char local
/// preview) so error stacks and tool observations survive the first pass.
pub fn compaction_prompt(messages: &[Message]) -> Option<Vec<Message>> {
    compaction_prompt_with_instructions(messages, None)
}

/// 同上，但允许附加用户的 `/compact` 指令（例如「保留所有失败堆栈」）。
pub fn compaction_prompt_with_instructions(
    messages: &[Message],
    instructions: Option<&str>,
) -> Option<Vec<Message>> {
    let (sys_len, to_summarize, _preserved) = compaction_segments(messages)?;
    let constraints = messages[..sys_len]
        .iter()
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let extra = instructions
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| format!("\n\nAdditional instructions from the user for this summary (follow them while keeping the headings):\n{item}"))
        .unwrap_or_default();
    let prompt = format!(
        "Create a concise structured handoff for the next model turn. Preserve the active user goal, system constraints, completed changes, important tool observations, verification results, and pending work. Do not call tools and do not answer the user directly. Use these headings: User goal, Constraints, Completed work, Verification, Pending work.{extra}\n\nSystem constraints:\n{}\n\nEarlier conversation:\n{}",
        if constraints.is_empty() {
            "(none)"
        } else {
            &constraints
        },
        handoff_transcript(&to_summarize)
    );
    Some(vec![
        Message::system(
            "You compact coding-agent context into a factual structured handoff. Keep it short and preserve actionable details.",
        ),
        Message::user(prompt),
    ])
}

fn compaction_segments(messages: &[Message]) -> Option<(usize, Vec<Message>, Vec<Message>)> {
    let sys_len = system_prefix_len(messages);
    let rest = &messages[sys_len..];
    let groups = group_user_turns(rest);
    if groups.is_empty() {
        return None;
    }
    if groups.len() >= 2 {
        let old = &groups[..groups.len() - 1];
        if old.len() == 1 && old[0].first().is_some_and(is_context_summary) {
            return None;
        }
        let to_summarize = old.iter().flatten().cloned().collect::<Vec<_>>();
        let preserved = groups.last().cloned().unwrap_or_default();
        return Some((sys_len, to_summarize, preserved));
    }

    // A single user turn can still produce many assistant/tool messages. Keep
    // the latest tail and summarize the older part just like reset_local.
    if rest.len() <= RESET_KEEP_MESSAGES || rest.first().is_some_and(is_context_summary) {
        return None;
    }
    let keep_from = rest.len().saturating_sub(RESET_KEEP_MESSAGES);
    let to_summarize = rest[..keep_from].to_vec();
    let mut preserved = rest[keep_from..].to_vec();
    if let Some(last_user) = messages
        .iter()
        .rposition(|message| message.role == Role::User && !is_context_summary(message))
    {
        if !preserved.iter().any(|message| {
            message.role == Role::User && message.content == messages[last_user].content
        }) {
            preserved.insert(0, messages[last_user].clone());
        }
    }
    Some((sys_len, to_summarize, preserved))
}

fn replace_with_summary(
    messages: &mut Vec<Message>,
    summary: &str,
    preserved: &[Message],
    sys_len: usize,
) -> bool {
    if summary.trim().is_empty() || preserved.is_empty() {
        return false;
    }
    let mut next = messages[..sys_len].to_vec();
    next.push(Message::user(format!(
        "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\n{summary}\n\nRecent messages are preserved verbatim."
    )));
    next.extend_from_slice(preserved);
    *messages = next;
    true
}

fn system_prefix_len(messages: &[Message]) -> usize {
    messages
        .iter()
        .take_while(|message| message.role == Role::System)
        .count()
}

fn group_user_turns(messages: &[Message]) -> Vec<Vec<Message>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for message in messages {
        if message.role == Role::User && !is_synthetic_user(message) && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current.push(message.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn local_summary(messages: &[Message]) -> String {
    let mut goals = Vec::new();
    let mut completed = Vec::new();
    let mut observations = Vec::new();
    let mut pending = Vec::new();
    for message in messages {
        let preview = message_preview(message);
        if preview.is_empty() {
            continue;
        }
        match message.role {
            Role::User if is_synthetic_user(message) => observations.push(preview),
            Role::User => goals.push(preview),
            Role::Assistant => completed.push(preview),
            Role::Tool => observations.push(preview),
            Role::System => pending.push(preview),
        }
    }
    let mut sections = Vec::new();
    push_summary_section(&mut sections, "User goals", &goals);
    push_summary_section(&mut sections, "Completed work", &completed);
    push_summary_section(&mut sections, "Tool observations", &observations);
    push_summary_section(&mut sections, "Pending context", &pending);
    if sections.is_empty() {
        "No earlier messages were available.".to_string()
    } else {
        sections.join("\n")
    }
}

fn push_summary_section(sections: &mut Vec<String>, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let keep = items.len().min(SUMMARY_KEEP);
    let lines = items[items.len() - keep..]
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    sections.push(format!("### {title}\n{}", lines.join("\n")));
}

fn message_preview(message: &Message) -> String {
    if message.content.trim().is_empty() && !message.tool_calls.is_empty() {
        return format!(
            "tool_calls: {}",
            message
                .tool_calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let text = message.content.replace('\n', " ");
    if text.chars().count() > PREVIEW_CHARS {
        let prefix: String = text.chars().take(PREVIEW_CHARS).collect();
        format!("{prefix}…")
    } else {
        text
    }
}

pub fn is_usable_compaction_summary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() < 80 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        "user goal",
        "constraints",
        "completed work",
        "verification",
        "pending work",
    ]
    .iter()
    .filter(|heading| lower.contains(*heading))
    .count()
        >= 2
}

fn handoff_transcript(messages: &[Message]) -> String {
    let entries: Vec<(bool, String)> = messages
        .iter()
        .filter_map(|message| {
            let text = handoff_message(message);
            if text.is_empty() {
                None
            } else {
                Some((
                    message.role == Role::Tool && looks_like_error_observation(&message.content),
                    text,
                ))
            }
        })
        .collect();
    if entries.is_empty() {
        return "No earlier messages were available.".to_string();
    }
    let mut include = vec![true; entries.len()];
    let mut total: usize = entries.iter().map(|(_, text)| text.chars().count()).sum();
    for (index, (important, text)) in entries.iter().enumerate() {
        if total <= HANDOFF_TOTAL_CHARS {
            break;
        }
        if *important {
            continue;
        }
        include[index] = false;
        total = total.saturating_sub(text.chars().count());
    }
    for (index, (_, text)) in entries.iter().enumerate() {
        if total <= HANDOFF_TOTAL_CHARS {
            break;
        }
        if !include[index] {
            continue;
        }
        include[index] = false;
        total = total.saturating_sub(text.chars().count());
    }
    entries
        .into_iter()
        .enumerate()
        .filter(|(index, _)| include[*index])
        .map(|(_, (_, text))| text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn handoff_message(message: &Message) -> String {
    let role = match message.role {
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::Tool => "Tool",
        Role::System => "System",
    };
    let mut body = if message.content.trim().is_empty() && !message.tool_calls.is_empty() {
        format!(
            "tool_calls: {}",
            message
                .tool_calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        message.content.clone()
    };
    if body.chars().count() > HANDOFF_CHARS {
        let prefix: String = body.chars().take(HANDOFF_CHARS).collect();
        body = format!("{prefix}…");
    }
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("{role}:\n{body}")
    }
}

fn looks_like_error_observation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "error",
        "fail",
        "panic",
        "assertion",
        "traceback",
        "exception",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// 工具图片说明消息的固定片段：`（{工具} 工具返回的图片：{名称}）`。
pub const TOOL_IMAGE_NOTE_MARKER: &str = " 工具返回的图片：";
pub const STOP_HOOK_CONTINUE_PREFIX: &str = "[Stop 钩子要求继续]";
pub const BACKGROUND_NOTICE_PREFIX: &str = "[后台任务提醒]";
const LAST_TURN_NOTE_PREFIX: &str = "工具轮次已达上限";

/// 由 Agent 自己插入的 user 消息（工具图片、钩子、提醒），不是新的用户回合。
pub fn is_synthetic_user(message: &Message) -> bool {
    if message.role != Role::User {
        return false;
    }
    let content = message.content.as_str();
    (content.starts_with('（') && content.contains(TOOL_IMAGE_NOTE_MARKER))
        || content.starts_with(STOP_HOOK_CONTINUE_PREFIX)
        || content.starts_with(BACKGROUND_NOTICE_PREFIX)
        || content.starts_with(LAST_TURN_NOTE_PREFIX)
}

/// 压缩时由后端拼进摘要的运行状态，不依赖模型复述。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreservedState {
    pub goal: Option<String>,
    pub permissions: String,
    pub approved_plan: Option<String>,
    pub open_todos: Vec<String>,
}

const PRESERVED_HEADER: &str = "[压缩保留的状态]";
const PRESERVED_ITEM_CHARS: usize = 240;
const DENIAL_MARKERS: [&str; 3] = ["用户不允许", "已按拒绝处理", "拒绝了该操作"];
const DENIAL_HEADING: &str = "- 已被拒绝的操作（不要重复尝试同样的操作）：";
const MEDIA_HEADING: &str = "- 已摘要的媒体（只保留引用，需要时重新读取）：";
/// 每类保留条目的上限，防止多次压缩后越积越长。
const PRESERVED_MAX_ITEMS: usize = 20;

fn keep_recent(mut items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.clone()));
    let skip = items.len().saturating_sub(PRESERVED_MAX_ITEMS);
    items.split_off(skip)
}

/// 从被再次压缩的旧摘要里取出上一轮保留块的条目。
fn carried_items(dropped: &[Message], heading: &str) -> Vec<String> {
    let mut items = Vec::new();
    for message in dropped.iter().filter(|message| is_context_summary(message)) {
        let Some(start) = message.content.rfind(PRESERVED_HEADER) else {
            continue;
        };
        let mut inside = false;
        for line in message.content[start..].lines() {
            if line == heading {
                inside = true;
            } else if let Some(item) = line.strip_prefix("  - ").filter(|_| inside) {
                items.push(item.to_string());
            } else {
                inside = false;
            }
        }
    }
    items
}

/// 生成摘要前缀：运行状态 + 被摘要掉的消息里的拒绝记录和媒体引用。
pub fn preserved_context_block(state: &PreservedState, dropped: &[Message]) -> String {
    let mut lines = vec![PRESERVED_HEADER.to_string()];
    if let Some(goal) = state.goal.as_deref().filter(|goal| !goal.trim().is_empty()) {
        lines.push(format!("- 当前目标：{}", goal.trim()));
    }
    if !state.permissions.is_empty() {
        lines.push(format!("- 权限：{}", state.permissions));
    }
    if let Some(plan) = state.approved_plan.as_deref() {
        lines.push(format!("- 已批准计划文件：{plan}"));
    }
    if !state.open_todos.is_empty() {
        lines.push("- 未完成待办：".to_string());
        lines.extend(state.open_todos.iter().map(|todo| format!("  - {todo}")));
    }
    let denials = keep_recent(
        carried_items(dropped, DENIAL_HEADING)
            .into_iter()
            .chain(denied_operations(dropped))
            .collect(),
    );
    if !denials.is_empty() {
        lines.push(DENIAL_HEADING.to_string());
        lines.extend(denials.iter().map(|item| format!("  - {item}")));
    }
    let media = keep_recent(
        carried_items(dropped, MEDIA_HEADING)
            .into_iter()
            .chain(dropped_media_refs(dropped))
            .collect(),
    );
    if !media.is_empty() {
        lines.push(MEDIA_HEADING.to_string());
        lines.extend(media.iter().map(|item| format!("  - {item}")));
    }
    if lines.len() == 1 {
        return String::new();
    }
    lines.join("\n")
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.replace('\n', " ");
    if text.chars().count() > limit {
        format!("{}…", text.chars().take(limit).collect::<String>())
    } else {
        text
    }
}

fn denied_operations(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .filter(|message| {
            DENIAL_MARKERS
                .iter()
                .any(|marker| message.content.contains(marker))
        })
        .map(|result| {
            let call = messages
                .iter()
                .flat_map(|message| message.tool_calls.iter())
                .find(|call| call.id == result.tool_call_id);
            match call {
                Some(call) => format!(
                    "{} {}：{}",
                    call.name,
                    clip(&call.arguments, PRESERVED_ITEM_CHARS),
                    clip(&result.content, PRESERVED_ITEM_CHARS)
                ),
                None => clip(&result.content, PRESERVED_ITEM_CHARS),
            }
        })
        .collect()
}

fn dropped_media_refs(messages: &[Message]) -> Vec<String> {
    let mut refs = Vec::new();
    for message in messages {
        if !message.images.is_empty() {
            refs.push(crate::native::media_plan::image_ref_summary(
                &message.images,
            ));
        } else {
            refs.extend(
                message
                    .media
                    .iter()
                    .map(|item| format!("附件 {}", item.attachment_id())),
            );
        }
    }
    refs
}

/// 压缩前有、压缩后不再出现的消息（按历史身份，未入库的按角色 + 内容）。
pub fn dropped_messages(before: &[Message], after: &[Message]) -> Vec<Message> {
    before
        .iter()
        .filter(|old| {
            !after.iter().any(|new| {
                if !old.history_id.is_empty() {
                    new.history_id == old.history_id
                } else {
                    new.role == old.role
                        && new.tool_call_id == old.tool_call_id
                        && new.content == old.content
                }
            })
        })
        .cloned()
        .collect()
}

/// 把保留块追加到刚生成的压缩摘要消息；没有摘要消息（微压缩）时不改动。
pub fn inject_preserved_context(messages: &mut [Message], block: &str) -> bool {
    if block.is_empty() {
        return false;
    }
    let Some(summary) = messages
        .iter_mut()
        .find(|message| is_context_summary(message))
    else {
        return false;
    };
    summary.content = format!("{}\n\n{block}", summary.content);
    true
}

fn is_context_summary(message: &Message) -> bool {
    message.role == Role::User
        && message
            .content
            .starts_with("This session is being continued from a previous conversation")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_message(id: &str, name: &str, content: String) -> Message {
        let mut message = Message::tool_result(id, content);
        message.name = name.to_string();
        message
    }

    #[test]
    fn microcompact_stubs_old_long_tool_results_only() {
        let mut messages = vec![Message::system("sys"), Message::user("go")];
        for index in 0..9 {
            messages.push(Message::assistant_text(format!("step {index}")));
            messages.push(tool_message(
                &format!("c{index}"),
                "Read",
                format!("line one of {index}\n{}", "x".repeat(600)),
            ));
        }
        messages.push(tool_message("short", "Glob", "tiny".to_string()));
        let replaced = microcompact(&mut messages);
        // 10 条工具结果，保留最近 6 条 → 前 4 条里 4 条够长被替换。
        assert_eq!(replaced, 4);
        let stubs: Vec<&Message> = messages
            .iter()
            .filter(|message| message.content.starts_with("[已微压缩]"))
            .collect();
        assert_eq!(stubs.len(), 4);
        assert!(stubs[0].content.contains("Read"));
        assert!(stubs[0].content.contains("line one of 0"));
        // 再跑一次不重复替换。
        assert_eq!(microcompact(&mut messages), 0);
        let mut few = vec![tool_message("a", "Read", "y".repeat(1000))];
        assert_eq!(microcompact(&mut few), 0);
    }

    #[test]
    fn boundary_line_round_trips_and_overflow_detection() {
        let boundary = CompactBoundary {
            trigger: CompactTrigger::Reactive,
            source: "model".to_string(),
            outcome: CompactOutcome::NoGain,
            pre_tokens: 120_000,
            post_tokens: 30_000,
            pre_messages: 80,
            post_messages: 12,
            instructions: Some("keep stacks".to_string()),
        };
        let line = boundary.line();
        assert!(line.starts_with(COMPACT_BOUNDARY_PREFIX));
        let parsed = CompactBoundary::parse_line(&line).expect("parse");
        assert_eq!(parsed, boundary);
        // 旧记录没有 outcome 字段，按成功读取。
        let legacy = CompactBoundary::parse_line(
            r#"[COMPACT_BOUNDARY] {"trigger":"auto","source":"local","pre_tokens":9,"post_tokens":3,"pre_messages":4,"post_messages":2}"#,
        )
        .expect("legacy");
        assert_eq!(legacy.outcome, CompactOutcome::Success);
        assert!(compaction_gained(100, 94));
        assert!(!compaction_gained(100, 95));
        assert!(CompactBoundary::parse_line("[工具] nope").is_none());
        assert!(is_context_overflow_error(
            "模型请求失败（HTTP 400）: This model's maximum context length is 128000 tokens"
        ));
        assert!(is_context_overflow_error(
            "prompt is too long: 210000 tokens > 200000"
        ));
        assert!(is_context_overflow_error("context_length_exceeded"));
        assert!(!is_context_overflow_error(
            "rate limit exceeded, context window fine"
        ));
        assert!(!is_context_overflow_error("invalid api key"));
    }

    fn image(id: &str, page: Option<u32>) -> crate::native::model::types::NativeImage {
        crate::native::model::types::NativeImage {
            name: format!("{id}.png"),
            mime_type: "image/png".to_string(),
            data_base64: "QUJD".repeat(1_000),
            attachment_id: id.to_string(),
            page,
            time_range: None,
        }
    }

    #[test]
    fn synthetic_user_notes_do_not_split_the_latest_turn() {
        let mut call = Message::assistant_text("");
        call.tool_calls = vec![crate::native::model::types::ToolCall {
            id: "c1".to_string(),
            name: "Read".to_string(),
            arguments: "{}".to_string(),
        }];
        let mut note = Message::user(format!("（Read{TOOL_IMAGE_NOTE_MARKER}shot.png）"));
        note.images.push(image("att-1", None));
        let mut messages = vec![
            Message::user("旧任务"),
            Message::assistant_text("旧任务完成"),
            Message::user("真正的新任务"),
            call,
            Message::tool_result("c1", "读取结果"),
            note,
            Message::assistant_text("看完图片"),
            Message::user(format!("{STOP_HOOK_CONTINUE_PREFIX} 还有测试没跑")),
        ];
        assert!(is_synthetic_user(&messages[5]));
        assert!(is_synthetic_user(&messages[7]));
        assert!(!is_synthetic_user(&messages[2]));
        assert!(compact_local(&mut messages));
        // 真正的用户请求及其工具调用、结果、图片说明原样保留，没有被摘要掉。
        assert!(messages
            .iter()
            .any(|message| message.role == Role::User && message.content == "真正的新任务"));
        assert!(messages.iter().any(|message| message.tool_call_id == "c1"));
        assert!(messages.iter().any(|message| !message.images.is_empty()));
        assert!(!messages
            .iter()
            .any(|message| message.content == "旧任务完成"));
    }

    #[test]
    fn preserved_block_keeps_state_denials_and_media_references() {
        let mut denied_call = Message::assistant_text("");
        denied_call.tool_calls = vec![crate::native::model::types::ToolCall {
            id: "d1".to_string(),
            name: "Bash".to_string(),
            arguments: r#"{"command":"rm -rf build"}"#.to_string(),
        }];
        let mut with_image = Message::user("看这张图");
        with_image.images.push(image("att-9", Some(2)));
        let dropped = vec![
            with_image,
            denied_call,
            Message::tool_result("d1", "工具执行失败：用户不允许该高风险操作"),
        ];
        let state = PreservedState {
            goal: Some("目标：修复登录".to_string()),
            permissions: "计划模式关；只读否；高风险操作需逐项确认".to_string(),
            approved_plan: Some(".noxcode/plans/plan-s.md".to_string()),
            open_todos: vec!["[pending] 补测试".to_string()],
        };
        let block = preserved_context_block(&state, &dropped);
        for expected in [
            "目标：修复登录",
            "高风险操作需逐项确认",
            ".noxcode/plans/plan-s.md",
            "[pending] 补测试",
            "rm -rf build",
            "用户不允许",
            "附件 att-9",
            "第 2 页",
        ] {
            assert!(block.contains(expected), "{expected}\n{block}");
        }
        assert!(!block.contains("QUJD"), "不能带原始媒体载荷");

        // 二次压缩：旧摘要里的拒绝和媒体条目继承下来，且不重复。
        let mut messages = vec![
            Message::user("旧任务"),
            Message::assistant_text("旧回答"),
            Message::user("新任务"),
        ];
        assert!(compact_local(&mut messages));
        assert!(inject_preserved_context(&mut messages, &block));
        let old_summary = messages[0].clone();
        let next = preserved_context_block(
            &PreservedState::default(),
            &[old_summary.clone(), old_summary],
        );
        assert_eq!(next.matches("附件 att-9").count(), 1, "{next}");
        assert!(next.contains("rm -rf build"));
        assert!(preserved_context_block(&PreservedState::default(), &[]).is_empty());
    }

    #[test]
    fn output_reserve_scales_with_window_instead_of_fixed_32k() {
        assert_eq!(output_reserve_for(8_192, None), 2_048);
        assert_eq!(output_reserve_for(128_000, None), 19_200);
        assert_eq!(output_reserve_for(128_000, Some(8_192)), 8_192);
        assert_eq!(output_reserve_for(1_000_000, Some(128_000)), 128_000);
        assert_eq!(output_reserve_for(1_000_000, None), 150_000);
        let mut window = ContextWindow::new(1_000_000);
        window.set_output_reserve(output_reserve_for(1_000_000, Some(128_000)));
        assert_eq!(window.trigger_tokens(), 850_000);
        assert!(window.should_compact_used(850_000));
        assert!(!window.should_compact_used(849_999));
    }

    #[test]
    fn threshold_percent_and_instruction_prompt() {
        let mut window = ContextWindow::new(1_000);
        window.set_threshold_percent(50);
        assert_eq!(window.threshold_tokens(), 500);
        window.set_threshold_percent(5);
        assert_eq!(window.threshold_percent, 30);
        let messages = vec![
            Message::system("sys"),
            Message::user("first task"),
            Message::assistant_text("done first"),
            Message::user("second task"),
        ];
        let prompt = compaction_prompt_with_instructions(&messages, Some("保留所有失败堆栈"))
            .expect("prompt");
        assert!(prompt[1].content.contains("保留所有失败堆栈"));
        assert!(prompt[1].content.contains("Additional instructions"));
        let plain = compaction_prompt(&messages).expect("prompt");
        assert!(!plain[1].content.contains("Additional instructions"));
    }

    #[test]
    fn budget_reservation_is_shared_and_settled() {
        let budget = RolloutBudget::new(100);
        assert!(budget.try_reserve(60));
        assert!(!budget.try_reserve(50));
        budget.settle(60, Some(20));
        assert_eq!(budget.spent(), 20);
        assert_eq!(budget.remaining(), 80);
        assert!(budget.try_reserve(80));
        assert!(budget.is_exhausted());
    }

    #[test]
    fn budget_release_restores_remaining_capacity() {
        let budget = RolloutBudget::new(100);
        assert!(budget.try_reserve(75));
        budget.release(75);
        assert_eq!(budget.spent(), 0);
        assert_eq!(budget.snapshot().active_reservations, 0);
    }

    #[test]
    fn unlimited_budget_still_records_usage_for_diagnostics() {
        let budget = RolloutBudget::new(0);
        assert!(budget.try_reserve(40));
        assert_eq!(budget.snapshot().active_reservations, 40);
        budget.settle(40, Some(12));
        assert_eq!(budget.spent(), 12);
        assert_eq!(budget.remaining(), u64::MAX);
        budget.record_usage(Usage {
            prompt_tokens: 3,
            completion_tokens: 5,
            ..Usage::default()
        });
        assert_eq!(budget.spent(), 20);
    }

    #[test]
    fn keeps_system_and_latest_user_turn() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("first"),
            Message::assistant_text("looked"),
            Message::tool_result("c1", "fn a() {}"),
            Message::user("second"),
            Message::assistant_text("done"),
        ];
        assert!(compact_local(&mut messages));
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[1].content.contains("User goals"));
        assert!(messages.iter().any(|item| item.content == "second"));
        assert!(messages.iter().any(|item| item.content == "done"));
        assert!(!messages.iter().any(|item| item.content == "first"));
    }

    #[test]
    fn reset_keeps_recent_tail_for_single_user_turn() {
        let mut messages = vec![Message::system("sys"), Message::user("task")];
        for index in 0..8 {
            messages.push(Message::assistant_text(format!("step {index}")));
        }
        assert!(reset_local(&mut messages));
        assert!(messages.iter().any(|item| item.content == "task"));
        assert!(messages
            .iter()
            .any(|item| item.content.contains("Completed work")));
        assert!(messages.len() < 10);
    }

    #[test]
    fn skips_when_only_one_short_turn() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("only"),
            Message::assistant_text("ok"),
        ];
        assert!(!compact_local(&mut messages));
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn does_not_resummarize_an_already_compacted_window() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user(
                "This session is being continued from a previous conversation that ran out of context.\n\n### Completed work\n- done",
            ),
            Message::user("current"),
            Message::assistant_text("answer"),
        ];
        assert!(!compact_local(&mut messages));
    }

    #[test]
    fn reset_can_compact_more_history_after_a_summary() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user(
                "This session is being continued from a previous conversation that ran out of context.\n\n### Completed work\n- done",
            ),
            Message::user("current"),
        ];
        for index in 0..8 {
            messages.push(Message::assistant_text(format!("step {index}")));
        }
        assert!(reset_local(&mut messages));
        assert!(messages
            .iter()
            .any(|message| message.content.contains("Completed work")));
        assert!(messages.iter().any(|message| message.content == "current"));
        assert!(messages.len() < 12);
    }

    #[test]
    fn compaction_prompt_supports_tool_heavy_single_turn() {
        let mut messages = vec![Message::system("keep constraints"), Message::user("task")];
        for index in 0..8 {
            messages.push(Message::assistant_text(format!("step {index}")));
        }
        let prompt = compaction_prompt(&messages).expect("compaction prompt");
        assert_eq!(prompt[0].role, Role::System);
        assert!(prompt[1].content.contains("Completed work"));
        assert!(compact_with_summary(
            &mut messages,
            "### Completed work\n- summarized"
        ));
        assert!(messages
            .iter()
            .any(|message| message.content.contains("summarized")));
    }

    #[test]
    fn compaction_prompt_keeps_error_stack_beyond_old_preview_limit() {
        let stack = format!("error: boom\n{}", "frame\n".repeat(80));
        assert!(stack.chars().count() > 320);
        let messages = vec![
            Message::system("keep constraints"),
            Message::user("first"),
            Message::assistant_text("ran tests"),
            Message::tool_result("c1", stack.clone()),
            Message::user("current"),
            Message::assistant_text("next"),
        ];
        let prompt = compaction_prompt(&messages).expect("compaction prompt");
        assert!(
            prompt[1].content.contains("error: boom"),
            "handoff must include the error stack: {}",
            prompt[1].content
        );
        assert!(
            prompt[1].content.contains("frame"),
            "handoff must keep more than a 320-char preview"
        );
    }

    #[test]
    fn usable_compaction_summary_requires_headings_and_length() {
        assert!(!is_usable_compaction_summary("ok"));
        assert!(!is_usable_compaction_summary(
            "This is a long enough paragraph without any required headings at all for a handoff."
        ));
        assert!(is_usable_compaction_summary(
            "### User goal\nFix login.\n\n### Completed work\nUpdated the auth handler and added a regression test for expired tokens."
        ));
    }
}
