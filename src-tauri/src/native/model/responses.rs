use serde_json::{json, Value};

use super::openai::normalize_effort;
use super::response::{provider_error, FinishReason, ModelError, ModelErrorKind, ModelResponse};
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

pub fn parse_responses_sse(text: &str) -> Result<ModelResponse, ModelError> {
    parse_responses_sse_with_id(text)
}

/// Parse a Responses stream and retain the server response identifier when one
/// is present. The identifier lets the caller use `previous_response_id` on
/// the next request without re-sending the entire conversation.
pub fn parse_responses_sse_with_id(text: &str) -> Result<ModelResponse, ModelError> {
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
    terminal: bool,
    refused: bool,
    incomplete_tool: bool,
    raw_finish_reason: Option<String>,
    error: Option<ModelError>,
    item_ids: std::collections::HashMap<String, String>,
}

impl Default for ResponsesStreamState {
    fn default() -> Self {
        Self {
            message: Message::assistant_text(""),
            tools: Vec::new(),
            usage: Usage::default(),
            response_id: None,
            terminal: false,
            refused: false,
            incomplete_tool: false,
            raw_finish_reason: None,
            error: None,
            item_ids: Default::default(),
        }
    }
}

impl ResponsesStreamState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &SseEvent) -> Vec<StreamDelta> {
        // [DONE] is a framing marker, not proof of a completed Responses response.
        if event.data == "[DONE]" {
            return Vec::new();
        }
        let Ok(mut payload) = serde_json::from_str::<Value>(&event.data) else {
            self.error = Some(ModelError::new(
                ModelErrorKind::InvalidResponse,
                "无效的 Responses SSE JSON",
            ));
            return Vec::new();
        };
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event.event.as_str())
            .to_string();
        let response = payload.get("response").unwrap_or(&payload);
        if let Some(error) = provider_error(response).or_else(|| provider_error(&payload)) {
            self.error = Some(error);
            return Vec::new();
        }
        if matches!(event_type.as_str(), "error" | "response.failed") {
            self.error = Some(ModelError::new(
                ModelErrorKind::Provider,
                event.data.clone(),
            ));
            return Vec::new();
        }
        if matches!(
            event_type.as_str(),
            "response.completed" | "response.done" | "response.incomplete"
        ) {
            self.terminal = true;
            self.raw_finish_reason = responses_finish_reason(response);
            self.refused |= responses_refused(response);
            if matches!(
                response.get("status").and_then(Value::as_str),
                Some("failed" | "in_progress" | "queued")
            ) || ((event_type == "response.incomplete"
                || response.get("status").and_then(Value::as_str) == Some("incomplete"))
                && FinishReason::from_raw(self.raw_finish_reason.as_deref())
                    == FinishReason::Unknown
                && !self.refused)
            {
                self.error = Some(ModelError::new(
                    ModelErrorKind::InvalidResponse,
                    "Responses 返回未完成且无法分类的响应",
                ));
            }
            if self.raw_finish_reason.is_none() && event_type == "response.incomplete" {
                self.raw_finish_reason = Some("incomplete".to_string());
            }
            if let Some(id) = responses_response_id(&payload) {
                self.response_id = Some(id.to_string());
            }
            if let Some(output) = response
                .get("output")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
            {
                let mut complete = Message::assistant_text("");
                let mut tools = Vec::new();
                apply_responses_output(&mut complete, &mut tools, output);
                if !complete.content.is_empty() {
                    self.message.content = complete.content;
                }
                if !complete.reasoning_content.is_empty() {
                    self.message.reasoning_content = complete.reasoning_content;
                }
                self.tools = tools;
                if output
                    .iter()
                    .any(|item| item.get("status").and_then(Value::as_str) == Some("incomplete"))
                    && !matches!(
                        FinishReason::from_raw(self.raw_finish_reason.as_deref()),
                        FinishReason::OutputLimit
                            | FinishReason::ContextLimit
                            | FinishReason::Refusal
                    )
                    && !self.refused
                {
                    self.error = Some(ModelError::new(
                        ModelErrorKind::InvalidResponse,
                        "Responses 工具或文本块尚未完成",
                    ));
                }
            }
        }
        if matches!(
            event_type.as_str(),
            "response.refusal.delta" | "response.refusal.done"
        ) {
            self.refused = true;
            if let Some(text) = payload
                .get("delta")
                .or_else(|| payload.get("refusal"))
                .and_then(Value::as_str)
            {
                if event_type.ends_with(".delta") || self.message.content.is_empty() {
                    self.message.content.push_str(text);
                    return vec![StreamDelta::Text(text.to_string())];
                }
            }
        }
        if let Some(item) = payload.get("item") {
            if event_type == "response.output_item.done"
                && item.get("status").and_then(Value::as_str) == Some("incomplete")
            {
                self.incomplete_tool = true;
            }
            if let (Some(id), Some(call_id)) = (
                item.get("id").and_then(Value::as_str),
                item.get("call_id").and_then(Value::as_str),
            ) {
                self.item_ids.insert(id.to_string(), call_id.to_string());
            }
        }
        if event_type.starts_with("response.function_call_arguments.") {
            match argument_call_id(&payload, &self.item_ids, &self.tools) {
                Ok(call_id) => payload["call_id"] = json!(call_id),
                Err(error) => {
                    self.error = Some(error);
                    return Vec::new();
                }
            }
        }
        apply_responses_event(
            &mut self.message,
            &mut self.tools,
            &mut self.usage,
            &mut self.response_id,
            &event_type,
            &payload,
        )
    }

    pub fn finish(mut self) -> Result<ModelResponse, ModelError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.terminal {
            return Err(ModelError::new(
                ModelErrorKind::IncompleteStream,
                "Responses 响应流缺少结束事件",
            ));
        }
        if self.raw_finish_reason.as_deref() == Some("incomplete") {
            return Err(ModelError::new(
                ModelErrorKind::InvalidResponse,
                "Responses 响应未完成且缺少结束原因",
            ));
        }
        let reason = if self.refused {
            FinishReason::Refusal
        } else {
            FinishReason::from_raw(self.raw_finish_reason.as_deref())
        };
        if self.incomplete_tool
            && !matches!(
                reason,
                FinishReason::OutputLimit | FinishReason::ContextLimit | FinishReason::Refusal
            )
        {
            return Err(ModelError::new(
                ModelErrorKind::InvalidResponse,
                "Responses 输出工具尚未完成",
            ));
        }
        self.message.tool_calls = self.tools;
        ModelResponse::from_parts(
            self.message,
            self.usage,
            reason,
            self.raw_finish_reason,
            self.response_id,
        )
    }
}

