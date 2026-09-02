use serde_json::{json, Value};

use super::openai::normalize_effort;
use super::sse::{parse_sse, SseEvent};
use super::types::{Message, Role, StreamDelta, ToolCall, ToolSpec, Usage};
use super::usage::parse_usage;

pub fn build_responses_body(
    messages: &[Message],
    tools: &[ToolSpec],
    model: &str,
    effort: Option<&str>,
    max_output_tokens: Option<u32>,
    thinking_enabled: bool,
    stream: bool,
) -> Value {
    let (instructions, input) = responses_input(messages);
    let mut body = json!({
        "model": model,
        "input": input,
        "stream": stream,
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = json!(responses_tools(tools));
    }
    if let Some(max_tokens) = max_output_tokens.filter(|value| *value > 0) {
        body["max_output_tokens"] = json!(max_tokens);
    }
    if thinking_enabled {
        if let Some(level) = normalize_effort(effort) {
            body["reasoning"] = json!({"effort": level});
        }
    }
    body
}

pub fn responses_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect()
}

pub fn responses_input(messages: &[Message]) -> (String, Vec<Value>) {
    let mut instructions = String::new();
    let mut input = Vec::new();
    for message in messages {
        match message.role {
            Role::System => {
                if !instructions.is_empty() {
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&message.content);
            }
            Role::User => input.push(json!({
                "role": "user",
                "content": responses_user_content(message),
            })),
            Role::Assistant => {
                if !message.content.is_empty() {
                    input.push(json!({"role": "assistant", "content": message.content}));
                }
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
            }
            Role::Tool => input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id,
                "output": message.content,
            })),
        }
    }
    (instructions, input)
}

fn responses_user_content(message: &Message) -> Value {
    if message.images.is_empty() {
        return json!(message.content);
    }
    let mut parts = vec![json!({"type": "input_text", "text": message.content})];
    for image in &message.images {
        parts.push(json!({
            "type": "input_image",
            "image_url": image.data_url(),
        }));
    }
    json!(parts)
}

pub fn parse_responses_sse(text: &str) -> Result<(Message, Usage), String> {
    parse_responses_sse_with_id(text).map(|(message, usage, _)| (message, usage))
}

/// Parse a Responses stream and retain the server response identifier when one
/// is present. The identifier lets the caller use `previous_response_id` on
/// the next request without re-sending the entire conversation.
pub fn parse_responses_sse_with_id(text: &str) -> Result<(Message, Usage, Option<String>), String> {
    let mut state = ResponsesStreamState::new();
    for event in parse_sse(text) {
        state.apply(&event);
    }
    state.finish()
}

/// Incremental counterpart of [`parse_responses_sse_with_id`]: the same
/// accumulation rules applied one event at a time, plus the text/reasoning
/// deltas that can be shown before the response completes.
#[derive(Debug)]
pub struct ResponsesStreamState {
    message: Message,
    tools: Vec<ToolCall>,
    usage: Usage,
    response_id: Option<String>,
}

impl Default for ResponsesStreamState {
    fn default() -> Self {
        Self {
            message: Message::assistant_text(""),
            tools: Vec::new(),
            usage: Usage::default(),
            response_id: None,
        }
    }
}

impl ResponsesStreamState {
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
        apply_responses_event(
            &mut self.message,
            &mut self.tools,
            &mut self.usage,
            &mut self.response_id,
            event_type,
            &payload,
        )
    }

    pub fn finish(mut self) -> Result<(Message, Usage, Option<String>), String> {
        self.message.tool_calls = self.tools;
        if self.message.content.is_empty()
            && self.message.reasoning_content.is_empty()
            && self.message.tool_calls.is_empty()
        {
            return Err("模型返回空响应".to_string());
        }
        Ok((self.message, self.usage, self.response_id))
    }
}

