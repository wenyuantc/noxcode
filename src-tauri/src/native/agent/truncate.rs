use std::collections::HashSet;

use serde_json::Value;

use crate::native::model::types::{Message, NativeImage, Role, ToolSpec};

const IMAGE_REMOVED_NOTICE: &str = "[图片已因上下文超限移除]";

/// The model API does not expose a tokenizer for every configured provider. A
/// deliberately conservative estimate keeps the context bounded while still
/// working for both ASCII and CJK text: ASCII words average roughly four
/// characters per token, while a non-ASCII code point is usually one token.
pub fn estimate_text_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.saturating_add(3) / 4 + non_ascii + usize::from(!text.is_empty())
}

/// Compatibility alias used by callers that only have an arbitrary string.
pub fn estimate_tokens(text: &str) -> usize {
    estimate_text_tokens(text)
}

pub fn message_chars(message: &Message) -> usize {
    message.content.chars().count()
        + message.reasoning_content.chars().count()
        + message
            .tool_calls
            .iter()
            .map(|call| call.arguments.chars().count() + call.name.chars().count())
            .sum::<usize>()
        + message
            .images
            .iter()
            .map(|image| image.data_base64.chars().count())
            .sum::<usize>()
}

/// Estimate the serialized cost of one message. The fixed overhead accounts
/// for role/name/tool-call JSON framing that is otherwise easy to overlook.
pub fn message_tokens(message: &Message) -> usize {
    let content = estimate_text_tokens(&message.content);
    let reasoning = estimate_text_tokens(&message.reasoning_content);
    let calls = message
        .tool_calls
        .iter()
        .map(|call| {
            4usize
                .saturating_add(estimate_text_tokens(&call.id))
                .saturating_add(estimate_text_tokens(&call.name))
                .saturating_add(estimate_text_tokens(&call.arguments))
        })
        .sum::<usize>();
    let images = message.images.iter().map(image_tokens).sum::<usize>();
    4usize
        .saturating_add(content)
        .saturating_add(reasoning)
        .saturating_add(calls)
        .saturating_add(images)
}

fn decoded_image_bytes(image: &NativeImage) -> usize {
    let raw = image.data_base64.trim().as_bytes();
    if raw.is_empty() {
        return 0;
    }
    let padding = raw.iter().rev().take_while(|byte| **byte == b'=').count();
    raw.len().saturating_mul(3) / 4 - padding
}

/// Visual-token estimate based on decoded image bytes, not base64 length.
pub fn image_tokens(image: &NativeImage) -> usize {
    let bytes = decoded_image_bytes(image);
    let visual: usize = if bytes < 100 * 1024 {
        800
    } else if bytes < 500 * 1024 {
        1_600
    } else if bytes < 2 * 1024 * 1024 {
        2_600
    } else {
        4_000
    };
    visual.saturating_add(16)
}

pub fn total_message_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}

/// Estimate the serialized tool definitions sent alongside a request. Tool
/// schemas are part of every non-final input and can be surprisingly large for
/// MCP servers, so they must count toward both the context and rollout guards.
pub fn total_tool_tokens(tools: &[ToolSpec]) -> usize {
    tools
        .iter()
        .map(|tool| {
            4usize
                .saturating_add(estimate_text_tokens(&tool.name))
                .saturating_add(estimate_text_tokens(&tool.description))
                .saturating_add(estimate_text_tokens(&tool.parameters.to_string()))
        })
        .sum()
}

/// Convert the legacy character setting to a conservative token budget.
pub fn chars_to_tokens(chars: usize) -> usize {
    // CJK text is close to one token per character; using the smaller ASCII
    // ratio would allow a Chinese prompt to exceed the configured window.
    chars.saturating_add(1) / 2
}

pub const DEFAULT_TOOL_RESULT_TOKEN_LIMIT: usize = 4_096;

