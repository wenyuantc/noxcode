//! Provider-independent completion status and whole-batch tool validation.

use std::collections::HashSet;
use std::fmt;

use super::types::{Message, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    OutputLimit,
    ContextLimit,
    Refusal,
    Unknown,
}

impl FinishReason {
    pub fn from_raw(reason: Option<&str>) -> Self {
        match reason {
            Some("stop" | "end_turn" | "stop_sequence" | "completed") => Self::Stop,
            Some("tool_calls" | "tool_use" | "function_call") => Self::ToolCalls,
            Some("length" | "max_tokens" | "max_output_tokens") => Self::OutputLimit,
            Some("model_context_window_exceeded" | "context_length_exceeded") => Self::ContextLimit,
            Some("refusal" | "content_filter" | "safety" | "blocked") => Self::Refusal,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorKind {
    /// 连接或读流失败，且不能归入更具体的超时、断线分类。
    Transport,
    /// 在总期限内没有收到有效协议数据。
    FirstByteTimeout,
    /// 已有协议进展后，空闲超过期限。心跳不刷新这个计时。
    StreamIdle,
    /// 整个请求超过首包/总期限。
    RequestTimeout,
    /// 连接被重置、提前关闭或无法建立。
    Network,
    /// 流结束了，但没有协议规定的终止事件。
    IncompleteStream,
    /// SSE/JSON 无法按协议解析。
    InvalidResponse,
    Provider,
    ContextLimit,
    Cancelled,
}

impl ModelErrorKind {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Transport => "传输失败",
            Self::FirstByteTimeout => "首包等待",
            Self::StreamIdle => "流空闲",
            Self::RequestTimeout => "请求总期限",
            Self::Network => "网络中断",
            Self::IncompleteStream => "无合法终止",
            Self::InvalidResponse => "协议错误",
            Self::Provider => "提供商错误",
            Self::ContextLimit => "上下文上限",
            Self::Cancelled => "用户取消",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub kind: ModelErrorKind,
    pub message: String,
}

impl ModelError {
    pub fn new(kind: ModelErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for ModelError {}

impl From<ModelError> for String {
    fn from(error: ModelError) -> Self {
        error.message
    }
}

pub fn provider_error(value: &serde_json::Value) -> Option<ModelError> {
    let error = value.get("error").filter(|error| !error.is_null())?;
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.get("type").and_then(serde_json::Value::as_str));
    let kind = if FinishReason::from_raw(code) == FinishReason::ContextLimit {
        ModelErrorKind::ContextLimit
    } else {
        ModelErrorKind::Provider
    };
    Some(ModelError::new(
        kind,
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| error.to_string()),
    ))
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub message: Message,
    pub usage: Usage,
    pub finish_reason: FinishReason,
    pub raw_finish_reason: Option<String>,
    pub response_id: Option<String>,
}

impl ModelResponse {
    pub fn new(
        message: Message,
        usage: Usage,
        raw_finish_reason: Option<String>,
        response_id: Option<String>,
    ) -> Result<Self, ModelError> {
        let finish_reason = FinishReason::from_raw(raw_finish_reason.as_deref());
        Self::from_parts(
            message,
            usage,
            finish_reason,
            raw_finish_reason,
            response_id,
        )
    }

    /// The normalized outcome may also come from a refusal content block;
    /// retain the actual provider finish field independently.
    pub fn from_parts(
        message: Message,
        usage: Usage,
        mut finish_reason: FinishReason,
        raw_finish_reason: Option<String>,
        response_id: Option<String>,
    ) -> Result<Self, ModelError> {
        if finish_reason == FinishReason::Stop && !message.tool_calls.is_empty() {
            finish_reason = FinishReason::ToolCalls;
        }
        if message.content.is_empty()
            && message.reasoning_content.is_empty()
            && message.tool_calls.is_empty()
            && !matches!(
                finish_reason,
                FinishReason::OutputLimit | FinishReason::ContextLimit | FinishReason::Refusal
            )
        {
            return Err(ModelError::new(
                ModelErrorKind::InvalidResponse,
                "模型返回空响应",
            ));
        }
        Self {
            message,
            usage,
            finish_reason,
            raw_finish_reason,
            response_id,
        }
        .validated()
    }

    /// Auxiliary calls do not retry partial generations or treat them as complete artifacts.
    pub fn complete_message(self) -> Result<Message, ModelError> {
        if matches!(
            self.finish_reason,
            FinishReason::OutputLimit | FinishReason::ContextLimit
        ) {
            return Err(ModelError::new(
                ModelErrorKind::InvalidResponse,
                "模型输出未完成",
            ));
        }
        Ok(self.message)
    }
    /// Call only after the provider parser has proved transport completion.
    /// A valid partial answer is useful, but its tool arguments are not executable.
    pub fn validated(mut self) -> Result<Self, ModelError> {
        if matches!(
            self.finish_reason,
            FinishReason::OutputLimit | FinishReason::ContextLimit | FinishReason::Refusal
        ) {
            self.message.tool_calls.clear();
            return Ok(self);
        }
        let mut ids = HashSet::new();
        for call in &self.message.tool_calls {
            if call.id.trim().is_empty() || call.name.trim().is_empty() || !ids.insert(&call.id) {
                return Err(ModelError::new(
                    ModelErrorKind::InvalidResponse,
                    "模型工具调用缺少名称、标识或包含重复标识，未执行该批工具",
                ));
            }
            if !serde_json::from_str::<serde_json::Value>(&call.arguments)
                .is_ok_and(|value| value.is_object())
            {
                return Err(ModelError::new(
                    ModelErrorKind::InvalidResponse,
                    "模型工具参数不是完整的 JSON 对象，未执行该批工具",
                ));
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::model::types::ToolCall;

    fn response(reason: FinishReason, calls: Vec<ToolCall>) -> ModelResponse {
        let mut message = Message::assistant_text("visible partial answer");
        message.tool_calls = calls;
        ModelResponse {
            message,
            usage: Usage::default(),
            finish_reason: reason,
            raw_finish_reason: None,
            response_id: None,
        }
    }

    fn call(id: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "Write".into(),
            arguments: arguments.into(),
        }
    }

    #[test]
    fn normalizes_provider_reasons_without_guessing_unknown_reasons() {
        for (raw, expected) in [
            ("stop", FinishReason::Stop),
            ("end_turn", FinishReason::Stop),
            ("tool_use", FinishReason::ToolCalls),
            ("tool_calls", FinishReason::ToolCalls),
            ("length", FinishReason::OutputLimit),
            ("max_tokens", FinishReason::OutputLimit),
            ("max_output_tokens", FinishReason::OutputLimit),
            ("model_context_window_exceeded", FinishReason::ContextLimit),
            ("content_filter", FinishReason::Refusal),
            ("refusal", FinishReason::Refusal),
            ("new_gateway_reason", FinishReason::Unknown),
        ] {
            assert_eq!(FinishReason::from_raw(Some(raw)), expected, "{raw}");
        }
        assert_eq!(FinishReason::from_raw(None), FinishReason::Unknown);
    }

    #[test]
    fn invalid_last_tool_rejects_entire_batch_before_any_execution() {
        for arguments in ["{", "[]", "null", "\"text\""] {
            let result = response(
                FinishReason::ToolCalls,
                vec![
                    call("first", r#"{"file_path":"a"}"#),
                    call("last", arguments),
                ],
            )
            .validated();
            assert_eq!(result.unwrap_err().kind, ModelErrorKind::InvalidResponse);
        }
    }

    #[test]
    fn duplicate_or_missing_tool_identity_is_not_executable() {
        for calls in [
            vec![call("same", "{}"), call("same", "{}")],
            vec![call("", "{}")],
            vec![ToolCall {
                name: String::new(),
                ..call("id", "{}")
            }],
        ] {
            assert!(response(FinishReason::ToolCalls, calls)
                .validated()
                .is_err());
        }
    }

    #[test]
    fn partial_and_refused_answers_keep_text_but_never_tools() {
        for reason in [
            FinishReason::OutputLimit,
            FinishReason::ContextLimit,
            FinishReason::Refusal,
        ] {
            let output = response(reason, vec![call("id", "{")]).validated().unwrap();
            assert_eq!(output.message.content, "visible partial answer");
            assert!(output.message.tool_calls.is_empty());
        }
    }

    #[test]
    fn complete_unknown_reason_still_validates_tools() {
        let output = response(FinishReason::Unknown, vec![call("id", "{}")])
            .validated()
            .unwrap();
        assert_eq!(output.message.tool_calls.len(), 1);
        assert_eq!(output.finish_reason, FinishReason::Unknown);
    }
}
