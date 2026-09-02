use serde_json::{json, Value};

use super::sse::{parse_sse, SseEvent};
use super::types::{Message, Role, StreamDelta, ToolCall, ToolSpec, Usage};
use super::usage::parse_usage;

pub fn build_openai_body(
    messages: &[Message],
    tools: &[ToolSpec],
    model: &str,
    effort: Option<&str>,
    max_output_tokens: Option<u32>,
    thinking_enabled: bool,
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": openai_messages(messages),
        "stream": stream,
    });
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    if !tools.is_empty() {
        body["tools"] = json!(openai_tools(tools));
        body["tool_choice"] = json!("auto");
    }
    if let Some(max_tokens) = max_output_tokens.filter(|value| *value > 0) {
        body["max_tokens"] = json!(max_tokens);
        if thinking_enabled {
            body["max_completion_tokens"] = json!(max_tokens);
        }
    }
    // DeepSeek V4 defaults thinking ON. OpenAI-format toggle is `thinking.type`;
    // omitting it keeps thinking enabled even when our catalog says off.
    if model_uses_thinking_toggle(model) {
        body["thinking"] = json!({
            "type": if thinking_enabled { "enabled" } else { "disabled" }
        });
    }
    if thinking_enabled {
        if let Some(level) = normalize_effort(effort) {
            body["reasoning_effort"] = json!(level);
        }
    }
    body
}

pub fn model_uses_thinking_toggle(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.contains("deepseek")
}

pub fn openai_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| match message.role {
            Role::Assistant => {
                let mut item = json!({"role": "assistant"});
                if !message.reasoning_content.is_empty() {
                    item["reasoning_content"] = json!(message.reasoning_content);
                }
                if !message.tool_calls.is_empty() {
                    item["tool_calls"] = json!(message
                        .tool_calls
                        .iter()
                        .map(|call| json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments,
                            }
                        }))
                        .collect::<Vec<_>>());
                }
                let need_content = !message.content.is_empty()
                    || !message.reasoning_content.is_empty()
                    || message.tool_calls.is_empty();
                if need_content {
                    item["content"] = json!(message.content);
                }
                item
            }
            Role::Tool => {
                let mut item = json!({
                    "role": "tool",
                    "content": message.content,
                    "tool_call_id": message.tool_call_id,
                });
                if !message.name.is_empty() {
                    item["name"] = json!(message.name);
                }
                item
            }
            Role::System => json!({"role": "system", "content": message.content}),
            Role::User => json!({
                "role": "user",
                "content": openai_user_content(message),
            }),
        })
        .collect()
}

fn openai_user_content(message: &Message) -> Value {
    if message.images.is_empty() {
        return json!(message.content);
    }
    let mut parts = vec![json!({"type": "text", "text": message.content})];
    for image in &message.images {
        parts.push(json!({
            "type": "image_url",
            "image_url": {"url": image.data_url()},
        }));
    }
    json!(parts)
}

pub fn openai_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

pub fn parse_openai_sse(text: &str) -> Result<(Message, Usage), String> {
    let mut state = OpenAiStreamState::new();
    for event in parse_sse(text) {
        state.apply(&event);
    }
    state.finish()
}

/// Incremental counterpart of [`parse_openai_sse`]: the same accumulation
/// rules applied one event at a time, plus the deltas that can be shown
/// before the response completes.
#[derive(Debug)]
pub struct OpenAiStreamState {
    message: Message,
    usage: Usage,
    tools: Vec<(i64, ToolCall)>,
    done: bool,
}

impl Default for OpenAiStreamState {
    fn default() -> Self {
        Self {
            message: Message::assistant_text(""),
            usage: Usage::default(),
            tools: Vec::new(),
            done: false,
        }
    }
}

impl OpenAiStreamState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &SseEvent) -> Vec<StreamDelta> {
        if self.done {
            return Vec::new();
        }
        if event.data == "[DONE]" {
            self.done = true;
            return Vec::new();
        }
        let Ok(chunk) = serde_json::from_str::<Value>(&event.data) else {
            return Vec::new();
        };
        if let Some(raw_usage) = chunk.get("usage") {
            self.usage = parse_usage(raw_usage);
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            return Vec::new();
        };
        apply_openai_choice(&mut self.message, &mut self.tools, choice)
    }

    pub fn finish(mut self) -> Result<(Message, Usage), String> {
        self.tools.sort_by_key(|(index, _)| *index);
        self.message.tool_calls = self.tools.into_iter().map(|(_, call)| call).collect();
        if message_is_empty(&self.message) {
            return Err("模型返回空响应".to_string());
        }
        Ok((self.message, self.usage))
    }
}

