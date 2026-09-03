use serde_json::{Map, Value};

use super::retry::redact_secrets;
use super::sse::{parse_sse, SseEvent};
use super::types::Usage;
use super::usage::parse_usage;

pub const CALL_LOG_BODY_CHAR_LIMIT: usize = 64 * 1024;
pub const CALL_KIND_CHAT: &str = "chat";
pub const CALL_KIND_COMPACT: &str = "compact";
pub const CALL_KIND_ONE_SHOT: &str = "one_shot";
pub const CALL_KIND_SUBAGENT: &str = "subagent";
pub const CALL_KIND_PLAN: &str = "plan";
pub const CALL_STATUS_SUCCESS: &str = "success";
pub const CALL_STATUS_FAILED: &str = "failed";
pub const CALL_STATUS_CANCELLED: &str = "cancelled";
pub const RESPONSE_ENCODING_SSE: &str = "sse";
pub const RESPONSE_ENCODING_JSON: &str = "json";
pub const RESPONSE_ENCODING_UNKNOWN: &str = "unknown";

const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "access_token",
    "accesstoken",
    "authorization",
    "x-api-key",
    "password",
    "secret",
    "token",
];

/// 调用用途（比 `call_kind` 更细）：写入 `native_api_call_logs.operation`。
pub const OPERATION_AGENT_STEP: &str = "agent_step";
pub const OPERATION_COMPACT: &str = "compact";
pub const OPERATION_MEMORY_EXTRACT: &str = "memory_extract";
pub const OPERATION_MEMORY_DREAM: &str = "memory_dream";
pub const OPERATION_HOOK_AGENT: &str = "hook_agent";
pub const OPERATION_SUBAGENT: &str = "subagent";
pub const OPERATION_ONE_SHOT: &str = "one_shot";
/// 模型角色：主模型或轻量模型。
pub const MODEL_ROLE_MAIN: &str = "main";
pub const MODEL_ROLE_LITE: &str = "lite";

#[derive(Debug, Clone, Default)]
pub struct CallLogContext {
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub workspace_id: Option<String>,
    pub subagent_id: Option<String>,
    pub call_kind: Option<String>,
    pub execution_target: Option<String>,
    /// 为空时按 `agent_step` 记录。
    pub operation: Option<String>,
    /// 为空时按 `main` 记录。
    pub model_role: Option<String>,
}

impl CallLogContext {
    pub fn with_call_kind(mut self, call_kind: impl Into<String>) -> Self {
        self.call_kind = Some(call_kind.into());
        self
    }

    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    pub fn with_model_role(mut self, role: impl Into<String>) -> Self {
        self.model_role = Some(role.into());
        self
    }

    pub fn with_subagent_id(mut self, subagent_id: impl Into<String>) -> Self {
        self.subagent_id = Some(subagent_id.into());
        self
    }