/// Keep a bounded head and tail of a tool result and tell the model how to
/// retrieve the omitted part. The full result remains available to the event
/// stream; only this representation is appended to model history.
pub fn truncate_tool_result(
    name: &str,
    arguments: &str,
    output: &str,
    max_tokens: usize,
) -> String {
    if max_tokens == 0 {
        return "[truncated: tool output omitted]".to_string();
    }
    let original_tokens = estimate_text_tokens(output);
    if original_tokens <= max_tokens {
        return output.to_string();
    }

    let trimmed = output.trim_end();
    let lines: Vec<&str> = trimmed.lines().collect();
    let line_count = lines.len();
    // Start with a cheap marker to reserve space, then replace its path hint
    // with the actual head offset once the head slice has been selected.
    let hint = continuation_hint(name, arguments, 1);
    let mut marker =
        format!("\n…[truncated: {original_tokens} tokens, {line_count} lines omitted]\n{hint}\n");
    let marker_tokens = estimate_text_tokens(&marker);
    let available = max_tokens.saturating_sub(marker_tokens);
    if available < 2 {
        return fit_text_tokens(&marker, max_tokens);
    }

    let head_budget = available.saturating_mul(2) / 3;
    let tail_budget = available.saturating_sub(head_budget);
    let mut head = snap_prefix_to_complete_lines(&take_prefix_tokens(trimmed, head_budget));
    let mut tail = take_suffix_tokens(trimmed, tail_budget);

    // Estimation is intentionally conservative, but make the bound exact for
    // the same estimator in case the marker itself contains CJK text. Remove a
    // token at a time from the larger side so the useful head and tail
    // survive. The marker is rebuilt every round so the continuation offset
    // always points at the first line the model has not seen.
    loop {
        marker = format!(
            "\n…[truncated: {original_tokens} tokens, {line_count} lines omitted]\n{}\n",
            continuation_hint(name, arguments, continuation_offset_for_head(&head))
        );
        let result = format_parts(&head, &marker, &tail);
        if estimate_text_tokens(&result) <= max_tokens || (head.is_empty() && tail.is_empty()) {
            return result;
        }
        if estimate_text_tokens(&head) >= estimate_text_tokens(&tail) && !head.is_empty() {
            let budget = estimate_text_tokens(&head).saturating_sub(1);
            head = snap_prefix_to_complete_lines(&take_prefix_tokens(&head, budget));
        } else {
            let budget = estimate_text_tokens(&tail).saturating_sub(1);
            tail = take_suffix_tokens(&tail, budget);
        }
    }
}

fn format_parts(head: &str, marker: &str, tail: &str) -> String {
    if head.is_empty() {
        format!("{marker}{tail}")
    } else if tail.is_empty() {
        format!("{head}{marker}")
    } else {
        format!("{head}{marker}{tail}")
    }
}

fn snap_prefix_to_complete_lines(head: &str) -> String {
    if head.is_empty() || head.ends_with('\n') || !head.contains('\n') {
        return head.to_string();
    }
    match head.rfind('\n') {
        Some(index) => head[..=index].to_string(),
        None => head.to_string(),
    }
}

fn continuation_offset_for_head(head: &str) -> usize {
    if head.is_empty() || !head.contains('\n') {
        return 1;
    }
    let complete_lines = if head.ends_with('\n') {
        head.lines().count()
    } else {
        head.lines().count().saturating_sub(1)
    };
    complete_lines.saturating_add(1).max(1)
}

fn continuation_hint(name: &str, arguments: &str, next_offset: usize) -> String {
    let args = serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null);
    if let Some(path) = args
        .get("file_path")
        .or_else(|| args.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
    {
        return format!(
            "[继续读取] {name} 输出过长；需要更多内容时调用 Read {{\"file_path\":{path:?},\"offset\":{next_offset}}}。"
        );
    }
    if let Some(pattern) = args.get("pattern").and_then(Value::as_str) {
        return format!(
            "[继续读取] {name} 输出过长；请缩小搜索范围（pattern={}) 或使用分页参数。",
            truncate_for_hint(pattern)
        );
    }
    format!("[继续读取] {name} 输出过长；请使用更窄的查询或分页参数获取剩余内容。")
}

fn truncate_for_hint(text: &str) -> String {
    let mut result = text.chars().take(80).collect::<String>();
    if result.chars().count() < text.chars().count() {
        result.push('…');
    }
    result
}

fn fit_text_tokens(text: &str, max_tokens: usize) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_string();
    }
    take_prefix_tokens(text, max_tokens).trim_end().to_string()
}

fn take_prefix_tokens(text: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
        let candidate_tokens = ascii.saturating_add(3) / 4 + non_ascii + 1;
        if candidate_tokens > max_tokens {
            break;
        }
        result.push(ch);
    }
    result
}

fn take_suffix_tokens(text: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return String::new();
    }
    let mut selected = Vec::new();
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars().rev() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
        let candidate_tokens = ascii.saturating_add(3) / 4 + non_ascii + 1;
        if candidate_tokens > max_tokens {
            break;
        }
        selected.push(ch);
    }
    selected.into_iter().rev().collect()
}

