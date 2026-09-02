use serde_json::{json, Value};

use super::openai::normalize_effort;
use super::sse::{parse_sse, SseEvent};
use super::types::{Message, Role, StreamDelta, ToolCall, ToolSpec, Usage};
use super::usage::parse_usage;

pub fn thinking_budget_tokens(effort: Option<&str>) -> u32 {
    match effort.map(str::trim).unwrap_or("medium") {
        "minimal" | "min" | "low" => 2048,
        "high" => 16384,
        "xhigh" => 32768,
        "max" => 64000,
        _ => 8192,
    }
}

pub fn build_anthropic_body(
    messages: &[Message],
    tools: &[ToolSpec],
    model: &str,
    effort: Option<&str>,
    max_output_tokens: Option<u32>,
    thinking_enabled: bool,
    stream: bool,
) -> Value {
    let (system, wire_messages) = anthropic_messages(messages);
    let requested = max_output_tokens.unwrap_or(8192).max(1);
    let mut max_tokens = requested;
    let mut body = json!({
        "model": model,
        "messages": wire_messages,
        "stream": stream,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = json!(anthropic_tools(tools));
    }
    if thinking_enabled && normalize_effort(effort).is_some() {
        let budget = thinking_budget_tokens(effort);
        max_tokens = requested.max(budget.saturating_add(1024));
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }
    body["max_tokens"] = json!(max_tokens);
    body
}

pub fn anthropic_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters,
            })
        })
        .collect()
}

pub fn anthropic_messages(messages: &[Message]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut wire = Vec::new();
    for message in messages {
        match message.role {
            Role::System => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&message.content);
            }
            Role::User => wire.push(json!({
                "role": "user",
                "content": anthropic_user_content(message),
            })),
            Role::Assistant => {
                let mut content = Vec::new();
                if !message.reasoning_content.is_empty() {
                    content
                        .push(json!({"type": "thinking", "thinking": message.reasoning_content}));
                }
                if !message.content.is_empty() {
                    content.push(json!({"type": "text", "text": message.content}));
                }
                for call in &message.tool_calls {
                    let input = serde_json::from_str::<Value>(&call.arguments)
                        .unwrap_or_else(|_| json!(call.arguments));
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                if content.is_empty() {
                    content.push(json!({"type": "text", "text": ""}));
                }
                wire.push(json!({"role": "assistant", "content": content}));
            }
            Role::Tool => wire.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id,
                    "content": message.content,
                }]
            })),
        }
    }
    (system, wire)
}

fn anthropic_user_content(message: &Message) -> Value {
    if message.images.is_empty() {
        return json!(message.content);
    }
    let mut parts = vec![json!({"type": "text", "text": message.content})];
    for image in &message.images {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.mime_type,
                "data": image.data_base64,
            }
        }));
    }
    json!(parts)
}

pub fn parse_anthropic_sse(text: &str) -> Result<(Message, Usage), String> {
    let mut state = AnthropicStreamState::new();
    for event in parse_sse(text) {
        state.apply(&event);
    }
    state.finish()
}

/// Incremental counterpart of [`parse_anthropic_sse`]: the same accumulation
/// rules applied one event at a time, plus the deltas that can be shown
/// before the response completes.
#[derive(Debug)]
pub struct AnthropicStreamState {
    message: Message,
    usage: Usage,
    tools: Vec<(i64, ToolCall)>,
}

impl Default for AnthropicStreamState {
    fn default() -> Self {
        Self {
            message: Message::assistant_text(""),
            usage: Usage::default(),
            tools: Vec::new(),
        }
    }
}