    pub fn for_session(
        channel_id: Option<String>,
        channel_name: Option<String>,
        session_id: Option<String>,
        profile_id: Option<String>,
        workspace_id: Option<String>,
        call_kind: &str,
        execution_target: Option<String>,
    ) -> Self {
        Self {
            channel_id,
            channel_name,
            session_id,
            profile_id,
            workspace_id,
            subagent_id: None,
            call_kind: Some(call_kind.to_string()),
            execution_target,
            operation: None,
            model_role: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeApiCallLogInsert {
    pub id: String,
    pub call_id: String,
    pub attempt: i64,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub protocol: String,
    pub response_encoding: Option<String>,
    pub model: Option<String>,
    pub thinking_enabled: bool,
    pub thinking_level: Option<String>,
    pub request_format: String,
    pub request_body: Option<String>,
    pub request_truncated: bool,
    pub response_body: Option<String>,
    pub response_truncated: bool,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub http_status: Option<i64>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub workspace_id: Option<String>,
    pub subagent_id: Option<String>,
    pub call_kind: Option<String>,
    pub execution_target: Option<String>,
    pub operation: String,
    pub model_role: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggedUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedBody {
    pub text: String,
    pub truncated: bool,
}

pub fn sanitize_json_body(value: &Value) -> Value {
    redact_json_value(value)
}

pub fn redact_and_truncate_json(value: &Value) -> RedactedBody {
    let sanitized = sanitize_json_body(value);
    let serialized = serde_json::to_string(&sanitized).unwrap_or_else(|_| sanitized.to_string());
    truncate_logged_text(&redact_secrets(&serialized))
}

pub fn redact_and_truncate_text(text: &str) -> RedactedBody {
    let sanitized = match serde_json::from_str::<Value>(text) {
        Ok(value) => {
            serde_json::to_string(&sanitize_json_body(&value)).unwrap_or_else(|_| text.to_string())
        }
        Err(_) => omit_inline_images(text),
    };
    truncate_logged_text(&redact_secrets(&sanitized))
}

pub fn truncate_logged_text(text: &str) -> RedactedBody {
    let count = text.chars().count();
    if count <= CALL_LOG_BODY_CHAR_LIMIT {
        return RedactedBody {
            text: text.to_string(),
            truncated: false,
        };
    }
    let mut truncated: String = text.chars().take(CALL_LOG_BODY_CHAR_LIMIT).collect();
    truncated.push_str("\n...[truncated]");
    RedactedBody {
        text: truncated,
        truncated: true,
    }
}

pub fn logged_usage_from_parsed(usage: Usage, provider_reported: bool) -> LoggedUsage {
    if !provider_reported {
        return LoggedUsage {
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            total_tokens: None,
        };
    }
    let input = i64::from(usage.prompt_tokens);
    let output = i64::from(usage.completion_tokens);
    LoggedUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        cached_tokens: Some(i64::from(usage.cached_tokens)),
        total_tokens: Some(input.saturating_add(output)),
    }
}

pub fn provider_reported_usage(text: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}').trim()) {
        if json_has_usage_object(&value) {
            return true;
        }
    }
    parse_sse(text)
        .into_iter()
        .any(|event| json_has_usage_object_str(&event.data))
}

pub fn detect_response_encoding(text: &str) -> &'static str {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return RESPONSE_ENCODING_UNKNOWN;
    }
    if looks_like_sse(trimmed) {
        return RESPONSE_ENCODING_SSE;
    }
    if serde_json::from_str::<Value>(trimmed).is_ok() {
        return RESPONSE_ENCODING_JSON;
    }
    RESPONSE_ENCODING_UNKNOWN
}

pub fn first_meaningful_event_offset(buffer: &str) -> Option<usize> {
    let trimmed = buffer.trim_start_matches('\u{feff}');
    if trimmed.trim().is_empty() {
        return None;
    }
    if looks_like_sse(trimmed) {
        return first_meaningful_sse_offset(trimmed);
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed.trim()) {
        if json_has_message_content(&value) {
            return Some(0);
        }
    }
    None
}

/// Single-event view of [`first_meaningful_event_offset`], used while the
/// response is still streaming in. Keep-alive comments never reach this
/// point, and `[DONE]` / usage-only events stay excluded.
pub fn sse_event_is_meaningful(event: &SseEvent) -> bool {
    sse_data_is_meaningful(&event.event, std::slice::from_ref(&event.data))
}

/// Single-event view of [`provider_reported_usage`].
pub fn sse_event_reports_usage(event: &SseEvent) -> bool {
    json_has_usage_object_str(&event.data)
}

fn first_meaningful_sse_offset(text: &str) -> Option<usize> {
    let mut event_name = String::new();
    let mut data_lines: Vec<String> = Vec::new();
    let mut event_start: Option<usize> = None;
    let mut offset = 0usize;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        let trimmed_line = line.trim_start();
        if event_start.is_none() && !trimmed_line.is_empty() && !trimmed_line.starts_with(':') {
            event_start = Some(offset);
        }
        if trimmed_line.is_empty() {
            if sse_data_is_meaningful(&event_name, &data_lines) {
                return event_start;
            }
            event_name.clear();
            data_lines.clear();
            event_start = None;
            offset += raw.len();
            continue;
        }
        if trimmed_line.starts_with(':') {
            offset += raw.len();
            continue;
        }
        if let Some(value) = trimmed_line.strip_prefix("event:") {
            event_name = value.trim().to_string();
            offset += raw.len();
            continue;
        }
        if let Some(value) = trimmed_line.strip_prefix("data:") {
            let data = value.trim_start().to_string();
            if data.trim() == "[DONE]" || is_complete_json_line(&data) {
                if !data_lines.is_empty() && sse_data_is_meaningful(&event_name, &data_lines) {
                    return event_start;
                }
                data_lines.push(data);
                if sse_data_is_meaningful(&event_name, &data_lines) {
                    return event_start;
                }
                event_name.clear();
                data_lines.clear();
                event_start = None;
                offset += raw.len();
                continue;
            }
            data_lines.push(data);
        }
        offset += raw.len();
    }
    if sse_data_is_meaningful(&event_name, &data_lines) {
        event_start
    } else {
        None
    }
}

