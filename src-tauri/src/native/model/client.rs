use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

use crate::app::network_settings::{build_http_client, NetworkSettings};
use crate::app::shared::new_id;
use crate::native::protocol::{
    channel_chat_url, channel_models_url, model_list_next_page, parse_model_list_json,
    PROTOCOL_ANTHROPIC, PROTOCOL_CODEX, PROTOCOL_OPENAI,
};
use crate::native::tools::CancelFlag;

use super::anthropic::{
    apply_anthropic_prompt_cache, build_anthropic_body, parse_anthropic_json, parse_anthropic_sse,
    AnthropicStreamState,
};
use super::call_log::{
    detect_response_encoding, extract_request_model, extract_thinking_level,
    first_meaningful_event_offset, logged_usage_from_parsed, parse_usage_from_body,
    provider_reported_usage, redact_and_truncate_json, redact_and_truncate_text,
    request_thinking_enabled, sse_event_is_meaningful, sse_event_reports_usage, CallLogContext,
    NativeApiCallLogInsert, CALL_STATUS_CANCELLED, CALL_STATUS_FAILED, CALL_STATUS_SUCCESS,
    MODEL_ROLE_MAIN, OPERATION_AGENT_STEP,
};
use super::openai::{
    build_openai_body, parse_max_output_token_limit, parse_openai_json, parse_openai_sse,
    OpenAiStreamState,
};
use super::responses::{
    build_responses_body, parse_responses_json, parse_responses_json_with_id, parse_responses_sse,
    parse_responses_sse_with_id, responses_input, ResponsesStreamState,
};
use super::retry::{
    format_http_error, format_retry_line, is_retryable_error, parse_retry_after, redact_secrets,
    RetryConfig,
};
use super::sse::{parse_sse, SseEvent, SseStreamParser};
use super::types::{Message, StreamDelta, ToolSpec, Usage};

/// Once the body is known to be SSE the retained text is only used for the
/// call log and for error snippets, so a long answer does not need to stay in
/// memory. A body that is not SSE still needs to be complete for the JSON
/// fallback parser, hence the much larger guard.
const SSE_TEXT_BUFFER_LIMIT: usize = 256 * 1024;
const BODY_TEXT_BUFFER_LIMIT: usize = 8 * 1024 * 1024;

type ParsedResponse = (Message, Usage, Option<String>);

#[derive(Default)]
struct StreamScan {
    first_token_ms: Option<i64>,
    usage_reported: bool,
    saw_sse_event: bool,
}

struct TimedHttpBody {
    status: u16,
    text: String,
    first_token_ms: Option<i64>,
    duration_ms: i64,
    cancelled: bool,
    usage_reported: bool,
    /// 服务端 `Retry-After`（限流 / 过载时给出），重试等待优先采用。
    retry_after: Option<Duration>,
    /// `Some` when the body arrived as SSE and was consumed incrementally by
    /// the protocol state machine. `None` means the text fallback still owns
    /// parsing (complete JSON payloads, empty bodies, gateway errors).
    parsed: Option<Result<ParsedResponse, String>>,
}

enum ProtocolStreamState {
    Responses(ResponsesStreamState),
    OpenAi(OpenAiStreamState),
    Anthropic(AnthropicStreamState),
}

impl ProtocolStreamState {
    fn new(protocol: &str) -> Option<Self> {
        match protocol {
            PROTOCOL_ANTHROPIC => Some(Self::Anthropic(AnthropicStreamState::new())),
            PROTOCOL_CODEX => Some(Self::Responses(ResponsesStreamState::new())),
            PROTOCOL_OPENAI => Some(Self::OpenAi(OpenAiStreamState::new())),
            _ => None,
        }
    }

    fn apply(&mut self, event: &SseEvent) -> Vec<StreamDelta> {
        match self {
            Self::Responses(state) => state.apply(event),
            Self::OpenAi(state) => state.apply(event),
            Self::Anthropic(state) => state.apply(event),
        }
    }