pub fn parse_openai_json(value: &Value) -> Result<(Message, Usage), String> {
    let mut message = Message::assistant_text("");
    let usage = value.get("usage").map(parse_usage).unwrap_or_default();
    let mut tools: Vec<(i64, ToolCall)> = Vec::new();
    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
    else {
        return Err("模型返回空响应".to_string());
    };
    apply_openai_choice(&mut message, &mut tools, choice);
    tools.sort_by_key(|(index, _)| *index);
    message.tool_calls = tools.into_iter().map(|(_, call)| call).collect();
    if message_is_empty(&message) {
        return Err("模型返回空响应".to_string());
    }
    Ok((message, usage))
}

fn message_is_empty(message: &Message) -> bool {
    message.content.is_empty()
        && message.reasoning_content.is_empty()
        && message.tool_calls.is_empty()
}

fn apply_openai_choice(
    message: &mut Message,
    tools: &mut Vec<(i64, ToolCall)>,
    choice: &Value,
) -> Vec<StreamDelta> {
    if let Some(delta) = choice.get("delta") {
        let mut deltas = append_openai_delta(message, tools, delta);
        if message.content.is_empty() {
            if let Some(complete) = choice.get("message") {
                if let Some(text) = delta_text(complete.get("content")) {
                    message.content.push_str(&text);
                    deltas.push(StreamDelta::Text(text));
                }
            }
        }
        return deltas;
    }
    if let Some(complete) = choice.get("message") {
        return append_openai_delta(message, tools, complete);
    }
    Vec::new()
}

fn append_openai_delta(
    message: &mut Message,
    tools: &mut Vec<(i64, ToolCall)>,
    delta: &Value,
) -> Vec<StreamDelta> {
    let mut deltas = Vec::new();
    if let Some(text) = delta_text(delta.get("content")) {
        message.content.push_str(&text);
        deltas.push(StreamDelta::Text(text));
    }
    if let Some(text) = reasoning_text(delta.get("reasoning_content"))
        .or_else(|| reasoning_text(delta.get("reasoning")))
    {
        message.reasoning_content.push_str(&text);
        deltas.push(StreamDelta::Reasoning(text));
    }
    let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) else {
        return deltas;
    };
    for call in calls {
        let index = call
            .get("index")
            .and_then(Value::as_i64)
            .unwrap_or(tools.len() as i64);
        let slot = if let Some(existing) = tools.iter_mut().find(|(item, _)| *item == index) {
            &mut existing.1
        } else {
            tools.push((index, ToolCall::default()));
            &mut tools.last_mut().expect("just pushed").1
        };
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                slot.id = id.to_string();
            }
        }
        if let Some(name) = call
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            if !name.is_empty() {
                slot.name = name.to_string();
            }
        }
        if let Some(arguments) = call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
        {
            slot.arguments.push_str(arguments);
        }
    }
    deltas
}