fn argument_call_id(
    payload: &Value,
    item_ids: &std::collections::HashMap<String, String>,
    tools: &[ToolCall],
) -> Result<String, ModelError> {
    let invalid = || {
        ModelError::new(
            ModelErrorKind::InvalidResponse,
            "Responses 工具参数标识缺失、未知或相互冲突",
        )
    };
    let direct = payload
        .get("call_id")
        .map(|value| {
            value
                .as_str()
                .filter(|id| !id.is_empty() && tools.iter().any(|tool| tool.id == *id))
                .map(ToOwned::to_owned)
                .ok_or_else(invalid)
        })
        .transpose()?;
    let item = payload
        .get("item_id")
        .map(|value| {
            value
                .as_str()
                .and_then(|id| item_ids.get(id))
                .filter(|id| tools.iter().any(|tool| tool.id == **id))
                .cloned()
                .ok_or_else(invalid)
        })
        .transpose()?;
    match (direct, item) {
        (Some(direct), Some(item)) if direct != item => Err(invalid()),
        (Some(id), _) | (_, Some(id)) => Ok(id),
        (None, None) if tools.len() == 1 => Ok(tools[0].id.clone()),
        _ => Err(invalid()),
    }
}

fn responses_finish_reason(response: &Value) -> Option<String> {
    response
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
        .or_else(|| response.get("finish_reason").and_then(Value::as_str))
        .or_else(|| response.get("status").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn responses_refused(response: &Value) -> bool {
    response
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts
                            .iter()
                            .any(|part| part.get("type").and_then(Value::as_str) == Some("refusal"))
                    })
            })
        })
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
        "response.created"
        | "response.in_progress"
        | "response.completed"
        | "response.done"
        | "response.incomplete" => {
            if response_id.is_none() {
                *response_id = responses_response_id(payload).map(ToOwned::to_owned);
            }
            if matches!(
                event_type,
                "response.completed" | "response.done" | "response.incomplete"
            ) {
                if let Some(raw) = payload
                    .pointer("/response/usage")
                    .or_else(|| payload.get("usage"))
                {
                    *usage = parse_usage(raw);
                }
                if message.content.is_empty() && tools.is_empty() {
                    if let Some(output) = payload
                        .pointer("/response/output")
                        .or_else(|| payload.get("output"))
                        .and_then(Value::as_array)
                    {
                        let mut completed = Message::assistant_text("");
                        let mut completed_tools = Vec::new();
                        apply_responses_output(&mut completed, &mut completed_tools, output);
                        message.content = completed.content;
                        if message.reasoning_content.is_empty() {
                            message.reasoning_content = completed.reasoning_content;
                        }
                        tools.extend(completed_tools);
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
        "response.reasoning_summary_text.delta" => {
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
        "response.reasoning_summary_text.done" => {
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
        "response.output_item.done" => {
            if let Some(item) = payload
                .get("item")
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
            {
                let mut complete = Vec::new();
                apply_responses_output(
                    &mut Message::assistant_text(""),
                    &mut complete,
                    std::slice::from_ref(item),
                );
                for call in complete {
                    if let Some(existing) = tools.iter_mut().find(|existing| existing.id == call.id)
                    {
                        *existing = call;
                    } else {
                        tools.push(call);
                    }
                }
            }
        }
        "response.function_call_arguments.done" => {
            let id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
            if let Some(call) = tools.iter_mut().find(|call| call.id == id || id.is_empty()) {
                if let Some(arguments) = payload.get("arguments").and_then(Value::as_str) {
                    call.arguments = arguments.to_string();
                }
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
        _ => {}
    }
    deltas
}

pub fn parse_responses_json(value: &Value) -> Result<ModelResponse, ModelError> {
    parse_responses_json_with_id(value)
}

/// Parse a complete Responses payload and retain its server response id.
pub fn parse_responses_json_with_id(value: &Value) -> Result<ModelResponse, ModelError> {
    let mut message = Message::assistant_text("");
    let mut tools = Vec::new();
    let response = value.get("response").unwrap_or(value);
    if let Some(error) = provider_error(response).or_else(|| provider_error(value)) {
        return Err(error);
    }
    let reason = responses_finish_reason(response);
    if matches!(
        response.get("status").and_then(Value::as_str),
        Some("failed" | "in_progress" | "queued")
    ) || (response.get("status").and_then(Value::as_str) == Some("incomplete")
        && FinishReason::from_raw(reason.as_deref()) == FinishReason::Unknown
        && !responses_refused(response))
    {
        return Err(ModelError::new(
            ModelErrorKind::InvalidResponse,
            "Responses 返回未完成的响应",
        ));
    }
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
    let normalized = if responses_refused(response) {
        FinishReason::Refusal
    } else {
        FinishReason::from_raw(reason.as_deref())
    };
    if !matches!(
        normalized,
        FinishReason::OutputLimit | FinishReason::ContextLimit | FinishReason::Refusal
    ) && response
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("status").and_then(Value::as_str) == Some("incomplete"))
        })
    {
        return Err(ModelError::new(
            ModelErrorKind::InvalidResponse,
            "Responses 输出块尚未完成",
        ));
    }
    ModelResponse::from_parts(
        message,
        usage,
        normalized,
        reason,
        response_id.map(ToOwned::to_owned),
    )
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
                        if matches!(
                            part_type,
                            Some("output_text") | Some("text") | Some("refusal") | None
                        ) {
                            if let Some(text) = part
                                .get("text")
                                .or_else(|| part.get("refusal"))
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
        let ModelResponse { message, usage, .. } =
            parse_responses_sse(sse).expect("parse responses sse");
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
        let ModelResponse { response_id, .. } =
            parse_responses_sse_with_id(sse).expect("parse response id");
        assert_eq!(response_id.as_deref(), Some("resp_123"));
    }

    #[test]
    fn parses_completed_output_without_deltas() {
        let sse = concat!(
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"plan ok\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
        );
        let ModelResponse { message, usage, .. } =
            parse_responses_sse(sse).expect("parse completed output");
        assert_eq!(message.content, "plan ok");
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn parses_output_text_done_without_deltas() {
        let sse = concat!("event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"text\":\"only done\"}\n\n", "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n");
        let ModelResponse { message, .. } =
            parse_responses_sse(sse).expect("parse output_text.done");
        assert_eq!(message.content, "only done");
    }

    #[test]
    fn keeps_reasoning_summary_out_of_response_text() {
        let sse = concat!(
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"思考过程\"}\n\n",
            "event: response.reasoning_summary_text.done\n",
            "data: {\"type\":\"response.reasoning_summary_text.done\",\"text\":\"思考过程\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"问候\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        );
        let ModelResponse { message, .. } =
            parse_responses_sse(sse).expect("parse reasoning summary");
        assert_eq!(message.content, "问候");
        assert_eq!(message.reasoning_content, "思考过程");
    }

    #[test]
    fn ignores_unknown_text_events() {
        let sse = concat!(
            "event: response.unknown_text.delta\n",
            "data: {\"type\":\"response.unknown_text.delta\",\"delta\":\"not output\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"actual output\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n",
        );
        let ModelResponse { message, .. } = parse_responses_sse(sse).expect("parse unknown event");
        assert_eq!(message.content, "actual output");
        assert_eq!(message.reasoning_content, "");
    }

    #[test]
    fn completed_output_fills_text_after_reasoning_summary() {
        let sse = concat!(
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"思考\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"reasoning\",\"summary\":[{\"text\":\"思考\"}]},{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"问候\"}]}]}}\n\n",
        );
        let ModelResponse { message, .. } =
            parse_responses_sse(sse).expect("parse completed output");
        assert_eq!(message.content, "问候");
        assert_eq!(message.reasoning_content, "思考");
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
        let ModelResponse { message, usage, .. } =
            parse_responses_json(&value).expect("parse json");
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
        let ModelResponse { response_id, .. } =
            parse_responses_json_with_id(&value).expect("parse response id");
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
