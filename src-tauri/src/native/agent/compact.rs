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

/// State for a logical model context window. A new generation is used after
/// local compaction/reset, making it possible to diagnose repeated input
/// growth without retaining the old messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextWindow {
    pub generation: u32,
    pub token_limit: usize,
    pub compactions: u32,
    pub resets: u32,
}

impl ContextWindow {
    pub fn new(token_limit: usize) -> Self {
        Self {
            generation: 0,
            token_limit,
            compactions: 0,
            resets: 0,
        }
    }

    pub fn set_token_limit(&mut self, token_limit: usize) {
        self.token_limit = token_limit;
    }

    pub fn should_compact(&self, messages: &[Message]) -> bool {
        should_compact_tokens(messages, self.token_limit)
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
    if limit == 0 {
        return false;
    }
    total_message_tokens(messages).saturating_mul(100)
        >= limit.saturating_mul(COMPACT_THRESHOLD_PERCENT)
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
    let (sys_len, to_summarize, _preserved) = compaction_segments(messages)?;
    let constraints = messages[..sys_len]
        .iter()
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = format!(
        "Create a concise structured handoff for the next model turn. Preserve the active user goal, system constraints, completed changes, important tool observations, verification results, and pending work. Do not call tools and do not answer the user directly. Use these headings: User goal, Constraints, Completed work, Verification, Pending work.\n\nSystem constraints:\n{}\n\nEarlier conversation:\n{}",
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
        if message.role == Role::User && !current.is_empty() {
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

fn is_context_summary(message: &Message) -> bool {
    message.role == Role::User
        && message
            .content
            .starts_with("This session is being continued from a previous conversation")
}

#[cfg(test)]
mod tests {
    use super::*;

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