/// Legacy character-budget API retained for callers and old settings files.
/// New code should prefer [`truncate_messages_tokens`].
pub fn truncate_messages(messages: &mut [Message], limit: usize) {
    if limit == 0 {
        return;
    }
    let total: usize = messages.iter().map(message_chars).sum();
    if total <= limit {
        return;
    }
    for index in 0..messages.len() {
        if messages[index].role != Role::Tool {
            continue;
        }
        let current = message_chars(&messages[index]);
        if current <= 240 {
            continue;
        }
        let other_chars: usize = messages
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .map(|(_, message)| message_chars(message))
            .sum();
        let max_chars = limit.saturating_sub(other_chars).max(64);
        let max_tokens = chars_to_tokens(max_chars);
        let name = messages[index].name.clone();
        let original = std::mem::take(&mut messages[index].content);
        messages[index].content = truncate_tool_result(&name, "{}", &original, max_tokens);
        if message_chars(&messages[index]) > limit {
            messages[index].content = "[truncated: tool output omitted]".to_string();
        }
        if messages.iter().map(message_chars).sum::<usize>() <= limit {
            return;
        }
    }
}

/// Token-aware truncation used immediately before a model request. Every tool
/// result gets its own cap first, then old non-system messages are shortened
/// only as a last resort.
pub fn truncate_messages_tokens(
    messages: &mut Vec<Message>,
    limit_tokens: usize,
    tool_result_limit: usize,
) {
    if limit_tokens == 0 {
        return;
    }
    for message in messages.iter_mut() {
        if message.role == Role::Tool {
            let name = message.name.clone();
            let original = std::mem::take(&mut message.content);
            message.content =
                truncate_tool_result(&name, "{}", &original, tool_result_limit.min(limit_tokens));
        }
    }
    // Context resets can leave the retained tail starting at a tool result,
    // even when no further clipping is needed. Sanitize unconditionally so a
    // request never carries an orphaned tool result/call after any trimming.
    sanitize_tool_message_pairs(messages);
    if total_message_tokens(messages) <= limit_tokens {
        return;
    }

    // Preserve system constraints and the latest user request. Older content
    // can be compacted by compact.rs; this final guard prevents an oversized
    // request if a provider returns unusually large assistant text.
    let mut protected = messages
        .iter()
        .rposition(|message| message.role == Role::User);
    if protected.is_none() {
        protected = messages
            .iter()
            .rposition(|message| message.role != Role::System);
    }

    // First shorten eligible messages in proportion to the excess. This keeps
    // useful recent observations when the request is only slightly over the
    // window instead of dropping an entire conversation prefix immediately.
    loop {
        let total = total_message_tokens(messages);
        if total <= limit_tokens {
            return;
        }
        let excess = total.saturating_sub(limit_tokens).max(1);
        let mut changed = false;
        for index in 0..messages.len() {
            if messages[index].role == Role::System || protected == Some(index) {
                continue;
            }
            let current = message_tokens(&messages[index]);
            if current <= 4 {
                continue;
            }
            let target = current.saturating_sub(excess).max(4);
            let before = current;
            messages[index].content =
                fit_text_tokens(&messages[index].content, target.saturating_sub(4));
            messages[index].reasoning_content.clear();
            messages[index].tool_calls.clear();
            clear_images_with_notice(&mut messages[index]);
            changed |= message_tokens(&messages[index]) < before;
            if total_message_tokens(messages) <= limit_tokens {
                return;
            }
        }
        if !changed {
            break;
        }
    }

    // If short messages still exceed the window, discard the oldest history
    // entries. The current user request remains protected; preserving it is
    // more useful than sending an oversized request that the provider rejects.
    while total_message_tokens(messages) > limit_tokens {
        let Some(index) = (0..messages.len())
            .find(|index| messages[*index].role != Role::System && protected != Some(*index))
        else {
            break;
        };
        messages.remove(index);
        if let Some(value) = protected {
            if index < value {
                protected = Some(value.saturating_sub(1));
            } else if index == value {
                protected = messages
                    .iter()
                    .rposition(|message| message.role == Role::User)
                    .or_else(|| {
                        messages
                            .iter()
                            .rposition(|message| message.role != Role::System)
                    });
            }
        }
    }

    // If only protected/system messages remain, clip their textual payloads as
    // the final fallback. Repeat until the aggregate framing and content fit.
    loop {
        let total = total_message_tokens(messages);
        if total <= limit_tokens {
            break;
        }
        let excess = total.saturating_sub(limit_tokens).max(1);
        let mut changed = false;
        for index in 0..messages.len() {
            let current = message_tokens(&messages[index]);
            if current <= 4 {
                continue;
            }
            let target = current.saturating_sub(excess).max(4);
            messages[index].content =
                fit_text_tokens(&messages[index].content, target.saturating_sub(4));
            messages[index].reasoning_content.clear();
            messages[index].tool_calls.clear();
            let skip_protected_images = protected == Some(index)
                && messages[index].role == Role::User
                && !messages[index].images.is_empty();
            if !skip_protected_images {
                clear_images_with_notice(&mut messages[index]);
            }
            changed |= message_tokens(&messages[index]) < current;
            if total_message_tokens(messages) <= limit_tokens {
                break;
            }
        }
        if !changed {
            break;
        }
    }

    if total_message_tokens(messages) > limit_tokens {
        if let Some(index) = protected {
            if index < messages.len() {
                clear_images_with_notice(&mut messages[index]);
            }
        }
    }

    // Truncation can remove one side of an assistant tool-call/tool-result
    // pair (or clear an assistant's calls while retaining its result). Such
    // orphaned messages are rejected by both Chat Completions and Responses,
    // so keep only complete pairs before the request is serialized.
    sanitize_tool_message_pairs(messages);
}