fn sse_data_is_meaningful(event_name: &str, data_lines: &[String]) -> bool {
    if data_lines.is_empty() {
        return false;
    }
    let data = data_lines.join("\n");
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return false;
    }
    if event_name.eq_ignore_ascii_case("ping") {
        return false;
    }
    json_has_message_content_str(trimmed)
}

fn looks_like_sse(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("data:") || trimmed.starts_with("event:")
    })
}

fn is_complete_json_line(data: &str) -> bool {
    let trimmed = data.trim();
    if trimmed == "[DONE]" {
        return true;
    }
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    serde_json::from_str::<Value>(trimmed).is_ok()
}

fn json_has_usage_object_str(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|value| json_has_usage_object(&value))
}

fn json_has_usage_object(value: &Value) -> bool {
    if value.get("usage").is_some() {
        return true;
    }
    if value.pointer("/message/usage").is_some() {
        return true;
    }
    if value.pointer("/response/usage").is_some() {
        return true;
    }
    match value {
        Value::Object(map) => map.values().any(json_has_usage_object),
        Value::Array(items) => items.iter().any(json_has_usage_object),
        _ => false,
    }
}

fn json_has_message_content_str(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|value| json_has_message_content(&value))
}

fn json_has_message_content(value: &Value) -> bool {
    if is_usage_only_object(value) {
        return false;
    }
    if has_nonempty_text(value.get("content"))
        || has_nonempty_text(value.get("text"))
        || has_nonempty_text(value.get("reasoning"))
        || has_nonempty_text(value.get("reasoning_content"))
        || has_nonempty_text(value.get("thinking"))
        || has_nonempty_text(value.get("delta"))
        || has_tool_calls(value)
    {
        return true;
    }
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| *key != "usage")
            .any(|(_, child)| json_has_message_content(child)),
        Value::Array(items) => items.iter().any(json_has_message_content),
        _ => false,
    }
}

fn is_usage_only_object(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    if map.is_empty() {
        return false;
    }
    map.keys().all(|key| {
        matches!(
            key.as_str(),
            "usage"
                | "id"
                | "object"
                | "created"
                | "model"
                | "system_fingerprint"
                | "type"
                | "event"
                | "response"
        )
    }) && map.contains_key("usage")
        && !has_tool_calls(value)
        && !has_nonempty_text(value.get("content"))
        && !has_nonempty_text(value.get("text"))
        && !has_nonempty_text(value.get("delta"))
}

fn has_tool_calls(value: &Value) -> bool {
    value
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || value.get("function_call").is_some()
        || value
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty())
            && value.get("arguments").is_some()
}

fn has_nonempty_text(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(parts)) => parts.iter().any(|part| has_nonempty_text(Some(part))),
        Some(Value::Object(map)) => map.iter().any(|(key, child)| {
            matches!(
                key.as_str(),
                "content"
                    | "text"
                    | "output_text"
                    | "reasoning"
                    | "reasoning_content"
                    | "thinking"
                    | "delta"
                    | "summary"
            ) && has_nonempty_text(Some(child))
        }),
        _ => false,
    }
}

fn redact_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_json_map(map)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json_value).collect()),
        Value::String(text) => Value::String(omit_inline_images(text)),
        other => other.clone(),
    }
}