fn apply_responses_event(
    message: &mut Message,
    tools: &mut Vec<ToolCall>,
    usage: &mut Usage,
    response_id: &mut Option<String>,
    event_type: &str,
    payload: &Value,
) -> Vec<StreamDelta> {
    let mut deltas = Vec::new();
    match event_type {
        "response.created" | "response.in_progress" | "response.completed" | "response.done" => {
            if response_id.is_none() {
                *response_id = responses_response_id(payload).map(ToOwned::to_owned);
            }
            if matches!(event_type, "response.completed" | "response.done") {
                if let Some(raw) = payload
                    .pointer("/response/usage")
                    .or_else(|| payload.get("usage"))
                {
                    *usage = parse_usage(raw);
                }
                let empty = message.content.is_empty()
                    && message.reasoning_content.is_empty()
                    && tools.is_empty();
                if empty {
                    if let Some(output) = payload
                        .pointer("/response/output")
                        .or_else(|| payload.get("output"))
                        .and_then(Value::as_array)
                    {
                        apply_responses_output(message, tools, output);
                    }
                }
            }
        }
        "response.output_text.delta" | "response.content_part.delta" => {
            if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                message.content.push_str(delta);
                deltas.push(StreamDelta::Text(delta.to_string()));
            }
        }
        "response.output_text.done" | "response.output_text.completed" => {
            if message.content.is_empty() {
                if let Some(text) = payload.get("text").and_then(Value::as_str) {
                    message.content.push_str(text);
                    deltas.push(StreamDelta::Text(text.to_string()));
                }
            }
        }
        "response.reasoning_text.delta" | "response.reasoning.delta" => {
            if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                message.reasoning_content.push_str(delta);
                deltas.push(StreamDelta::Reasoning(delta.to_string()));
            }
        }
        "response.reasoning_text.done" | "response.reasoning.done" => {
            if message.reasoning_content.is_empty() {
                if let Some(text) = payload.get("text").and_then(Value::as_str) {
                    message.reasoning_content.push_str(text);
                    deltas.push(StreamDelta::Reasoning(text.to_string()));
                }
            }
        }
        "response.output_item.added" => {
            let item = payload.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                tools.push(ToolCall {
                    id: item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
        "response.function_call_arguments.delta" => {
            let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
            let delta = payload.get("delta").and_then(Value::as_str).unwrap_or("");
            if let Some(call) = tools
                .iter_mut()
                .rev()
                .find(|item| item.id == call_id || call_id.is_empty())
            {
                call.arguments.push_str(delta);
            }
        }
        _ => {
            if let Some(text) = payload.get("delta").and_then(Value::as_str) {
                if event_type.contains("text") {
                    message.content.push_str(text);
                    deltas.push(StreamDelta::Text(text.to_string()));
                }
            }
        }
    }
    deltas
}

pub fn parse_responses_json(value: &Value) -> Result<(Message, Usage), String> {
    parse_responses_json_with_id(value).map(|(message, usage, _)| (message, usage))
}

/// Parse a complete Responses payload and retain its server response id.
pub fn parse_responses_json_with_id(
    value: &Value,
) -> Result<(Message, Usage, Option<String>), String> {
    let mut message = Message::assistant_text("");
    let mut tools = Vec::new();
    let response = value.get("response").unwrap_or(value);
    let response_id = responses_response_id(value).or_else(|| responses_response_id(response));
    let usage = response
        .get("usage")
        .or_else(|| value.get("usage"))
        .map(parse_usage)
        .unwrap_or_default();
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        apply_responses_output(&mut message, &mut tools, output);
    }
    if message.content.is_empty() {
        if let Some(text) = response.get("output_text").and_then(Value::as_str) {
            message.content.push_str(text);
        }
    }
    message.tool_calls = tools;
    if message.content.is_empty()
        && message.reasoning_content.is_empty()
        && message.tool_calls.is_empty()
    {
        return Err("模型返回空响应".to_string());
    }
    Ok((message, usage, response_id.map(ToOwned::to_owned)))
}

fn responses_response_id(value: &Value) -> Option<&str> {
    value
        .pointer("/response/id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

fn apply_responses_output(message: &mut Message, tools: &mut Vec<ToolCall>, output: &[Value]) {
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        let part_type = part.get("type").and_then(Value::as_str);
                        if matches!(part_type, Some("output_text") | Some("text") | None) {
                            if let Some(text) = part
                                .get("text")
                                .and_then(Value::as_str)
                                .filter(|item| !item.is_empty())
                            {
                                message.content.push_str(text);
                            }
                        }
                    }
                } else if let Some(text) = item.get("content").and_then(Value::as_str) {
                    message.content.push_str(text);
                }
            }
            Some("reasoning") => {
                if let Some(text) = responses_reasoning_text(item) {
                    message.reasoning_content.push_str(&text);
                }
            }
            Some("function_call") => {
                tools.push(ToolCall {
                    id: item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            _ => {}
        }
    }
}