    fn finish(self) -> Result<ParsedResponse, String> {
        match self {
            Self::Responses(state) => state.finish(),
            Self::OpenAi(state) => state
                .finish()
                .map(|(message, usage)| (message, usage, None)),
            Self::Anthropic(state) => state
                .finish()
                .map(|(message, usage)| (message, usage, None)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelClientConfig {
    pub protocol: String,
    pub base_url: String,
    pub api_key: String,
    pub extra_headers: HashMap<String, String>,
    pub retry: RetryConfig,
    pub timeout: Duration,
    pub network: NetworkSettings,
}

/// Prompt cache 策略：`Auto` 只对官方端点开启（Anthropic 的 `cache_control`、
/// OpenAI 的 `prompt_cache_key`），第三方兼容网关默认不改请求体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptCacheMode {
    #[default]
    Auto,
    On,
    Off,
}

/// 官方端点才默认打开 prompt cache，避免兼容网关因未知字段报 400。
pub fn host_supports_prompt_cache(base_url: &str, protocol: &str) -> bool {
    let host = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_default();
    match protocol {
        PROTOCOL_ANTHROPIC => host == "api.anthropic.com" || host.ends_with(".anthropic.com"),
        PROTOCOL_OPENAI | PROTOCOL_CODEX => {
            host == "api.openai.com"
                || host.ends_with(".openai.com")
                || host.ends_with(".openai.azure.com")
        }
        _ => false,
    }
}

pub struct ChatRequest<'a> {
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub max_output_tokens: Option<u32>,
    pub thinking_enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListedModels {
    pub models: Vec<String>,
    pub truncated: bool,
}

const MODEL_LIST_PAGE_LIMIT: usize = 20;
const MODEL_LIST_ITEM_LIMIT: usize = 500;
const MAX_OUTPUT_LIMIT_RETRIES: u8 = 3;

pub type RetryHook = Arc<dyn Fn(&str) + Send + Sync>;
pub type CallLogSink = Arc<dyn Fn(NativeApiCallLogInsert) + Send + Sync>;
/// Receives text/reasoning fragments while the response streams in. The hook
/// is deliberately not inherited by clones: child agents and context
/// compaction reuse the transport but must not write into the live view.
pub type DeltaHook = Arc<dyn Fn(StreamDelta) + Send + Sync>;

pub struct ModelClient {
    http: reqwest::Client,
    config: ModelClientConfig,
    on_retry: Option<RetryHook>,
    on_delta: Option<DeltaHook>,
    cancel: Option<CancelFlag>,
    call_log: Option<CallLogContext>,
    call_log_sink: Option<CallLogSink>,
    prompt_cache: PromptCacheMode,
    /// OpenAI / Responses 的 `prompt_cache_key`（按会话），Anthropic 忽略。
    prompt_cache_key: Option<String>,
    /// Responses continuation ids are kept per conversation anchor. A client
    /// is cloned for child agents, so a map prevents a child request from
    /// accidentally continuing the parent's server-side conversation.
    continuations: Arc<Mutex<HashMap<String, ResponseContinuation>>>,
}

#[derive(Debug, Clone)]
struct ResponseContinuation {
    response_id: String,
    message_count: usize,
    anchor: String,
    prefix_hash: u64,
}

impl Clone for ModelClient {
    fn clone(&self) -> Self {
        Self {
            http: self.http.clone(),
            config: self.config.clone(),
            on_retry: self.on_retry.clone(),
            on_delta: None,
            cancel: self.cancel.clone(),
            call_log: self.call_log.clone(),
            call_log_sink: self.call_log_sink.clone(),
            prompt_cache: self.prompt_cache,
            prompt_cache_key: self.prompt_cache_key.clone(),
            // A cloned client is used for child agents and must start its own
            // Responses conversation. Sharing this map would let a child with
            // a matching prompt accidentally attach to its parent's
            // `previous_response_id`.
            continuations: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl ModelClient {
    /// Clone the transport and hooks while retaining the Responses
    /// continuation state for one logical session. `Clone` intentionally
    /// starts a fresh state for child agents; session wrappers use this
    /// variant so follow-up turns can continue the parent's response.
    pub(crate) fn clone_for_conversation(&self) -> Self {
        Self {
            http: self.http.clone(),
            config: self.config.clone(),
            on_retry: self.on_retry.clone(),
            on_delta: None,
            cancel: self.cancel.clone(),
            call_log: self.call_log.clone(),
            call_log_sink: self.call_log_sink.clone(),
            prompt_cache: self.prompt_cache,
            prompt_cache_key: self.prompt_cache_key.clone(),
            continuations: self.continuations.clone(),
        }
    }
}

impl ModelClient {
    pub fn new(config: ModelClientConfig) -> Result<Self, String> {
        let http = build_http_client(config.timeout, &config.network)?;
        Ok(Self {
            http,
            config,
            on_retry: None,
            on_delta: None,
            cancel: None,
            call_log: None,
            call_log_sink: None,
            prompt_cache: PromptCacheMode::Auto,
            prompt_cache_key: None,
            continuations: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn with_prompt_cache(mut self, mode: PromptCacheMode) -> Self {
        self.prompt_cache = mode;
        self
    }

    /// 设置会话级 `prompt_cache_key`（OpenAI / Responses）。
    pub fn with_prompt_cache_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.prompt_cache_key = if key.trim().is_empty() {
            None
        } else {
            Some(key)
        };
        self
    }

    pub fn prompt_cache_enabled(&self) -> bool {
        match self.prompt_cache {
            PromptCacheMode::On => true,
            PromptCacheMode::Off => false,
            PromptCacheMode::Auto => {
                host_supports_prompt_cache(&self.config.base_url, &self.config.protocol)
            }
        }
    }

    /// 在协议请求体上应用 prompt cache 标记。
    fn apply_prompt_cache(&self, body: &mut Value) {
        if !self.prompt_cache_enabled() {
            return;
        }
        match self.config.protocol.as_str() {
            PROTOCOL_ANTHROPIC => apply_anthropic_prompt_cache(body),
            PROTOCOL_OPENAI | PROTOCOL_CODEX => {
                if let Some(key) = &self.prompt_cache_key {
                    body["prompt_cache_key"] = Value::String(key.clone());
                }
            }
            _ => {}
        }
    }

    pub fn with_retry_hook(mut self, hook: RetryHook) -> Self {
        self.on_retry = Some(hook);
        self
    }

    pub fn with_delta_hook(mut self, hook: DeltaHook) -> Self {
        self.on_delta = Some(hook);
        self
    }

    pub fn with_cancel(mut self, cancel: CancelFlag) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn with_call_log(mut self, context: CallLogContext, sink: CallLogSink) -> Self {
        self.call_log = Some(context);
        self.call_log_sink = Some(sink);
        self
    }

    pub fn with_call_log_context(mut self, context: CallLogContext) -> Self {
        self.call_log = Some(context);
        self
    }

    pub fn call_log_context(&self) -> Option<&CallLogContext> {
        self.call_log.as_ref()
    }

    pub fn call_log_sink(&self) -> Option<CallLogSink> {
        self.call_log_sink.clone()
    }

    pub fn build_body(&self, request: &ChatRequest<'_>, stream: bool) -> Result<Value, String> {
        self.build_body_with_continuation(request, stream, None)
    }

    fn build_body_with_continuation(
        &self,
        request: &ChatRequest<'_>,
        stream: bool,
        previous_response_id: Option<&str>,
    ) -> Result<Value, String> {
        let mut body = self.build_body_inner(request, stream, previous_response_id)?;
        self.apply_prompt_cache(&mut body);
        Ok(body)
    }

    fn build_body_inner(
        &self,
        request: &ChatRequest<'_>,
        stream: bool,
        previous_response_id: Option<&str>,
    ) -> Result<Value, String> {
        match self.config.protocol.as_str() {
            PROTOCOL_ANTHROPIC => Ok(build_anthropic_body(
                request.messages,
                request.tools,
                request.model,
                request.effort,
                request.max_output_tokens,
                request.thinking_enabled,
                stream,
            )),
            PROTOCOL_CODEX => {
                let mut body = build_responses_body(
                    request.messages,
                    request.tools,
                    request.model,
                    request.effort,
                    request.max_output_tokens,
                    request.thinking_enabled,
                    stream,
                );
                let Some(previous_response_id) = previous_response_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Ok(body);
                };

                let key = continuation_key(request.messages);
                let Some(continuation) = self.continuation_for(&key, request.messages) else {
                    return Ok(body);
                };
                if continuation.response_id != previous_response_id {
                    return Ok(body);
                }
                let start = continuation.message_count.saturating_add(1);
                if start >= request.messages.len() {
                    return Ok(body);
                }
                let (_, input) = responses_input(&request.messages[start..]);
                if input.is_empty() {
                    return Ok(body);
                }
                body["input"] = Value::Array(input);
                body["previous_response_id"] = Value::String(previous_response_id.to_string());
                Ok(body)
            }
            PROTOCOL_OPENAI => Ok(build_openai_body(
                request.messages,
                request.tools,
                request.model,
                request.effort,
                request.max_output_tokens,
                request.thinking_enabled,
                stream,
            )),
            other => Err(format!("不支持的渠道协议: {other}")),
        }
    }

    pub fn parse_sse(&self, text: &str) -> Result<(Message, Usage), String> {
        match self.config.protocol.as_str() {
            PROTOCOL_ANTHROPIC => parse_anthropic_sse(text),
            PROTOCOL_CODEX => parse_responses_sse(text),
            PROTOCOL_OPENAI => parse_openai_sse(text),
            other => Err(format!("不支持的渠道协议: {other}")),
        }
    }

    pub async fn chat(&self, request: ChatRequest<'_>) -> Result<(Message, Usage), String> {
        let url = channel_chat_url(&self.config.base_url, &self.config.protocol)?;
        let mut max_output_tokens = request.max_output_tokens;
        let mut limit_retries = 0u8;
        let mut retried_continuation = false;
        let conversation_key = continuation_key(request.messages);
        loop {
            let adjusted = ChatRequest {
                messages: request.messages,
                tools: request.tools,
                model: request.model,
                effort: request.effort,
                max_output_tokens,
                thinking_enabled: request.thinking_enabled,
            };
            let continuation = self.continuation_for(&conversation_key, request.messages);
            let body = self.build_body_with_continuation(
                &adjusted,
                true,
                continuation.as_ref().map(|item| item.response_id.as_str()),
            )?;
            match self.post_stream(&url, &body).await {
                Ok((message, usage, response_id)) => {
                    self.update_continuation(&conversation_key, request.messages, response_id);
                    return Ok((message, usage));
                }
                Err(error)
                    if continuation.is_some()
                        && !retried_continuation
                        && is_continuation_rejection(&error) =>
                {
                    self.clear_continuation(&conversation_key);
                    retried_continuation = true;
                    continue;
                }
                Err(error) if limit_retries < MAX_OUTPUT_LIMIT_RETRIES => {
                    let Some(limit) = parse_max_output_token_limit(&error) else {
                        return Err(error);
                    };
                    let current = max_output_tokens.unwrap_or(u32::MAX);
                    if limit == 0 || limit >= current {
                        return Err(error);
                    }
                    max_output_tokens = Some(limit);
                    limit_retries += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub async fn list_models(&self) -> Result<ListedModels, String> {
        let url = channel_models_url(&self.config.base_url)?;
        let mut collected = Vec::new();
        let mut after_id: Option<String> = None;
        let mut truncated = false;
        for _ in 0..MODEL_LIST_PAGE_LIMIT {
            let mut query: Vec<(&str, String)> = Vec::new();
            if self.config.protocol == PROTOCOL_ANTHROPIC {
                query.push(("limit", "100".to_string()));
            }
            if let Some(id) = after_id.as_deref() {
                query.push(("after_id", id.to_string()));
            }
            let query_refs: Vec<(&str, &str)> = query
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();
            let (status, text) = self.get_raw(&url, &query_refs).await?;
            if !(200..300).contains(&status) {
                return Err(format_http_error(status, &url, &text));
            }
            let page = parse_model_list_json(&text)?;
            for model in page {
                if collected.len() >= MODEL_LIST_ITEM_LIMIT {
                    truncated = true;
                    break;
                }
                if !collected.iter().any(|existing| existing == &model) {
                    collected.push(model);
                }
            }
            if collected.len() >= MODEL_LIST_ITEM_LIMIT {
                truncated = truncated || model_list_next_page(&text).is_some();
                break;
            }
            after_id = model_list_next_page(&text);
            if after_id.is_none() {
                break;
            }
        }
        if after_id.is_some() {
            truncated = true;
        }
        collected.sort_by_key(|model| model.to_ascii_lowercase());
        Ok(ListedModels {
            models: collected,
            truncated,
        })
    }

    pub async fn probe(&self, model: &str) -> Result<(), String> {
        let url = channel_chat_url(&self.config.base_url, &self.config.protocol)?;
        let request = ChatRequest {
            messages: &[Message::user("ping")],
            tools: &[],
            model,
            effort: None,
            max_output_tokens: Some(16),
            thinking_enabled: false,
        };
        let body = self.build_body(&request, false)?;
        let timed = self.post_raw(&url, &body).await?;
        if (200..300).contains(&timed.status) {
            return Ok(());
        }
        Err(format_http_error(timed.status, &url, &timed.text))
    }

    async fn post_stream(
        &self,
        url: &str,
        body: &Value,
    ) -> Result<(Message, Usage, Option<String>), String> {
        let attempts = self.config.retry.max_retries.saturating_add(1);
        let call_id = new_id();
        let mut last_error = "模型请求失败".to_string();
        for attempt in 0..attempts {
            let mut retry_after_hint: Option<Duration> = None;
            if self.is_cancelled() {
                self.emit_call_log(
                    &call_id,
                    i64::from(attempt.saturating_add(1)),
                    body,
                    None,
                    None,
                    CALL_STATUS_CANCELLED,
                    None,
                    Some("已取消"),
                );
                return Err("已取消".to_string());
            }
            match self.post_raw(url, body).await {
                Ok(timed) if timed.cancelled => {
                    self.emit_call_log(
                        &call_id,
                        i64::from(attempt.saturating_add(1)),
                        body,
                        Some(&timed),
                        None,
                        CALL_STATUS_CANCELLED,
                        Some(i64::from(timed.status)),
                        Some("已取消"),
                    );
                    return Err("已取消".to_string());
                }
                Ok(mut timed) if (200..300).contains(&timed.status) => {
                    match self.take_parsed_response(&mut timed) {
                        Ok(result) => {
                            self.emit_call_log(
                                &call_id,
                                i64::from(attempt.saturating_add(1)),
                                body,
                                Some(&timed),
                                Some(&result.1),
                                CALL_STATUS_SUCCESS,
                                Some(i64::from(timed.status)),
                                None,
                            );
                            return Ok(result);
                        }
                        Err(error) => last_error = error,
                    }
                    self.emit_call_log(
                        &call_id,
                        i64::from(attempt.saturating_add(1)),
                        body,
                        Some(&timed),
                        None,
                        CALL_STATUS_FAILED,
                        Some(i64::from(timed.status)),
                        Some(&last_error),
                    );
                    if self.should_stop_retry(&last_error, Some(timed.status), attempt, attempts) {
                        return Err(last_error);
                    }
                }
                Ok(timed) => {
                    last_error = format_http_error(timed.status, url, &timed.text);
                    retry_after_hint = timed.retry_after;
                    self.emit_call_log(
                        &call_id,
                        i64::from(attempt.saturating_add(1)),
                        body,
                        Some(&timed),
                        None,
                        CALL_STATUS_FAILED,
                        Some(i64::from(timed.status)),
                        Some(&last_error),
                    );
                    if self.should_stop_retry(&last_error, Some(timed.status), attempt, attempts) {
                        return Err(last_error);
                    }
                }
                Err(error) => {
                    last_error = error;
                    let cancelled = self.is_cancelled() || last_error == "已取消";
                    self.emit_call_log(
                        &call_id,
                        i64::from(attempt.saturating_add(1)),
                        body,
                        None,
                        None,
                        if cancelled {
                            CALL_STATUS_CANCELLED
                        } else {
                            CALL_STATUS_FAILED
                        },
                        None,
                        Some(&last_error),
                    );
                    if cancelled {
                        return Err("已取消".to_string());
                    }
                    if self.should_stop_retry(&last_error, None, attempt, attempts) {
                        return Err(last_error);
                    }
                }
            }
            let delay = self
                .config
                .retry
                .delay_for_attempt_with_hint(attempt, retry_after_hint);
            // The next attempt regenerates the answer from scratch, so
            // anything already streamed into the live view is stale.
            self.emit_delta(StreamDelta::Reset);
            self.emit_retry(&last_error, attempt.saturating_add(1), delay);
            if let Err(error) = self.wait_before_retry(delay).await {
                self.emit_call_log(
                    &call_id,
                    i64::from(attempt.saturating_add(1)),
                    body,
                    None,
                    None,
                    CALL_STATUS_CANCELLED,
                    None,
                    Some(&error),
                );
                return Err(error);
            }
        }
        Err(last_error)
    }

    fn should_stop_retry(
        &self,
        error: &str,
        status: Option<u16>,
        attempt: u32,
        attempts: u32,
    ) -> bool {
        parse_max_output_token_limit(error).is_some()
            || is_continuation_rejection(error)
            || !is_retryable_error(status, error)
            || attempt + 1 >= attempts
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(CancelFlag::is_cancelled)
    }

    fn emit_retry(&self, error: &str, attempt: u32, delay: Duration) {
        let Some(hook) = &self.on_retry else {
            return;
        };
        hook(&format_retry_line(
            &redact_secrets(error),
            attempt,
            self.config.retry.max_retries,
            delay,
        ));
    }

    async fn wait_before_retry(&self, delay: Duration) -> Result<(), String> {
        let Some(cancel) = &self.cancel else {
            tokio::time::sleep(delay).await;
            return Ok(());
        };
        let mut remaining = delay;
        while remaining > Duration::ZERO {
            if cancel.is_cancelled() {
                return Err("已取消".to_string());
            }
            let slice = remaining.min(Duration::from_millis(200));
            tokio::time::sleep(slice).await;
            remaining = remaining.saturating_sub(slice);
        }
        if cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        Ok(())
    }

    fn apply_auth(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.config.protocol == PROTOCOL_ANTHROPIC {
            request = request
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            request = request.header("authorization", format!("Bearer {}", self.config.api_key));
        }
        for (name, value) in &self.config.extra_headers {
            if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("x-api-key")
            {
                continue;
            }
            request = request.header(name, value);
        }
        request
    }

    async fn get_raw(&self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
        let mut request = self.http.get(url).header("accept", "application/json");
        if !query.is_empty() {
            request = request.query(query);
        }
        let response = self
            .apply_auth(request)
            .send()
            .await
            .map_err(|error| format!("拉取模型列表失败: {error}"))?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        Ok((status, text))
    }

    /// Read the response body chunk by chunk. A 2xx SSE body is parsed as it
    /// arrives so text and reasoning fragments reach the delta hook before the
    /// model finishes; every other shape falls back to the buffered text.
    async fn post_raw(&self, url: &str, body: &Value) -> Result<TimedHttpBody, String> {
        let started = Instant::now();
        let request = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream, application/json");
        let response = self
            .apply_auth(request)
            .json(body)
            .send()
            .await
            .map_err(|error| format!("模型请求失败: {error}"))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let mut state = (200..300)
            .contains(&status)
            .then(|| ProtocolStreamState::new(&self.config.protocol))
            .flatten();
        let mut parser = SseStreamParser::new();
        let mut scan = StreamScan::default();
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        let mut cancelled = false;
        while let Some(chunk) = stream.next().await {
            if self.is_cancelled() {
                cancelled = true;
                break;
            }
            let chunk = chunk.map_err(|error| format!("读取模型响应失败: {error}"))?;
            let limit = if scan.saw_sse_event {
                SSE_TEXT_BUFFER_LIMIT
            } else {
                BODY_TEXT_BUFFER_LIMIT
            };
            if bytes.len() < limit {
                bytes.extend_from_slice(&chunk);
            }
            self.absorb_events(
                parser.push_bytes(&chunk),
                state.as_mut(),
                started,
                &mut scan,
            );
        }
        self.absorb_events(parser.finish(), state.as_mut(), started, &mut scan);
        if !cancelled && self.is_cancelled() {
            cancelled = true;
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let parsed = match state {
            Some(state) if scan.saw_sse_event => Some(state.finish()),
            _ => None,
        };
        if !scan.saw_sse_event {
            // A complete JSON payload only becomes meaningful once the whole
            // body has arrived, which is what the per-chunk scan used to
            // detect on its last iteration.
            scan.usage_reported = provider_reported_usage(&text);
            if first_meaningful_event_offset(&text).is_some() {
                scan.first_token_ms = Some(elapsed_ms(started));
            }
        }
        Ok(TimedHttpBody {
            status,
            text,
            first_token_ms: scan.first_token_ms,
            duration_ms: elapsed_ms(started),
            cancelled,
            usage_reported: scan.usage_reported,
            retry_after,
            parsed,
        })
    }

    fn absorb_events(
        &self,
        events: Vec<SseEvent>,
        mut state: Option<&mut ProtocolStreamState>,
        started: Instant,
        scan: &mut StreamScan,
    ) {
        for event in events {
            scan.saw_sse_event = true;
            if scan.first_token_ms.is_none() && sse_event_is_meaningful(&event) {
                scan.first_token_ms = Some(elapsed_ms(started));
            }
            scan.usage_reported = scan.usage_reported || sse_event_reports_usage(&event);
            let Some(state) = state.as_mut() else {
                continue;
            };
            for delta in state.apply(&event) {
                self.emit_delta(delta);
            }
        }
    }

    fn emit_delta(&self, delta: StreamDelta) {
        if let Some(hook) = &self.on_delta {
            hook(delta);
        }
    }

    /// Prefer the incrementally parsed stream and keep the buffered-text
    /// parser as the fallback for complete JSON payloads, empty bodies and
    /// gateway errors.
    fn take_parsed_response(&self, timed: &mut TimedHttpBody) -> Result<ParsedResponse, String> {
        match timed.parsed.take() {
            Some(Ok(parsed)) => Ok(parsed),
            Some(Err(_)) | None => self.parse_success_body_with_id(&timed.text),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_call_log(
        &self,
        call_id: &str,
        attempt: i64,
        request_body: &Value,
        timed: Option<&TimedHttpBody>,
        parsed_usage: Option<&Usage>,
        status: &str,
        http_status: Option<i64>,
        error_message: Option<&str>,
    ) {
        let Some(sink) = &self.call_log_sink else {
            return;
        };
        let context = self.call_log.clone().unwrap_or_default();
        let request = redact_and_truncate_json(request_body);
        let response = timed.map(|item| redact_and_truncate_text(&item.text));
        let usage = logged_usage_from_parsed(
            parsed_usage
                .copied()
                .or_else(|| timed.map(|item| parse_usage_from_body(&item.text)))
                .unwrap_or_default(),
            timed.is_some_and(|item| item.usage_reported),
        );
        let thinking_level = extract_thinking_level(request_body);
        let thinking_enabled = request_thinking_enabled(request_body, thinking_level.as_deref());
        sink(NativeApiCallLogInsert {
            id: String::new(),
            call_id: call_id.to_string(),
            attempt,
            channel_id: context.channel_id,
            channel_name: context.channel_name,
            protocol: self.config.protocol.clone(),
            response_encoding: timed.map(|item| detect_response_encoding(&item.text).to_string()),
            model: extract_request_model(request_body),
            thinking_enabled,
            thinking_level,
            request_format: self.config.protocol.clone(),
            request_body: Some(request.text),
            request_truncated: request.truncated,
            response_body: response.as_ref().map(|item| item.text.clone()),
            response_truncated: response.as_ref().is_some_and(|item| item.truncated),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            total_tokens: usage.total_tokens,
            first_token_ms: timed.and_then(|item| item.first_token_ms),
            duration_ms: timed.map(|item| item.duration_ms),
            status: status.to_string(),
            http_status,
            error_message: error_message.map(ToOwned::to_owned),
            session_id: context.session_id,
            profile_id: context.profile_id,
            workspace_id: context.workspace_id,
            subagent_id: context.subagent_id,
            call_kind: context.call_kind,
            execution_target: context.execution_target,
            operation: context
                .operation
                .unwrap_or_else(|| OPERATION_AGENT_STEP.to_string()),
            model_role: context
                .model_role
                .unwrap_or_else(|| MODEL_ROLE_MAIN.to_string()),
        });
    }

    fn parse_success_body_with_id(
        &self,
        text: &str,
    ) -> Result<(Message, Usage, Option<String>), String> {
        let trimmed = text.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() {
            return Err("模型返回空响应：正文为空".to_string());
        }
        if let Some(error) = extract_gateway_error(trimmed) {
            return Err(format_gateway_error(&error));
        }
        if self.config.protocol == PROTOCOL_CODEX {
            if let Ok(parsed) = parse_responses_sse_with_id(trimmed) {
                return Ok(parsed);
            }
        } else if let Ok(parsed) = self.parse_sse(trimmed) {
            return Ok((parsed.0, parsed.1, None));
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let payload = unwrap_gateway_payload(&value);
            if let Some(error) = json_error_message(payload) {
                return Err(format_gateway_error(&error));
            }
            if self.config.protocol == PROTOCOL_CODEX {
                if let Ok(parsed) = parse_responses_json_with_id(payload) {
                    return Ok(parsed);
                }
            } else if let Ok(parsed) = self.parse_complete_json(payload) {
                return Ok((parsed.0, parsed.1, None));
            }
        }
        Err(empty_response_error(trimmed))
    }

    fn parse_complete_json(&self, value: &Value) -> Result<(Message, Usage), String> {
        match self.config.protocol.as_str() {
            PROTOCOL_ANTHROPIC => parse_anthropic_json(value),
            PROTOCOL_CODEX => parse_responses_json(value),
            PROTOCOL_OPENAI => parse_openai_json(value),
            other => Err(format!("不支持的渠道协议: {other}")),
        }
    }

    fn continuation_for(&self, key: &str, messages: &[Message]) -> Option<ResponseContinuation> {
        if self.config.protocol != PROTOCOL_CODEX {
            return None;
        }
        let mut continuations = self.continuations.lock().ok()?;
        let state = continuations.get(key).cloned()?;
        // A compacted or freshly started message list must not be attached to
        // an old server response. The anchor check also isolates child agents
        // that share the same ModelClient instance.
        if state.anchor != continuation_anchor(messages)
            || messages.len() <= state.message_count
            || message_prefix_hash(messages, state.message_count) != state.prefix_hash
        {
            continuations.remove(key);
            return None;
        }
        Some(state)
    }

    fn update_continuation(&self, key: &str, messages: &[Message], response_id: Option<String>) {
        if self.config.protocol != PROTOCOL_CODEX {
            return;
        }
        let Ok(mut continuations) = self.continuations.lock() else {
            return;
        };
        let Some(response_id) = response_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continuations.remove(key);
            return;
        };
        continuations.insert(
            key.to_string(),
            ResponseContinuation {
                response_id,
                message_count: messages.len(),
                anchor: continuation_anchor(messages),
                prefix_hash: message_prefix_hash(messages, messages.len()),
            },
        );
    }

    fn clear_continuation(&self, key: &str) {
        if let Ok(mut continuations) = self.continuations.lock() {
            continuations.remove(key);
        }
    }
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn continuation_key(messages: &[Message]) -> String {
    let mut hasher = DefaultHasher::new();
    for message in messages.iter().take(2) {
        std::mem::discriminant(&message.role).hash(&mut hasher);
        message.content.hash(&mut hasher);
        message.tool_call_id.hash(&mut hasher);
        message.name.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn continuation_anchor(messages: &[Message]) -> String {
    continuation_key(messages)
}

fn message_prefix_hash(messages: &[Message], count: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    for message in messages.iter().take(count) {
        std::mem::discriminant(&message.role).hash(&mut hasher);
        message.content.hash(&mut hasher);
        message.reasoning_content.hash(&mut hasher);
        message.tool_call_id.hash(&mut hasher);
        message.name.hash(&mut hasher);
        for call in &message.tool_calls {
            call.id.hash(&mut hasher);
            call.name.hash(&mut hasher);
            call.arguments.hash(&mut hasher);
        }
        for image in &message.images {
            image.name.hash(&mut hasher);
            image.mime_type.hash(&mut hasher);
            image.data_base64.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn is_continuation_rejection(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    // Gateways vary in how they report an unsupported continuation field:
    // some return 4xx, while others return HTTP 200 with an `{error: ...}`
    // body. The caller only checks this after a continuation was attached, so
    // the field-specific match is sufficient and avoids retrying ten times.
    [
        "previous_response_id",
        "previous response",
        "response_id",
        "response id",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn extract_gateway_error(text: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return json_error_message(&value)
            .or_else(|| json_error_message(unwrap_gateway_payload(&value)));
    }
    for event in parse_sse(text) {
        let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
            continue;
        };
        // `response.failed` nests the reason one level down instead of
        // reporting a top-level `error` object.
        if let Some(error) = json_error_message(&value)
            .or_else(|| value.get("response").and_then(json_error_message))
        {
            return Some(error);
        }
    }
    None
}

fn json_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if error.is_null() || error.as_object().is_some_and(serde_json::Map::is_empty) {
        return None;
    }
    if let Some(text) = error.as_str() {
        let text = text.trim();
        return if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        };
    }
    if let Some(text) = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(text.to_string());
    }
    if let Some(text) = error
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(text.to_string());
    }
    Some("未知错误".to_string())
}

fn unwrap_gateway_payload(value: &Value) -> &Value {
    for key in ["data", "result"] {
        if let Some(inner) = value.get(key) {
            if inner.get("choices").is_some()
                || inner.get("content").is_some()
                || inner.get("output").is_some()
                || inner.get("error").is_some()
            {
                return inner;
            }
        }
    }
    value
}

fn format_gateway_error(message: &str) -> String {
    let snippet = snippet_for_error(message);
    format!("模型返回错误：{snippet}")
}

fn empty_response_error(text: &str) -> String {
    let snippet = snippet_for_error(text);
    if snippet.is_empty() {
        "模型返回空响应".to_string()
    } else {
        format!("模型返回空响应：{snippet}")
    }
}

fn snippet_for_error(text: &str) -> String {
    redact_secrets(text)
        .chars()
        .filter(|ch| *ch != '\n' && *ch != '\r')
        .take(180)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const OK_SSE: &str =
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut chunk).await.expect("read request");
            if read == 0 {
                return bytes;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
                header_end = position + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while bytes.len().saturating_sub(header_end) < content_length {
            let read = stream.read(&mut chunk).await.expect("read request body");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        bytes
    }

    async fn serve_once(status: u16, body: &str) -> String {
        serve_sequence(vec![(status, body.to_string())]).await
    }

    async fn serve_sequence(responses: Vec<(u16, String)>) -> String {
        serve_sequence_counted(responses, None).await
    }

    async fn serve_sequence_counted(
        responses: Vec<(u16, String)>,
        counter: Option<Arc<AtomicU32>>,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("accept");
                if let Some(counter) = &counter {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                let _ = read_http_request(&mut stream).await;
                let header = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        format!("http://{addr}")
    }

    async fn serve_capture_sequence(
        responses: Vec<(u16, String)>,
    ) -> (String, Arc<Mutex<Vec<Value>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let bytes = read_http_request(&mut stream).await;
                if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
                    let header_end = position + 4;
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes[header_end..]) {
                        captured.lock().expect("captured requests").push(value);
                    }
                }
                let header = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), requests)
    }

    fn client(base_url: String) -> ModelClient {
        client_with_protocol(base_url, PROTOCOL_OPENAI)
    }

    fn client_with_protocol(base_url: String, protocol: &str) -> ModelClient {
        client_with_retry_protocol(base_url, protocol, RetryConfig::none())
    }

    fn fast_retry() -> RetryConfig {
        RetryConfig::fixed(10, 1)
    }

    fn client_with_retry(base_url: String, retry: RetryConfig) -> ModelClient {
        client_with_retry_protocol(base_url, PROTOCOL_OPENAI, retry)
    }

    fn client_with_retry_protocol(
        base_url: String,
        protocol: &str,
        retry: RetryConfig,
    ) -> ModelClient {
        ModelClient::new(ModelClientConfig {
            protocol: protocol.to_string(),
            base_url,
            api_key: "sk-secret-key".to_string(),
            extra_headers: HashMap::new(),
            retry,
            timeout: Duration::from_secs(5),
            network: NetworkSettings::default(),
        })
        .expect("client")
    }

    async fn chat_hi_on(client: ModelClient) -> Result<(Message, Usage), String> {
        client
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "gpt-4o",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
    }

    #[tokio::test]
    async fn chat_parses_mock_openai_sse() {
        let base = serve_once(200, OK_SSE).await;
        let (message, _) = chat_hi_on(client(base)).await.expect("chat");
        assert_eq!(message.content, "ok");
    }

    #[tokio::test]
    async fn list_models_reads_openai_payload() {
        let body = r#"{"data":[{"id":"gpt-4o"},{"id":"o4-mini"}]}"#;
        let base = serve_once(200, body).await;
        let listed = client(base).list_models().await.expect("list models");
        assert_eq!(
            listed.models,
            vec!["gpt-4o".to_string(), "o4-mini".to_string()]
        );
        assert!(!listed.truncated);
    }

    #[tokio::test]
    async fn list_models_marks_truncated_when_page_exceeds_cap() {
        let items = (0..501)
            .map(|index| format!(r#"{{"id":"model-{index:03}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(r#"{{"data":[{items}]}}"#);
        let listed = client(serve_once(200, &body).await)
            .list_models()
            .await
            .expect("list models");
        assert_eq!(listed.models.len(), 500);
        assert!(listed.truncated);
    }

    #[tokio::test]
    async fn probe_error_hides_api_key() {
        let base = serve_once(401, "Authorization Bearer sk-secret-key denied").await;
        let error = client(base)
            .probe("gpt-4o")
            .await
            .expect_err("probe should fail");
        assert!(error.contains("HTTP 401"));
        assert!(!error.contains("sk-secret-key"));
    }

    fn capturing_delta_hook() -> (DeltaHook, Arc<Mutex<Vec<StreamDelta>>>) {
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let captured = deltas.clone();
        let hook: DeltaHook = Arc::new(move |delta: StreamDelta| {
            captured.lock().expect("deltas").push(delta);
        });
        (hook, deltas)
    }

    /// Serve one chunked response, pausing before each chunk so a test can
    /// observe output that arrives before the body is complete.
    async fn serve_delayed_chunks(chunks: Vec<(u64, String)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let _ = read_http_request(&mut stream).await;
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(header.as_bytes()).await;
            for (delay_ms, body) in chunks {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                let chunk = format!("{:x}\r\n{}\r\n", body.len(), body);
                let _ = stream.write_all(chunk.as_bytes()).await;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn chat_emits_text_deltas_before_the_body_completes() {
        let base = serve_delayed_chunks(vec![
            (
                0,
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi \"}}]}\n\n".to_string(),
            ),
            (
                400,
                "data: {\"choices\":[{\"delta\":{\"content\":\"there\"}}]}\n\ndata: [DONE]\n\n"
                    .to_string(),
            ),
        ])
        .await;
        let (hook, deltas) = capturing_delta_hook();
        let client = client(base).with_delta_hook(hook);
        let task = tokio::spawn(async move { chat_hi_on(client).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            deltas.lock().expect("deltas").as_slice(),
            [StreamDelta::Text("hi ".to_string())],
            "the first delta must arrive while the response is still open"
        );
        let (message, _) = task.await.expect("join").expect("chat");
        assert_eq!(message.content, "hi there");
        assert_eq!(
            deltas.lock().expect("deltas").as_slice(),
            [
                StreamDelta::Text("hi ".to_string()),
                StreamDelta::Text("there".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn chat_emits_reasoning_deltas_for_responses_protocol() {
        let sse = concat!(
            "event: response.reasoning_text.delta\ndata: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"think\"}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        );
        let base = serve_once(200, sse).await;
        let (hook, deltas) = capturing_delta_hook();
        let (message, _) = client_with_protocol(base, PROTOCOL_CODEX)
            .with_delta_hook(hook)
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: true,
            })
            .await
            .expect("chat");
        assert_eq!(message.content, "ok");
        assert_eq!(message.reasoning_content, "think");
        assert_eq!(
            deltas.lock().expect("deltas").as_slice(),
            [
                StreamDelta::Reasoning("think".to_string()),
                StreamDelta::Text("ok".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn chat_resets_deltas_before_retrying() {
        let base = serve_sequence(vec![(503, "busy".to_string()), (200, OK_SSE.to_string())]).await;
        let (hook, deltas) = capturing_delta_hook();
        let (message, _) = chat_hi_on(client_with_retry(base, fast_retry()).with_delta_hook(hook))
            .await
            .expect("retry then success");
        assert_eq!(message.content, "ok");
        assert_eq!(
            deltas.lock().expect("deltas").as_slice(),
            [StreamDelta::Reset, StreamDelta::Text("ok".to_string())]
        );
    }

    #[tokio::test]
    async fn complete_json_response_emits_no_deltas() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"json ok"}}]}"#;
        let base = serve_once(200, body).await;
        let (hook, deltas) = capturing_delta_hook();
        let (message, _) = chat_hi_on(client(base).with_delta_hook(hook))
            .await
            .expect("json chat");
        assert_eq!(message.content, "json ok");
        assert!(deltas.lock().expect("deltas").is_empty());
    }

    #[tokio::test]
    async fn cloned_client_does_not_inherit_delta_hook() {
        let base = serve_once(200, OK_SSE).await;
        let (hook, deltas) = capturing_delta_hook();
        let parent = client(base).with_delta_hook(hook);
        chat_hi_on(parent.clone_for_conversation())
            .await
            .expect("chat");
        assert!(deltas.lock().expect("deltas").is_empty());
    }

    #[tokio::test]
    async fn chat_retries_when_max_tokens_exceeds_gateway_limit() {
        let error = r#"{"error":{"message":"max_tokens is too large: 384000. This model supports at most 131072 completion tokens."}}"#;
        let base = serve_sequence(vec![(400, error.to_string()), (200, OK_SSE.to_string())]).await;
        let (message, _) = client(base)
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "deepseek-v4-flash",
                effort: None,
                max_output_tokens: Some(384000),
                thinking_enabled: true,
            })
            .await
            .expect("chat should retry with gateway limit");
        assert_eq!(message.content, "ok");
    }

    #[tokio::test]
    async fn chat_retries_when_max_tokens_exceeds_gateway_limit_twice() {
        let first = r#"{"error":{"message":"max_tokens is too large: 384000. This model supports at most 131072 completion tokens."}}"#;
        let second = r#"{"error":{"message":"max_tokens is too large: 131072. This model supports at most 8192 completion tokens."}}"#;
        let base = serve_sequence(vec![
            (400, first.to_string()),
            (400, second.to_string()),
            (200, OK_SSE.to_string()),
        ])
        .await;
        let (message, _) = client(base)
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "deepseek-v4-flash",
                effort: None,
                max_output_tokens: Some(384000),
                thinking_enabled: true,
            })
            .await
            .expect("chat should retry with two smaller gateway limits");
        assert_eq!(message.content, "ok");
    }

    async fn chat_hi(base: String) -> Result<(Message, Usage), String> {
        chat_hi_on(client(base)).await
    }

    #[tokio::test]
    async fn chat_parses_non_stream_json() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"json ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#;
        let (message, usage) = chat_hi(serve_once(200, body).await)
            .await
            .expect("json chat");
        assert_eq!(message.content, "json ok");
        assert_eq!(usage.prompt_tokens, 1);
    }

    #[tokio::test]
    async fn chat_parses_wrapped_gateway_json() {
        let body = r#"{"data":{"choices":[{"message":{"content":"wrapped"}}]}}"#;
        let (message, _) = chat_hi(serve_once(200, body).await)
            .await
            .expect("wrapped json");
        assert_eq!(message.content, "wrapped");
    }

    #[tokio::test]
    async fn chat_surfaces_json_error_on_http_200() {
        let body = r#"{"error":{"message":"insufficient quota for sk-secret-key"}}"#;
        let error = chat_hi(serve_once(200, body).await)
            .await
            .expect_err("json error should fail");
        assert!(error.contains("模型返回错误"));
        assert!(error.contains("insufficient quota"));
        assert!(!error.contains("空响应"));
        assert!(!error.contains("sk-secret-key"));
    }

    #[tokio::test]
    async fn chat_empty_body_includes_empty_hint() {
        let error = chat_hi(serve_once(200, "").await)
            .await
            .expect_err("empty body");
        assert!(error.contains("模型返回空响应"));
        assert!(error.contains("正文为空"));
    }

    #[tokio::test]
    async fn chat_parses_responses_completed_json() {
        let body = r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"done plan"}]}],"usage":{"input_tokens":2,"output_tokens":1}}"#;
        let (message, _) = client_with_protocol(serve_once(200, body).await, PROTOCOL_CODEX)
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("responses json");
        assert_eq!(message.content, "done plan");
    }

    #[tokio::test]
    async fn responses_reuses_previous_response_id_and_only_sends_new_items() {
        let first = r#"{"id":"resp_1","output":[{"type":"message","content":[{"type":"output_text","text":"need tool"}]}]}"#;
        let second = r#"{"id":"resp_2","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]}"#;
        let (base, requests) =
            serve_capture_sequence(vec![(200, first.to_string()), (200, second.to_string())]).await;
        let client = client_with_protocol(base, PROTOCOL_CODEX);
        let mut messages = vec![Message::system("rules"), Message::user("inspect")];
        client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("first response");
        messages.push(Message::assistant_text("need tool"));
        messages.push(Message::tool_result("call_1", "tool output"));
        client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("continued response");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].get("previous_response_id").is_none());
        assert_eq!(requests[0]["input"].as_array().map(Vec::len), Some(1));
        assert_eq!(requests[1]["previous_response_id"], "resp_1");
        let input = requests[1]["input"].as_array().expect("continued input");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call_output");
    }

    #[tokio::test]
    async fn responses_falls_back_when_gateway_rejects_continuation() {
        let first = r#"{"id":"resp_1","output":[{"type":"message","content":[{"type":"output_text","text":"first"}]}]}"#;
        let rejected = r#"{"error":{"message":"unknown parameter previous_response_id"}}"#;
        let third = r#"{"id":"resp_3","output":[{"type":"message","content":[{"type":"output_text","text":"fallback"}]}]}"#;
        let (base, requests) = serve_capture_sequence(vec![
            (200, first.to_string()),
            (400, rejected.to_string()),
            (200, third.to_string()),
        ])
        .await;
        let client = client_with_protocol(base, PROTOCOL_CODEX);
        let mut messages = vec![Message::system("rules"), Message::user("inspect")];
        client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("first response");
        messages.push(Message::assistant_text("first"));
        messages.push(Message::tool_result("call_1", "tool output"));
        let (message, _) = client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("fallback response");
        assert_eq!(message.content, "fallback");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1]["previous_response_id"], "resp_1");
        assert!(requests[2].get("previous_response_id").is_none());
        assert_eq!(requests[2]["input"].as_array().map(Vec::len), Some(3));
    }

    #[tokio::test]
    async fn responses_falls_back_when_http_200_rejects_continuation() {
        let first = r#"{"id":"resp_1","output":[{"type":"message","content":[{"type":"output_text","text":"first"}]}]}"#;
        let rejected = r#"{"error":{"message":"unsupported previous_response_id"}}"#;
        let fallback = r#"{"id":"resp_2","output":[{"type":"message","content":[{"type":"output_text","text":"fallback"}]}]}"#;
        let (base, requests) = serve_capture_sequence(vec![
            (200, first.to_string()),
            (200, rejected.to_string()),
            (200, fallback.to_string()),
        ])
        .await;
        let client = client_with_protocol(base, PROTOCOL_CODEX);
        let mut messages = vec![Message::system("rules"), Message::user("inspect")];
        client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("first response");
        messages.push(Message::assistant_text("first"));
        messages.push(Message::tool_result("call_1", "tool output"));
        let (message, _) = client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("fallback response");
        assert_eq!(message.content, "fallback");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1]["previous_response_id"], "resp_1");
        assert!(requests[2].get("previous_response_id").is_none());
        assert_eq!(requests[2]["input"].as_array().map(Vec::len), Some(3));
    }

    #[tokio::test]
    async fn cloned_responses_client_does_not_inherit_parent_continuation() {
        let first = r#"{"id":"resp_parent","output":[{"type":"message","content":[{"type":"output_text","text":"first"}]}]}"#;
        let second = r#"{"id":"resp_child","output":[{"type":"message","content":[{"type":"output_text","text":"child"}]}]}"#;
        let (base, requests) =
            serve_capture_sequence(vec![(200, first.to_string()), (200, second.to_string())]).await;
        let parent = client_with_protocol(base, PROTOCOL_CODEX);
        let mut messages = vec![Message::system("rules"), Message::user("inspect")];
        parent
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("parent response");
        messages.push(Message::assistant_text("first"));
        let child = parent.clone();
        child
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("child response");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 2);
        assert!(requests[1].get("previous_response_id").is_none());
        assert_eq!(requests[1]["input"].as_array().map(Vec::len), Some(2));
    }

    #[tokio::test]
    async fn conversation_clone_preserves_responses_continuation() {
        let first = r#"{"id":"resp_parent","output":[{"type":"message","content":[{"type":"output_text","text":"first"}]}]}"#;
        let second = r#"{"id":"resp_next","output":[{"type":"message","content":[{"type":"output_text","text":"next"}]}]}"#;
        let (base, requests) =
            serve_capture_sequence(vec![(200, first.to_string()), (200, second.to_string())]).await;
        let client = client_with_protocol(base, PROTOCOL_CODEX);
        let mut messages = vec![Message::system("rules"), Message::user("inspect")];
        client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("first response");
        messages.push(Message::assistant_text("first"));
        let session_client = client.clone_for_conversation();
        messages.push(Message::user("continue"));
        session_client
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.4",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("continued response");

        let requests = requests.lock().expect("captured requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["previous_response_id"], "resp_parent");
        assert_eq!(requests[1]["input"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn chat_retries_http_503_then_succeeds() {
        let counter = Arc::new(AtomicU32::new(0));
        let base = serve_sequence_counted(
            vec![(503, "busy".to_string()), (200, OK_SSE.to_string())],
            Some(counter.clone()),
        )
        .await;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let captured = lines.clone();
        let (message, _) = chat_hi_on(client_with_retry(base, fast_retry()).with_retry_hook(
            Arc::new(move |line: &str| {
                captured.lock().expect("retry lines").push(line.to_string());
            }),
        ))
        .await
        .expect("503 then success");
        assert_eq!(message.content, "ok");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        let lines = lines.lock().expect("retry lines");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[重试]"));
        assert!(lines[0].contains("1/10"));
        assert!(lines[0].contains("HTTP 503"));
    }

    #[tokio::test]
    async fn chat_retries_http_200_gateway_error_then_succeeds() {
        let error = r#"{"error":{"message":"overloaded"}}"#;
        let base = serve_sequence(vec![(200, error.to_string()), (200, OK_SSE.to_string())]).await;
        let (message, _) = chat_hi_on(client_with_retry(base, fast_retry()))
            .await
            .expect("gateway error then success");
        assert_eq!(message.content, "ok");
    }

    #[tokio::test]
    async fn chat_retries_http_200_empty_then_succeeds() {
        let base = serve_sequence(vec![(200, String::new()), (200, OK_SSE.to_string())]).await;
        let (message, _) = chat_hi_on(client_with_retry(base, fast_retry()))
            .await
            .expect("empty then success");
        assert_eq!(message.content, "ok");
    }

    #[tokio::test]
    async fn chat_retry_none_does_not_retry() {
        let counter = Arc::new(AtomicU32::new(0));
        let base =
            serve_sequence_counted(vec![(503, "busy".to_string())], Some(counter.clone())).await;
        let error = chat_hi_on(client_with_retry(base, RetryConfig::none()))
            .await
            .expect_err("none should fail immediately");
        assert!(error.contains("HTTP 503"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn chat_retry_line_redacts_api_key() {
        let body = "Authorization: Bearer sk-secret-key gateway busy";
        let base = serve_sequence(vec![(503, body.to_string()), (200, OK_SSE.to_string())]).await;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let captured = lines.clone();
        chat_hi_on(
            client_with_retry(base, fast_retry()).with_retry_hook(Arc::new(move |line: &str| {
                captured.lock().expect("retry lines").push(line.to_string());
            })),
        )
        .await
        .expect("retry then success");
        let lines = lines.lock().expect("retry lines");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[重试]"));
        assert!(!lines[0].contains("sk-secret-key"));
        assert!(!lines[0].to_ascii_lowercase().contains("bearer sk"));
    }

    #[tokio::test]
    async fn chat_does_not_retry_unauthorized() {
        let counter = Arc::new(AtomicU32::new(0));
        let base =
            serve_sequence_counted(vec![(401, "denied".to_string())], Some(counter.clone())).await;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let captured = lines.clone();
        let error = chat_hi_on(
            client_with_retry(base, fast_retry()).with_retry_hook(Arc::new(move |line: &str| {
                captured.lock().expect("retry lines").push(line.to_string());
            })),
        )
        .await
        .expect_err("401 should fail");
        assert!(error.contains("HTTP 401"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(lines.lock().expect("retry lines").is_empty());
    }

    #[tokio::test]
    async fn chat_max_tokens_limit_skips_http_retry_budget() {
        let error = r#"{"error":{"message":"max_tokens is too large: 384000. This model supports at most 131072 completion tokens."}}"#;
        let counter = Arc::new(AtomicU32::new(0));
        let base = serve_sequence_counted(
            vec![(400, error.to_string()), (200, OK_SSE.to_string())],
            Some(counter.clone()),
        )
        .await;
        let (message, _) = client_with_retry(base, fast_retry())
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "deepseek-v4-flash",
                effort: None,
                max_output_tokens: Some(384000),
                thinking_enabled: true,
            })
            .await
            .expect("max_tokens one-shot retry");
        assert_eq!(message.content, "ok");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn chat_cancels_during_retry_wait() {
        let base = serve_once(503, "busy").await;
        let cancel = CancelFlag::new();
        let client =
            client_with_retry(base, RetryConfig::fixed(10, 2_000)).with_cancel(cancel.clone());
        let task = tokio::spawn(async move { chat_hi_on(client).await });
        tokio::time::sleep(Duration::from_millis(80)).await;
        cancel.cancel();
        let error = task
            .await
            .expect("join")
            .expect_err("retry wait should cancel");
        assert_eq!(error, "已取消");
    }

    fn capturing_sink() -> (CallLogSink, Arc<Mutex<Vec<NativeApiCallLogInsert>>>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let sink: CallLogSink = Arc::new(move |record: NativeApiCallLogInsert| {
            captured.lock().expect("call logs").push(record);
        });
        (sink, records)
    }

    #[tokio::test]
    async fn chat_writes_success_call_log_with_first_token() {
        let ping = ": keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4,\"prompt_tokens_details\":{\"cached_tokens\":3}}}\n\ndata: [DONE]\n\n";
        let base = serve_once(200, ping).await;
        let (sink, records) = capturing_sink();
        let _ = client(base)
            .with_call_log(
                CallLogContext {
                    channel_id: Some("ch-1".to_string()),
                    channel_name: Some("OpenAI".to_string()),
                    session_id: Some("sess-1".to_string()),
                    profile_id: Some("emp-1".to_string()),
                    workspace_id: Some("ws-1".to_string()),
                    subagent_id: None,
                    call_kind: Some("chat".to_string()),
                    execution_target: Some("local".to_string()),
                    operation: None,
                    model_role: None,
                },
                sink,
            )
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "gpt-4o",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("chat");
        let records = records.lock().expect("call logs");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, CALL_STATUS_SUCCESS);
        assert_eq!(records[0].attempt, 1);
        assert_eq!(records[0].model.as_deref(), Some("gpt-4o"));
        assert_eq!(records[0].channel_name.as_deref(), Some("OpenAI"));
        assert_eq!(records[0].input_tokens, Some(10));
        assert_eq!(records[0].output_tokens, Some(4));
        assert_eq!(records[0].cached_tokens, Some(3));
        assert_eq!(records[0].total_tokens, Some(14));
        assert!(records[0].first_token_ms.is_some());
        assert!(records[0].duration_ms.is_some());
        let request = records[0].request_body.as_deref().unwrap_or_default();
        assert!(!request.contains("sk-secret-key"));
        assert!(!request.to_ascii_lowercase().contains("bearer"));
    }

    #[tokio::test]
    async fn chat_writes_failed_and_success_attempts() {
        let base = serve_sequence(vec![(503, "busy".to_string()), (200, OK_SSE.to_string())]).await;
        let (sink, records) = capturing_sink();
        chat_hi_on(
            client_with_retry(base, fast_retry()).with_call_log(CallLogContext::default(), sink),
        )
        .await
        .expect("retry then success");
        let records = records.lock().expect("call logs");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, CALL_STATUS_FAILED);
        assert_eq!(records[0].attempt, 1);
        assert_eq!(records[0].http_status, Some(503));
        assert_eq!(records[1].status, CALL_STATUS_SUCCESS);
        assert_eq!(records[1].attempt, 2);
        assert_eq!(records[0].call_id, records[1].call_id);
    }

    #[tokio::test]
    async fn chat_without_sink_does_not_write_call_log() {
        let base = serve_once(200, OK_SSE).await;
        chat_hi_on(client(base)).await.expect("chat");
    }

    #[tokio::test]
    async fn probe_does_not_write_call_log() {
        let base = serve_once(200, r#"{"choices":[{"message":{"content":"ok"}}]}"#).await;
        let (sink, records) = capturing_sink();
        client(base)
            .with_call_log(CallLogContext::default(), sink)
            .probe("gpt-4o")
            .await
            .expect("probe");
        assert!(records.lock().expect("call logs").is_empty());
    }

    #[tokio::test]
    async fn chat_cancel_during_retry_wait_reuses_failed_attempt() {
        let base = serve_once(503, "busy").await;
        let cancel = CancelFlag::new();
        let (sink, records) = capturing_sink();
        let client = client_with_retry(base, RetryConfig::fixed(10, 2_000))
            .with_cancel(cancel.clone())
            .with_call_log(CallLogContext::default(), sink);
        let task = tokio::spawn(async move { chat_hi_on(client).await });
        tokio::time::sleep(Duration::from_millis(250)).await;
        cancel.cancel();
        let error = task
            .await
            .expect("join")
            .expect_err("retry wait should cancel");
        assert_eq!(error, "已取消");
        let records = records.lock().expect("call logs");
        assert!(
            records.len() >= 2,
            "取消重试等待应至少留下失败与取消两条日志，实际 {}",
            records.len()
        );
        assert_eq!(records[0].status, CALL_STATUS_FAILED);
        assert_eq!(records[0].attempt, 1);
        assert_eq!(records[1].status, CALL_STATUS_CANCELLED);
        assert_eq!(records[1].attempt, 1);
    }

    #[tokio::test]
    async fn chat_logs_partial_body_when_cancelled_mid_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let _ = read_http_request(&mut stream).await;
            let body = OK_SSE;
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(header.as_bytes()).await;
            let chunk = format!("{:x}\r\n{}\r\n", body.len(), body);
            let _ = stream.write_all(chunk.as_bytes()).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
            let _ = stream.write_all(b"0\r\n\r\n").await;
        });
        let cancel = CancelFlag::new();
        let (sink, records) = capturing_sink();
        let client = client(format!("http://{addr}"))
            .with_cancel(cancel.clone())
            .with_call_log(CallLogContext::default(), sink);
        let task = tokio::spawn(async move { chat_hi_on(client).await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        let error = task.await.expect("join").expect_err("stream cancel");
        assert_eq!(error, "已取消");
        let records = records.lock().expect("call logs");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, CALL_STATUS_CANCELLED);
        assert!(records[0].duration_ms.is_some());
        assert!(records[0]
            .response_body
            .as_deref()
            .is_some_and(|body| body.contains("ok") || !body.is_empty()));
    }

    #[tokio::test]
    async fn chat_logs_thinking_disabled_for_deepseek_toggle() {
        let base = serve_once(200, OK_SSE).await;
        let (sink, records) = capturing_sink();
        client(base)
            .with_call_log(CallLogContext::default(), sink)
            .chat(ChatRequest {
                messages: &[Message::user("hi")],
                tools: &[],
                model: "deepseek-v4-flash",
                effort: None,
                max_output_tokens: None,
                thinking_enabled: false,
            })
            .await
            .expect("chat");
        let records = records.lock().expect("call logs");
        assert_eq!(records.len(), 1);
        assert!(!records[0].thinking_enabled);
        assert_eq!(records[0].thinking_level, None);
        assert!(records[0]
            .request_body
            .as_deref()
            .is_some_and(|body| body.contains("\"type\":\"disabled\"")
                || body.contains("\"type\": \"disabled\"")));
    }

    #[tokio::test]
    #[ignore = "需要 NOXCODE_TEST_CHANNEL_* 环境变量指向真实渠道"]
    async fn live_channel_chat_and_list_models() {
        let protocol =
            std::env::var("NOXCODE_TEST_CHANNEL_PROTOCOL").expect("NOXCODE_TEST_CHANNEL_PROTOCOL");
        let base_url =
            std::env::var("NOXCODE_TEST_CHANNEL_BASE_URL").expect("NOXCODE_TEST_CHANNEL_BASE_URL");
        let api_key =
            std::env::var("NOXCODE_TEST_CHANNEL_API_KEY").expect("NOXCODE_TEST_CHANNEL_API_KEY");
        let model =
            std::env::var("NOXCODE_TEST_CHANNEL_MODEL").expect("NOXCODE_TEST_CHANNEL_MODEL");
        let client = ModelClient::new(ModelClientConfig {
            protocol,
            base_url,
            api_key,
            extra_headers: HashMap::new(),
            retry: RetryConfig::none(),
            timeout: Duration::from_secs(30),
            network: NetworkSettings::default(),
        })
        .expect("live client");
        let listed = client.list_models().await.expect("list_models");
        assert!(!listed.models.is_empty(), "真实渠道应至少返回一个模型");
        let (message, _) = client
            .chat(ChatRequest {
                messages: &[Message::user("回复一个字：好")],
                tools: &[],
                model: &model,
                effort: None,
                max_output_tokens: Some(16),
                thinking_enabled: false,
            })
            .await
            .expect("chat");
        assert!(
            !message.content.trim().is_empty() || !message.reasoning_content.trim().is_empty(),
            "真实渠道 chat 应返回文本或思考内容"
        );
    }
}