fn redact_json_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if is_sensitive_key(key) {
            out.insert(key.clone(), Value::String("[redacted]".to_string()));
            continue;
        }
        if is_image_payload_key(key) {
            out.insert(key.clone(), omit_image_value(value));
            continue;
        }
        out.insert(key.clone(), redact_json_value(value));
    }
    out
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace('-', "_");
    SENSITIVE_KEYS.iter().any(|item| {
        let normalized = item.replace('-', "_");
        lower == normalized || lower.replace('_', "") == normalized.replace('_', "")
    })
}

fn is_image_payload_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "image_url" | "source" | "data_url" | "data" | "url" | "b64_json"
    )
}

fn omit_image_value(value: &Value) -> Value {
    match value {
        Value::String(text) if looks_like_image_payload(text) => {
            Value::String("[omitted image]".to_string())
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                if is_image_payload_key(key) {
                    out.insert(key.clone(), omit_image_value(child));
                } else {
                    out.insert(key.clone(), redact_json_value(child));
                }
            }
            Value::Object(out)
        }
        other => redact_json_value(other),
    }
}

fn looks_like_image_payload(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("data:image/")
        || trimmed.starts_with("data:") && trimmed.contains(";base64,")
        || (trimmed.len() > 128
            && trimmed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '/' || ch == '='))
}

fn omit_inline_images(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("data:") {
        out.push_str(&remaining[..start]);
        let rest = &remaining[start..];
        if rest.starts_with("data:image/") || rest.contains(";base64,") {
            out.push_str("[omitted image]");
            let skip = rest.find('"').unwrap_or(rest.len());
            remaining = &rest[skip..];
            continue;
        }
        out.push_str("data:");
        remaining = &rest["data:".len()..];
    }
    out.push_str(remaining);
    out
}

pub fn extract_request_model(body: &Value) -> Option<String> {
    body.get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
}

fn thinking_level_from_budget_tokens(budget: u64) -> &'static str {
    match budget {
        0..=2048 => "low",
        2049..=8192 => "medium",
        8193..=16384 => "high",
        16385..=32768 => "xhigh",
        _ => "max",
    }
}

pub fn extract_thinking_level(body: &Value) -> Option<String> {
    body.get("reasoning_effort")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/reasoning/effort").and_then(Value::as_str))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            body.pointer("/thinking/budget_tokens")
                .and_then(Value::as_u64)
                .filter(|budget| *budget > 0)
                .map(thinking_level_from_budget_tokens)
                .map(ToOwned::to_owned)
        })
}

pub fn request_thinking_enabled(body: &Value, thinking_level: Option<&str>) -> bool {
    if thinking_level.is_some() {
        return true;
    }
    match body.get("thinking") {
        Some(Value::Object(map)) => map
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.eq_ignore_ascii_case("disabled")),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("disabled")
        }
        Some(Value::Bool(enabled)) => *enabled,
        _ => false,
    }
}

pub fn parse_usage_from_body(text: &str) -> Usage {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}').trim()) {
        if let Some(usage) = first_usage_value(&value) {
            return parse_usage(usage);
        }
    }
    for event in parse_sse(text) {
        let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
            continue;
        };
        if let Some(usage) = first_usage_value(&value) {
            return parse_usage(usage);
        }
    }
    Usage::default()
}

