//! P3 精简版：仅含 `probe` / `list_models`。
//! P4.1 用完整 `client.rs` 覆盖本文件；请保留 `ModelClientConfig.network` 与
//! `new()` 里对 `build_http_client` 的调用。

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::app::network_settings::{build_http_client, NetworkSettings};
use crate::native::protocol::{
    channel_chat_url, channel_models_url, model_list_next_page, parse_model_list_json,
    PROTOCOL_ANTHROPIC, PROTOCOL_CODEX, PROTOCOL_OPENAI,
};

use super::retry::{format_http_error, RetryConfig};

const MODEL_LIST_PAGE_LIMIT: usize = 20;
const MODEL_LIST_ITEM_LIMIT: usize = 500;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListedModels {
    pub models: Vec<String>,
    pub truncated: bool,
}

pub struct ModelClient {
    http: reqwest::Client,
    config: ModelClientConfig,
}

impl ModelClient {
    pub fn new(config: ModelClientConfig) -> Result<Self, String> {
        let http = build_http_client(config.timeout, &config.network)?;
        Ok(Self { http, config })
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
        let body = probe_body(&self.config.protocol, model)?;
        let (status, text) = self.post_json(&url, &body).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(format_http_error(status, &url, &text))
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

    async fn post_json(&self, url: &str, body: &Value) -> Result<(u16, String), String> {
        let request = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json");
        let response = self
            .apply_auth(request)
            .json(body)
            .send()
            .await
            .map_err(|error| format!("模型请求失败: {error}"))?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        Ok((status, text))
    }
}

fn probe_body(protocol: &str, model: &str) -> Result<Value, String> {
    match protocol {
        PROTOCOL_OPENAI => {
            let mut body = json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "stream": false,
                "max_tokens": 16,
            });
            if model.to_ascii_lowercase().contains("deepseek") {
                body["thinking"] = json!({"type": "disabled"});
            }
            Ok(body)
        }
        PROTOCOL_ANTHROPIC => Ok(json!({
            "model": model,
            "messages": [{"role": "user", "content": "ping"}],
            "stream": false,
            "max_tokens": 16,
        })),
        PROTOCOL_CODEX => Ok(json!({
            "model": model,
            "input": [{"role": "user", "content": "ping"}],
            "stream": false,
            "max_output_tokens": 16,
        })),
        other => Err(format!("不支持的渠道协议: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    #[derive(Clone)]
    enum MockMode {
        ProbeOk,
        ModelsPaged,
        ModelsTruncated,
        Unauthorized,
    }

    struct MockServer {
        base_url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        _shutdown: oneshot::Sender<()>,
    }

    async fn start_mock(mode: MockMode) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (tx, mut rx) = oneshot::channel::<()>();
        let recorded = requests.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let recorded = recorded.clone();
                        let mode = mode.clone();
                        tokio::spawn(async move {
                            handle_connection(&mut stream, &recorded, &mode).await;
                        });
                    }
                }
            }
        });
        MockServer {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
            _shutdown: tx,
        }
    }

    async fn handle_connection(
        stream: &mut tokio::net::TcpStream,
        recorded: &Arc<Mutex<Vec<RecordedRequest>>>,
        mode: &MockMode,
    ) {
        let mut buf = vec![0u8; 16 * 1024];
        let n = match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let raw = String::from_utf8_lossy(&buf[..n]);
        let Some((header_part, body_part)) = raw.split_once("\r\n\r\n") else {
            return;
        };
        let mut lines = header_part.split("\r\n");
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        let mut headers = HashMap::new();
        let mut content_length = 0usize;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.insert(name, value);
        }
        let mut body = body_part.as_bytes().to_vec();
        while body.len() < content_length {
            let extra = stream.read(&mut buf).await.unwrap_or(0);
            if extra == 0 {
                break;
            }
            body.extend_from_slice(&buf[..extra]);
        }
        body.truncate(content_length);
        let body = String::from_utf8_lossy(&body).into_owned();
        recorded.lock().expect("lock").push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            headers: headers.clone(),
            body: body.clone(),
        });

        let (status, payload) = match mode {
            MockMode::Unauthorized => (
                401,
                r#"{"error":"invalid Authorization: Bearer sk-secret-key"}"#.to_string(),
            ),
            MockMode::ProbeOk if method == "POST" => (200, r#"{"ok":true}"#.to_string()),
            MockMode::ModelsPaged if path.starts_with("/v1/models") => {
                if path.contains("after_id=m2") {
                    (
                        200,
                        r#"{"data":[{"id":"m3"}],"has_more":false}"#.to_string(),
                    )
                } else {
                    (
                        200,
                        r#"{"data":[{"id":"m1"},{"id":"m2"}],"has_more":true,"last_id":"m2"}"#
                            .to_string(),
                    )
                }
            }
            MockMode::ModelsTruncated if path.starts_with("/v1/models") => {
                let items: Vec<String> = (0..500)
                    .map(|i| format!(r#"{{"id":"model-{i:04}"}}"#))
                    .collect();
                (
                    200,
                    format!(
                        r#"{{"data":[{}],"has_more":true,"last_id":"model-0499"}}"#,
                        items.join(",")
                    ),
                )
            }
            _ => (404, r#"{"error":"not found"}"#.to_string()),
        };
        let response = format!(
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    fn client(protocol: &str, base_url: &str, extra: HashMap<String, String>) -> ModelClient {
        ModelClient::new(ModelClientConfig {
            protocol: protocol.to_string(),
            base_url: base_url.to_string(),
            api_key: "sk-secret-key".to_string(),
            extra_headers: extra,
            retry: RetryConfig::none(),
            timeout: Duration::from_secs(5),
            network: NetworkSettings::default(),
        })
        .expect("client")
    }

    fn header_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[tokio::test]
    async fn openai_probe_sends_bearer_and_body() {
        let server = start_mock(MockMode::ProbeOk).await;
        let model = client(PROTOCOL_OPENAI, &server.base_url, HashMap::new());
        model.probe("gpt-4o").await.expect("probe");
        let req = server
            .requests
            .lock()
            .expect("lock")
            .last()
            .cloned()
            .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/chat/completions");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer sk-secret-key")
        );
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 16);
        assert_eq!(body["messages"][0]["content"], "ping");
        assert!(body.get("thinking").is_none());
    }

    #[tokio::test]
    async fn deepseek_probe_disables_thinking() {
        let server = start_mock(MockMode::ProbeOk).await;
        let model = client(PROTOCOL_OPENAI, &server.base_url, HashMap::new());
        model.probe("deepseek-chat").await.expect("probe");
        let req = server
            .requests
            .lock()
            .expect("lock")
            .last()
            .cloned()
            .unwrap();
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["thinking"]["type"], "disabled");
    }

    #[tokio::test]
    async fn anthropic_probe_uses_x_api_key() {
        let server = start_mock(MockMode::ProbeOk).await;
        let model = client(PROTOCOL_ANTHROPIC, &server.base_url, HashMap::new());
        model.probe("claude-sonnet-4").await.expect("probe");
        let req = server
            .requests
            .lock()
            .expect("lock")
            .last()
            .cloned()
            .unwrap();
        assert_eq!(req.path, "/v1/messages");
        assert_eq!(
            req.headers.get("x-api-key").map(String::as_str),
            Some("sk-secret-key")
        );
        assert_eq!(
            req.headers.get("anthropic-version").map(String::as_str),
            Some("2023-06-01")
        );
        assert!(!req.headers.contains_key("authorization"));
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["max_tokens"], 16);
        assert!(body.get("thinking").is_none());
    }

    #[tokio::test]
    async fn codex_probe_uses_responses_shape() {
        let server = start_mock(MockMode::ProbeOk).await;
        let model = client(PROTOCOL_CODEX, &server.base_url, HashMap::new());
        model.probe("gpt-5.6-luna").await.expect("probe");
        let req = server
            .requests
            .lock()
            .expect("lock")
            .last()
            .cloned()
            .unwrap();
        assert_eq!(req.path, "/v1/responses");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer sk-secret-key")
        );
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body["input"][0]["content"], "ping");
        assert_eq!(body["max_output_tokens"], 16);
        assert!(body.get("max_tokens").is_none());
    }

    #[tokio::test]
    async fn extra_headers_cannot_override_auth() {
        let server = start_mock(MockMode::ProbeOk).await;
        let extras = header_map(&[
            ("Authorization", "Bearer hijacked"),
            ("x-api-key", "stolen"),
            ("X-Custom", "ok"),
        ]);
        let model = client(PROTOCOL_OPENAI, &server.base_url, extras);
        model.probe("gpt-4o").await.expect("probe");
        let req = server
            .requests
            .lock()
            .expect("lock")
            .last()
            .cloned()
            .unwrap();
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer sk-secret-key")
        );
        assert_eq!(req.headers.get("x-custom").map(String::as_str), Some("ok"));
        assert_ne!(
            req.headers.get("x-api-key").map(String::as_str),
            Some("stolen")
        );
    }

    #[tokio::test]
    async fn list_models_paginates_and_sorts() {
        let server = start_mock(MockMode::ModelsPaged).await;
        let model = client(PROTOCOL_OPENAI, &server.base_url, HashMap::new());
        let listed = model.list_models().await.expect("list");
        assert_eq!(listed.models, vec!["m1", "m2", "m3"]);
        assert!(!listed.truncated);
        assert_eq!(server.requests.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn list_models_marks_truncated_at_500() {
        let server = start_mock(MockMode::ModelsTruncated).await;
        let model = client(PROTOCOL_OPENAI, &server.base_url, HashMap::new());
        let listed = model.list_models().await.expect("list");
        assert_eq!(listed.models.len(), 500);
        assert!(listed.truncated);
    }

    #[tokio::test]
    async fn http_error_redacts_secret() {
        let server = start_mock(MockMode::Unauthorized).await;
        let model = client(PROTOCOL_OPENAI, &server.base_url, HashMap::new());
        let err = model.probe("gpt-4o").await.expect_err("should fail");
        assert!(err.contains("HTTP 401"));
        assert!(!err.contains("sk-secret-key"));
        assert!(!err.to_ascii_lowercase().contains("bearer sk"));
    }
}