fn responses_reasoning_text(item: &Value) -> Option<String> {
    if let Some(text) = item.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    for key in ["summary", "content"] {
        let Some(parts) = item.get(key).and_then(Value::as_array) else {
            continue;
        };
        let mut out = String::new();
        for part in parts {
            if let Some(text) = part
                .get("text")
                .or_else(|| part.get("summary"))
                .and_then(Value::as_str)
            {
                out.push_str(text);
            } else if let Some(text) = part.as_str() {
                out.push_str(text);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::model::types::ToolCall;

    #[test]
    fn parses_text_and_function_call_sse() {
        let sse = concat!(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hi \"}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"Read\",\"arguments\":\"\"}}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"call_id\":\"call_1\",\"delta\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":11,\"output_tokens\":4}}}\n\n",
        );
        let (message, usage) = parse_responses_sse(sse).expect("parse responses sse");
        assert_eq!(message.content, "hi ");
        assert_eq!(message.tool_calls[0].id, "call_1");
        assert_eq!(message.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(usage.prompt_tokens, 11);
        assert_eq!(usage.completion_tokens, 4);
    }

    #[test]
    fn captures_response_id_from_completed_sse() {
        let sse = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        );
        let (_, _, response_id) = parse_responses_sse_with_id(sse).expect("parse response id");
        assert_eq!(response_id.as_deref(), Some("resp_123"));
    }

    #[test]
    fn parses_completed_output_without_deltas() {
        let sse = concat!(
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"plan ok\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
        );
        let (message, usage) = parse_responses_sse(sse).expect("parse completed output");
        assert_eq!(message.content, "plan ok");
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn parses_output_text_done_without_deltas() {
        let sse = "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"text\":\"only done\"}\n\n";
        let (message, _) = parse_responses_sse(sse).expect("parse output_text.done");
        assert_eq!(message.content, "only done");
    }

    #[test]
    fn parses_complete_json_output() {
        let value = json!({
            "output": [
                {"type":"reasoning","summary":[{"text":"think"}]},
                {"type":"message","content":[{"type":"output_text","text":"hello"}]},
                {"type":"function_call","call_id":"call_1","name":"Read","arguments":"{}"}
            ],
            "usage": {"input_tokens": 4, "output_tokens": 1}
        });
        let (message, usage) = parse_responses_json(&value).expect("parse json");
        assert_eq!(message.content, "hello");
        assert_eq!(message.reasoning_content, "think");
        assert_eq!(message.tool_calls[0].id, "call_1");
        assert_eq!(usage.prompt_tokens, 4);
    }

    #[test]
    fn captures_response_id_from_complete_json() {
        let value = json!({
            "id": "resp_json",
            "output": [{"type":"message","content":[{"type":"output_text","text":"ok"}]}]
        });
        let (_, _, response_id) = parse_responses_json_with_id(&value).expect("parse response id");
        assert_eq!(response_id.as_deref(), Some("resp_json"));
    }

    #[test]
    fn second_round_emits_function_call_output() {
        let messages = vec![
            Message::system("sys"),
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
            Message::tool_result("call_1", "ok"),
        ];
        let body = build_responses_body(
            &messages,
            &[],
            "gpt-5.4",
            Some("high"),
            Some(128000),
            true,
            true,
        );
        assert_eq!(body["instructions"], "sys");
        let input = body["input"].as_array().expect("input");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn user_message_with_image_uses_input_image() {
        let mut user = Message::user("see this");
        user.images.push(crate::native::model::types::NativeImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_base64: "QQ==".to_string(),
        });
        let body = build_responses_body(&[user], &[], "gpt-5.4", None, None, false, false);
        let content = body["input"][0]["content"].as_array().expect("parts");
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,QQ==");
    }
}