fn delta_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.as_str() {
                    out.push_str(text);
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                    continue;
                }
                if let Some(text) = part.get("output_text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

fn reasoning_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Object(_) => object_text(value),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.as_str().filter(|item| !item.is_empty()) {
                    out.push_str(text);
                } else if let Some(text) = object_text(part) {
                    out.push_str(&text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

fn object_text(value: &Value) -> Option<String> {
    for key in ["content", "text", "summary"] {
        match value.get(key) {
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            Some(Value::Array(parts)) => {
                let mut out = String::new();
                for part in parts {
                    if let Some(text) = part.as_str() {
                        out.push_str(text);
                    } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                        out.push_str(text);
                    }
                }
                if !out.is_empty() {
                    return Some(out);
                }
            }
            _ => {}
        }
    }
    None
}

fn take_u32_after(haystack: &str, needle: &str) -> Option<u32> {
    let index = haystack.find(needle)?;
    let digits: String = haystack[index + needle.len()..]
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok().filter(|value| *value > 0)
}

/// Parse gateway errors such as:
/// `max_tokens is too large: 384000. This model supports at most 131072 completion tokens.`
pub fn parse_max_output_token_limit(error_body: &str) -> Option<u32> {
    let lower = error_body.to_ascii_lowercase();
    let mentions_max = [
        "max_token",
        "max_output_token",
        "max_completion_token",
        "too large",
        "too_large",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if !mentions_max {
        return None;
    }
    for needle in [
        "supports at most ",
        "maximum of ",
        "maximum is ",
        "at most ",
    ] {
        if let Some(value) = take_u32_after(&lower, needle) {
            return Some(value);
        }
    }
    None
}

pub fn normalize_effort(effort: Option<&str>) -> Option<&'static str> {
    match effort
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "off" | "none" | "disabled" => None,
        "minimal" | "min" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" | "ultra" => Some("max"),
        _ => Some("medium"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::model::types::ToolCall;

    fn read_tool() -> ToolSpec {
        ToolSpec {
            name: "Read".to_string(),
            description: "Read a file".to_string(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        }
    }

    #[test]
    fn parses_text_and_tool_call_sse() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"Read\",\"arguments\":\"{\\\"path\\\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"a.rs\\\"}\"}}]}}]}\n\n",
            "data: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let (message, usage) = parse_openai_sse(sse).expect("parse openai sse");
        assert_eq!(message.content, "hi ");
        assert_eq!(message.tool_calls[0].id, "call_1");
        assert_eq!(message.tool_calls[0].name, "Read");
        assert_eq!(message.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 5);
    }

    #[test]
    fn second_round_includes_tool_result() {
        let messages = vec![
            Message::user("read it"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "Read".to_string(),
                    arguments: r#"{"path":"a.rs"}"#.to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
            },
            Message::tool_result("call_1", "fn main() {}"),
        ];
        let body = build_openai_body(
            &messages,
            &[read_tool()],
            "gpt-4o",
            Some("high"),
            Some(16384),
            true,
            true,
        );
        let wire = body["messages"].as_array().expect("messages");
        assert_eq!(wire[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(wire[2]["role"], "tool");
        assert_eq!(wire[2]["tool_call_id"], "call_1");
        assert_eq!(wire[2]["content"], "fn main() {}");
        assert_eq!(body["tools"][0]["function"]["name"], "Read");
    }

    #[test]
    fn deepseek_thinking_toggle_is_explicit() {
        let on = build_openai_body(
            &[],
            &[],
            "deepseek-v4-pro",
            Some("max"),
            Some(8192),
            true,
            true,
        );
        assert_eq!(on["thinking"]["type"], "enabled");
        assert_eq!(on["reasoning_effort"], "max");

        let off = build_openai_body(&[], &[], "deepseek-v4-pro", None, Some(8192), false, false);
        assert_eq!(off["thinking"]["type"], "disabled");
        assert!(off.get("reasoning_effort").is_none());

        let gpt = build_openai_body(&[], &[], "gpt-4o", None, None, false, false);
        assert!(gpt.get("thinking").is_none());
    }

    #[test]
    fn prefers_message_content_when_delta_is_empty() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"},\"message\":{\"content\":\"plan json\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (message, _) = parse_openai_sse(sse).expect("parse message fallback");
        assert_eq!(message.reasoning_content, "think");
        assert_eq!(message.content, "plan json");
    }

    #[test]
    fn normalize_effort_keeps_xhigh_and_max() {
        assert_eq!(normalize_effort(Some("xhigh")), Some("xhigh"));
        assert_eq!(normalize_effort(Some("max")), Some("max"));
        assert_eq!(normalize_effort(Some("none")), None);
    }

    #[test]
    fn user_message_with_image_uses_image_url_parts() {
        let mut user = Message::user("see this");
        user.images.push(crate::native::model::types::NativeImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
        });
        let body = build_openai_body(&[user], &[], "gpt-4o", None, None, false, false);
        let content = body["messages"][0]["content"].as_array().expect("parts");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QQ==");
    }

    #[test]
    fn parse_max_output_token_limit_reads_gateway_cap() {
        let raw = r#"{"error":{"message":"Error from provider (Console Go): Upstream request failed: [bad_request] bad request: max_tokens is too large: 384000. This model supports at most 131072 completion tokens."}}"#;
        assert_eq!(parse_max_output_token_limit(raw), Some(131072));

        let truncated = "模型请求失败（HTTP 400）: {\"error\":{\"message\":\"max_tokens is too large: 384000. This model supports at most 131072 compl";
        assert_eq!(parse_max_output_token_limit(truncated), Some(131072));

        assert_eq!(
            parse_max_output_token_limit("max_tokens is too large: 384000"),
            None
        );
        assert_eq!(parse_max_output_token_limit("unauthorized"), None);
    }

    #[test]
    fn parses_reasoning_only_json_is_not_empty() {
        let value = json!({
            "choices": [{"message": {"reasoning": {"content": "think"}}}]
        });
        let (message, _) = parse_openai_json(&value).expect("reasoning only");
        assert_eq!(message.reasoning_content, "think");
        assert!(message.content.is_empty());
    }

    #[test]
    fn parses_complete_json_message() {
        let value = json!({
            "choices": [{"message": {"role": "assistant", "content": "hello plan"}}],
            "usage": {"prompt_tokens": 8, "completion_tokens": 2}
        });
        let (message, usage) = parse_openai_json(&value).expect("parse json");
        assert_eq!(message.content, "hello plan");
        assert_eq!(usage.prompt_tokens, 8);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn parses_reasoning_object_and_content_parts() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning\":{\"content\":\"think\"},\"content\":[{\"type\":\"output_text\",\"text\":\"hi\"}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (message, _) = parse_openai_sse(sse).expect("parse reasoning object");
        assert_eq!(message.reasoning_content, "think");
        assert_eq!(message.content, "hi");
    }

    #[test]
    fn parses_sse_without_blank_line_separators() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\ndata: [DONE]\n";
        let (message, _) = parse_openai_sse(sse).expect("parse packed sse");
        assert_eq!(message.content, "hello world");
    }
}