impl AnthropicStreamState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &SseEvent) -> Vec<StreamDelta> {
        let Ok(payload) = serde_json::from_str::<Value>(&event.data) else {
            return Vec::new();
        };
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event.event.as_str());
        match event_type {
            "content_block_start" => {
                start_anthropic_block(&mut self.message, &mut self.tools, &payload)
            }
            "content_block_delta" => {
                delta_anthropic_block(&mut self.message, &mut self.tools, &payload)
            }
            "message_start" => {
                if let Some(raw) = payload.pointer("/message/usage") {
                    let parsed = parse_usage(raw);
                    self.usage.prompt_tokens = parsed.prompt_tokens;
                    self.usage.cached_tokens = parsed.cached_tokens;
                }
                Vec::new()
            }
            "message_delta" => {
                if let Some(raw) = payload.get("usage") {
                    let parsed = parse_usage(raw);
                    if parsed.prompt_tokens > 0 {
                        self.usage.prompt_tokens = parsed.prompt_tokens;
                    }
                    if parsed.completion_tokens > 0 {
                        self.usage.completion_tokens = parsed.completion_tokens;
                    }
                    if parsed.cached_tokens > 0 {
                        self.usage.cached_tokens = parsed.cached_tokens;
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
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

pub fn parse_anthropic_json(value: &Value) -> Result<(Message, Usage), String> {
    let mut message = Message::assistant_text("");
    let mut tools: Vec<(i64, ToolCall)> = Vec::new();
    let usage = value.get("usage").map(parse_usage).unwrap_or_default();
    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        for (index, block) in blocks.iter().enumerate() {
            let payload = json!({
                "index": index as i64,
                "content_block": block,
            });
            start_anthropic_block(&mut message, &mut tools, &payload);
        }
    }
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

fn start_anthropic_block(
    message: &mut Message,
    tools: &mut Vec<(i64, ToolCall)>,
    payload: &Value,
) -> Vec<StreamDelta> {
    let index = payload.get("index").and_then(Value::as_i64).unwrap_or(0);
    let block = payload.get("content_block").unwrap_or(&Value::Null);
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            if let Some(text) = block
                .get("text")
                .and_then(Value::as_str)
                .filter(|item| !item.is_empty())
            {
                message.content.push_str(text);
                return vec![StreamDelta::Text(text.to_string())];
            }
        }
        Some("thinking") | Some("redacted_thinking") => {
            if let Some(text) = block
                .get("thinking")
                .or_else(|| block.get("text"))
                .and_then(Value::as_str)
                .filter(|item| !item.is_empty())
            {
                message.reasoning_content.push_str(text);
                return vec![StreamDelta::Reasoning(text.to_string())];
            }
        }
        Some("tool_use") => tools.push((
            index,
            ToolCall {
                id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments: String::new(),
            },
        )),
        _ => {}
    }
    Vec::new()
}

fn delta_anthropic_block(
    message: &mut Message,
    tools: &mut [(i64, ToolCall)],
    payload: &Value,
) -> Vec<StreamDelta> {
    let index = payload.get("index").and_then(Value::as_i64).unwrap_or(0);
    let delta = payload.get("delta").unwrap_or(&Value::Null);
    match delta.get("type").and_then(Value::as_str) {
        Some("text_delta") => {
            if let Some(text) = delta
                .get("text")
                .and_then(Value::as_str)
                .filter(|item| !item.is_empty())
            {
                message.content.push_str(text);
                return vec![StreamDelta::Text(text.to_string())];
            }
        }
        Some("thinking_delta") => {
            let chunk = delta
                .get("thinking")
                .and_then(Value::as_str)
                .or_else(|| delta.get("text").and_then(Value::as_str))
                .unwrap_or_default();
            message.reasoning_content.push_str(chunk);
            if !chunk.is_empty() {
                return vec![StreamDelta::Reasoning(chunk.to_string())];
            }
        }
        Some("input_json_delta") => {
            if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                if let Some((_, call)) = tools.iter_mut().find(|(item, _)| *item == index) {
                    call.arguments.push_str(partial);
                }
            }
        }
        _ => {}
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::model::types::ToolCall;

    #[test]
    fn parses_text_and_tool_use_sse() {
        let sse = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"hi \"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"there\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Read\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":6}}\n\n",
        );
        let (message, usage) = parse_anthropic_sse(sse).expect("parse anthropic sse");
        assert_eq!(message.content, "hi there");
        assert_eq!(message.tool_calls[0].id, "toolu_1");
        assert_eq!(message.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(usage.prompt_tokens, 9);
        assert_eq!(usage.completion_tokens, 6);
    }

    #[test]
    fn parses_complete_json_text_and_thinking() {
        let value = json!({
            "content": [
                {"type": "thinking", "thinking": "reason"},
                {"type": "text", "text": "hello"}
            ],
            "usage": {"input_tokens": 5, "output_tokens": 2}
        });
        let (message, usage) = parse_anthropic_json(&value).expect("parse json");
        assert_eq!(message.reasoning_content, "reason");
        assert_eq!(message.content, "hello");
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn parses_thinking_block_start() {
        let sse = concat!(
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"hm\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"m\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"ok\"}}\n\n",
        );
        let (message, _) = parse_anthropic_sse(sse).expect("parse thinking start");
        assert_eq!(message.reasoning_content, "hmm");
        assert_eq!(message.content, "ok");
    }

    #[test]
    fn second_round_uses_tool_result_block() {
        let messages = vec![
            Message::system("sys"),
            Message::user("read it"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "toolu_1".to_string(),
                    name: "Read".to_string(),
                    arguments: r#"{"path":"a.rs"}"#.to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
            },
            Message::tool_result("toolu_1", "ok"),
        ];
        let body = build_anthropic_body(
            &messages,
            &[],
            "claude-sonnet-4",
            None,
            Some(8192),
            false,
            true,
        );
        assert_eq!(body["system"], "sys");
        let wire = body["messages"].as_array().expect("messages");
        assert_eq!(wire[1]["content"][0]["type"], "tool_use");
        assert_eq!(wire[2]["content"][0]["type"], "tool_result");
        assert_eq!(wire[2]["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn user_message_with_image_uses_base64_source() {
        let mut user = Message::user("see this");
        user.images.push(crate::native::model::types::NativeImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
        });
        let body = build_anthropic_body(
            &[user],
            &[],
            "claude-sonnet-4",
            None,
            Some(1024),
            false,
            false,
        );
        let content = body["messages"][0]["content"].as_array().expect("parts");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "QQ==");
    }
}