fn first_usage_value(value: &Value) -> Option<&Value> {
    if let Some(usage) = value.get("usage") {
        return Some(usage);
    }
    if let Some(usage) = value.pointer("/message/usage") {
        return Some(usage);
    }
    if let Some(usage) = value.pointer("/response/usage") {
        return Some(usage);
    }
    match value {
        Value::Object(map) => map.values().find_map(first_usage_value),
        Value::Array(items) => items.iter().find_map(first_usage_value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_secret_fields_and_bearer_tokens() {
        let body = json!({
            "api_key": "sk-secret-key",
            "Authorization": "Bearer sk-secret-key",
            "model": "gpt-4o",
            "messages": [{"role":"user","content":"hi"}]
        });
        let redacted = redact_and_truncate_json(&body);
        assert!(!redacted.text.contains("sk-secret-key"));
        assert!(!redacted.text.to_ascii_lowercase().contains("bearer sk"));
        assert!(redacted.text.contains("[redacted]"));
        assert!(!redacted.truncated);
    }

    #[test]
    fn omits_base64_images() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": {"url": "data:image/png;base64,AAAA"}
                }]
            }]
        });
        let redacted = redact_and_truncate_json(&body);
        assert!(redacted.text.contains("[omitted image]"));
        assert!(!redacted.text.contains("AAAA"));
    }

    #[test]
    fn truncates_over_limit_and_marks_truncated() {
        let long = "x".repeat(CALL_LOG_BODY_CHAR_LIMIT + 8);
        let redacted = redact_and_truncate_text(&long);
        assert!(redacted.truncated);
        assert!(redacted.text.contains("[truncated]"));
        assert!(redacted.text.ends_with("[truncated]"));
        assert_eq!(
            redacted.text.chars().count(),
            CALL_LOG_BODY_CHAR_LIMIT + "\n...[truncated]".chars().count()
        );
    }

    #[test]
    fn unknown_usage_stays_null() {
        let usage = logged_usage_from_parsed(Usage::default(), false);
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.cached_tokens, None);
        assert_eq!(usage.total_tokens, None);
    }

    #[test]
    fn cached_tokens_are_kept_when_provider_reports_usage() {
        let usage = logged_usage_from_parsed(
            Usage {
                prompt_tokens: 10,
                completion_tokens: 4,
                cached_tokens: 3,
            },
            true,
        );
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.cached_tokens, Some(3));
        assert_eq!(usage.total_tokens, Some(14));
    }

    #[test]
    fn first_token_ignores_sse_comments_done_and_usage_only() {
        let usage_only = "data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n";
        assert!(first_meaningful_event_offset(usage_only).is_none());
        assert!(first_meaningful_event_offset("data: [DONE]\n\n").is_none());
        assert!(first_meaningful_event_offset(": keep-alive\n\n").is_none());

        let sse = ": keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n";
        assert!(first_meaningful_event_offset(sse).is_some());
    }

    #[test]
    fn complete_json_with_message_is_meaningful() {
        let json = r#"{"choices":[{"message":{"role":"assistant","content":"json ok"}}]}"#;
        assert_eq!(first_meaningful_event_offset(json), Some(0));
    }

    #[test]
    fn first_token_detects_anthropic_and_responses_deltas() {
        let anthropic = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n";
        assert!(first_meaningful_event_offset(anthropic).is_some());

        let responses = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n";
        assert!(first_meaningful_event_offset(responses).is_some());
    }

    #[test]
    fn thinking_object_disabled_is_not_enabled() {
        let disabled = json!({"thinking":{"type":"disabled"},"model":"deepseek-v4-flash"});
        assert_eq!(extract_thinking_level(&disabled), None);
        assert!(!request_thinking_enabled(&disabled, None));

        let enabled = json!({"thinking":{"type":"enabled"},"reasoning_effort":"high"});
        assert_eq!(extract_thinking_level(&enabled).as_deref(), Some("high"));
        assert!(request_thinking_enabled(
            &enabled,
            extract_thinking_level(&enabled).as_deref()
        ));
    }

    #[test]
    fn anthropic_budget_tokens_map_to_thinking_level() {
        let anthropic = json!({"thinking":{"type":"enabled","budget_tokens":16384}});
        assert_eq!(extract_thinking_level(&anthropic).as_deref(), Some("high"));
        assert!(request_thinking_enabled(
            &anthropic,
            extract_thinking_level(&anthropic).as_deref()
        ));

        let max = json!({"thinking":{"type":"enabled","budget_tokens":64000}});
        assert_eq!(extract_thinking_level(&max).as_deref(), Some("max"));
    }

    #[test]
    fn redacts_camel_case_and_access_token_keys() {
        let body = json!({
            "apiKey": "sk-secret-key",
            "access_token": "tok-secret",
            "model": "gpt-4o"
        });
        let redacted = redact_and_truncate_json(&body);
        assert!(!redacted.text.contains("sk-secret-key"));
        assert!(!redacted.text.contains("tok-secret"));
        assert!(redacted.text.contains("[redacted]"));
    }
}