fn clear_images_with_notice(message: &mut Message) {
    if message.images.is_empty() {
        return;
    }
    if !message.content.contains(IMAGE_REMOVED_NOTICE) {
        if message.content.trim().is_empty() {
            message.content = IMAGE_REMOVED_NOTICE.to_string();
        } else {
            message.content.push('\n');
            message.content.push_str(IMAGE_REMOVED_NOTICE);
        }
    }
    message.images.clear();
}

/// Remove orphaned tool messages and assistant tool calls after history
/// trimming. The model protocol requires every retained call to have a
/// preceding matching result, while a result without its call is invalid.
pub fn sanitize_tool_message_pairs(messages: &mut Vec<Message>) {
    let mut seen_calls = HashSet::new();
    let mut valid_calls = HashSet::new();
    for message in messages.iter() {
        match message.role {
            Role::Assistant => {
                for call in &message.tool_calls {
                    if !call.id.is_empty() {
                        seen_calls.insert(call.id.clone());
                    }
                }
            }
            // A result is valid only when its call appeared earlier in
            // the transcript. Checking this while walking forward avoids
            // retaining a result that happens to share an id with a call
            // later in the history.
            Role::Tool
                if !message.tool_call_id.is_empty()
                    && seen_calls.contains(&message.tool_call_id) =>
            {
                valid_calls.insert(message.tool_call_id.clone());
            }
            _ => {}
        }
    }

    let mut retained = Vec::with_capacity(messages.len());
    for mut message in messages.drain(..) {
        match message.role {
            Role::Assistant => {
                message
                    .tool_calls
                    .retain(|call| valid_calls.contains(&call.id));
                if message.content.is_empty()
                    && message.reasoning_content.is_empty()
                    && message.tool_calls.is_empty()
                {
                    continue;
                }
                retained.push(message);
            }
            Role::Tool => {
                if valid_calls.contains(&message.tool_call_id) {
                    retained.push(message);
                }
            }
            _ => retained.push(message),
        }
    }
    *messages = retained;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::model::types::{Message, NativeImage};

    #[test]
    fn estimates_cjk_more_conservatively_than_ascii() {
        assert!(estimate_text_tokens("你好世界") >= 4);
        assert_eq!(estimate_text_tokens("abcd"), 2);
    }

    #[test]
    fn tool_result_keeps_head_tail_and_read_hint() {
        let output = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_tool_result("Read", r#"{"file_path":"src/lib.rs"}"#, &output, 120);
        assert!(result.contains("line 0"));
        assert!(result.contains("[继续读取]"));
        assert!(result.contains("[truncated"));
        assert!(result.contains("\"offset\":"));
        assert!(!result.contains("\"offset\":501"));
        assert!(estimate_text_tokens(&result) <= 120);
    }

    #[test]
    fn continuation_offset_rereads_partial_line() {
        let output = format!("line 1\n{}\nline 3", "abcdefghij".repeat(800));
        let result = truncate_tool_result("Read", r#"{"file_path":"src/lib.rs"}"#, &output, 80);
        assert!(
            result.contains("\"offset\":2"),
            "partial second line must be reread: {result}"
        );
        assert!(
            !result.contains("\"offset\":3"),
            "must not skip the remainder of line 2: {result}"
        );
    }

    #[test]
    fn continuation_offset_on_single_oversized_line_starts_at_one() {
        let output = "x".repeat(8_000);
        let result = truncate_tool_result("Read", r#"{"file_path":"src/lib.rs"}"#, &output, 80);
        assert!(
            result.contains("\"offset\":1"),
            "single truncated line must continue at offset 1: {result}"
        );
    }

    #[test]
    fn continuation_offset_points_to_first_unseen_line() {
        let output = (1..=500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_tool_result("Read", r#"{"file_path":"src/lib.rs"}"#, &output, 120);
        let marker_at = result.find("\n…[truncated").expect("marker present");
        let retained_head_lines = result[..marker_at].lines().count();
        assert!(
            result.contains(&format!("\"offset\":{}", retained_head_lines + 1)),
            "offset must continue right after the retained head: {result}"
        );
    }

    #[test]
    fn shrinks_old_tool_results() {
        let mut messages = vec![
            Message::user("hi"),
            Message::tool_result("1", "x".repeat(800)),
            Message::user("next"),
        ];
        truncate_messages(&mut messages, 400);
        assert!(messages[1].content.contains("[truncated"));
        assert!(message_chars(&messages[1]) < 800);
    }

    #[test]
    fn token_budget_caps_each_tool_result() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("hi"),
            Message::tool_result("1", "x".repeat(20_000)),
            Message::user("next"),
        ];
        truncate_messages_tokens(&mut messages, 300, 80);
        assert!(total_message_tokens(&messages) <= 300);
        assert!(message_chars(&messages[2]) < 20_000);
    }

    #[test]
    fn token_budget_drops_old_short_messages_when_framing_exceeds_window() {
        let mut messages = vec![Message::system("rules"), Message::user("old task")];
        for index in 0..32 {
            messages.push(Message::assistant_text(format!("step {index}")));
        }
        messages.push(Message::user("current task"));
        truncate_messages_tokens(&mut messages, 40, 80);
        assert!(total_message_tokens(&messages) <= 40);
        assert!(messages
            .iter()
            .any(|message| message.content == "current task"));
        assert!(messages.iter().any(|message| message.content == "rules"));
    }

    #[test]
    fn truncation_keeps_tool_call_and_result_pairs_together() {
        let mut messages = vec![
            Message::system("rules"),
            Message::user("task"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![crate::native::model::types::ToolCall {
                    id: "call_1".to_string(),
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
            },
            Message::tool_result("call_1", "result"),
            Message::assistant_text("done"),
        ];
        sanitize_tool_message_pairs(&mut messages);
        assert!(messages.iter().any(|message| {
            message.role == Role::Assistant
                && message.tool_calls.iter().any(|call| call.id == "call_1")
        }));
        assert!(messages
            .iter()
            .any(|message| message.role == Role::Tool && message.tool_call_id == "call_1"));

        messages.remove(3);
        sanitize_tool_message_pairs(&mut messages);
        assert!(!messages.iter().any(|message| {
            message.role == Role::Assistant
                && message.tool_calls.iter().any(|call| call.id == "call_1")
        }));
        assert!(!messages
            .iter()
            .any(|message| message.role == Role::Tool && message.tool_call_id == "call_1"));
    }

    #[test]
    fn sanitization_requires_call_to_precede_result() {
        let mut messages = vec![
            Message::system("rules"),
            Message::user("task"),
            Message::tool_result("call_late", "orphan"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![crate::native::model::types::ToolCall {
                    id: "call_late".to_string(),
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
            },
        ];
        sanitize_tool_message_pairs(&mut messages);
        assert!(!messages.iter().any(|message| message.role == Role::Tool));
        assert!(!messages
            .iter()
            .any(|message| { message.role == Role::Assistant && !message.tool_calls.is_empty() }));
    }

    #[test]
    fn sanitization_runs_even_when_history_is_under_limit() {
        let mut messages = vec![Message::system("rules"), Message::user("task")];
        messages.push(Message::tool_result("missing", "orphan"));
        truncate_messages_tokens(&mut messages, 10_000, 4_096);
        assert!(!messages.iter().any(|message| message.role == Role::Tool));
    }

    #[test]
    fn megabyte_image_is_not_estimated_as_base64_text() {
        let mut message = Message::user("see");
        message.images.push(NativeImage {
            name: "shot.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "A".repeat(1_400_000),
        });
        assert!(message_tokens(&message) < 5_000);
    }

    #[test]
    fn protected_user_image_survives_128k_window() {
        let mut message = Message::user("look");
        message.images.push(NativeImage {
            name: "shot.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "A".repeat(100_000),
        });
        let mut messages = vec![Message::system("sys"), message];
        truncate_messages_tokens(&mut messages, 128_000, 80);
        assert!(!messages.last().expect("user").images.is_empty());
        assert!(!messages
            .last()
            .expect("user")
            .content
            .contains(IMAGE_REMOVED_NOTICE));
    }
}
