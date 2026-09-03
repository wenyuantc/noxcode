//! MCP 客户端。
//!
//! 传输：`stdio`（本机子进程 / SSH 远端）、`http`（Streamable HTTP：POST JSON-RPC，
//! 响应为 JSON 或 SSE）、`sse`（旧版：GET 事件流 + `endpoint` 事件给出的 POST 地址）。
//! 协议：`tools/*`、`resources/list|read`、`prompts/list|get`；服务端发起的
//! `elicitation/create`（转成向用户提问）、`sampling/createMessage`（用当前模型作答）、
//! `roots/list`、`ping` 由 [`McpHostHandlers`] 处理。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use crate::app::network_settings::{load_network_settings, proxy_env_vars};
use crate::app::ssh::exec::{spawn_ssh_command, SshCommandStream, SshStreamEvent};
use crate::app::ssh::shell::shell_escape_single_quoted;
use crate::db::models::{
    McpServerConfig, SshConfigRecord, MCP_TRANSPORT_HTTP, MCP_TRANSPORT_SSE,
};
use crate::native::model::types::ToolSpec;
use crate::process_spawn::configure_tokio_command;

use super::cancel::CancelFlag;
use super::contract::ToolContract;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_HTTP_BODY: usize = 16 * 1024 * 1024;

/// 服务端 `elicitation/create` 的处理：拿到 message 与 requestedSchema，返回 `{action, content}`。
pub type ElicitHandler = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;
/// 服务端 `sampling/createMessage` 的处理：拿到 params，返回 `{role, content, model}`。
pub type SampleHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>> + Send + Sync,
>;

#[derive(Default, Clone)]
pub struct McpHostHandlers {
    pub elicit: Option<ElicitHandler>,
    pub sample: Option<SampleHandler>,
    /// `roots/list` 返回的工作区根（file:// URI）。
    pub roots: Vec<String>,
}

pub struct McpSession {
    servers: Vec<McpLiveServer>,
    handlers: Arc<McpHostHandlers>,
}

struct McpLiveServer {
    id: String,
    name: String,
    tools: Vec<McpListedTool>,
    resources: Vec<McpResource>,
    prompts: Vec<McpPrompt>,
    capabilities: Value,
    transport: McpTransport,
    next_id: i64,
}

#[derive(Debug, Clone)]
struct McpListedTool {
    name: String,
    description: String,
    input_schema: Value,
    /// MCP `annotations.readOnlyHint`，缺省按 false（需审批、串行）。
    read_only_hint: bool,
    /// MCP `annotations.destructiveHint`，缺省按 true（规范默认值）。
    destructive_hint: bool,
}

#[derive(Debug, Clone)]
struct McpResource {
    uri: String,
    name: String,
    description: String,
}

#[derive(Debug, Clone)]
struct McpPrompt {
    name: String,
    description: String,
    arguments: Vec<(String, String, bool)>,
}

enum McpTransport {
    Local {
        child: Box<Child>,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    },
    Ssh {
        stream: SshCommandStream,
        pending: Vec<u8>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
        session_id: Option<String>,
        bearer: Option<String>,
        /// 服务端在 POST 响应流里发来的、尚未处理的消息。
        backlog: Vec<Value>,
    },
    Sse {
        client: reqwest::Client,
        post_url: String,
        headers: Vec<(String, String)>,
        bearer: Option<String>,
        incoming: mpsc::Receiver<Value>,
    },
}

pub struct McpConnectResult {
    pub session: McpSession,
    pub warnings: Vec<String>,
    pub connected: Vec<String>,
}

#[derive(Clone, Default)]
pub struct SharedMcp {
    inner: Option<Arc<tokio::sync::Mutex<McpSession>>>,
}

impl SharedMcp {
    pub fn empty() -> Self {
        Self { inner: None }
    }

    pub fn from_session(session: McpSession) -> Self {
        Self {
            inner: Some(Arc::new(tokio::sync::Mutex::new(session))),
        }
    }

    pub async fn has_tool(&self, name: &str) -> bool {
        match &self.inner {
            Some(inner) => inner.lock().await.has_tool(name),
            None => false,
        }
    }

    pub async fn server_id_for_tool(&self, name: &str) -> Option<String> {
        match &self.inner {
            Some(inner) => inner.lock().await.server_id_for_tool(name),
            None => None,
        }
    }

    /// 按 `annotations` 生成的 MCP 工具契约；未连接或未知工具返回 `None`。
    pub async fn contract_for(&self, name: &str) -> Option<ToolContract> {
        match &self.inner {
            Some(inner) => inner.lock().await.contract_for(name),
            None => None,
        }
    }

    /// 所有已连接工具的契约，供调度器一次性注册。
    pub async fn tool_contracts(&self) -> Vec<ToolContract> {
        match &self.inner {
            Some(inner) => inner.lock().await.tool_contracts(),
            None => Vec::new(),
        }
    }

    pub async fn call(&self, name: &str, arguments: &str) -> Result<String, String> {
        match &self.inner {
            Some(inner) => inner.lock().await.call(name, arguments).await,
            None => Err(format!("unknown tool: {name}")),
        }
    }

    /// 会话层注入 elicitation / sampling / roots 的处理器。
    pub async fn set_host_handlers(&self, handlers: McpHostHandlers) {
        if let Some(inner) = &self.inner {
            inner.lock().await.handlers = Arc::new(handlers);
        }
    }

    pub async fn shutdown(&self) {
        if let Some(inner) = &self.inner {
            inner.lock().await.shutdown().await;
        }
    }
}

pub fn sanitize_mcp_token(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("mcp");
    }
    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

pub fn mcp_tool_name(server_id: &str, tool_name: &str) -> String {
    format!(
        "mcp_{}_{}",
        sanitize_mcp_token(server_id),
        sanitize_mcp_token(tool_name)
    )
}

fn resources_tool_name(server_id: &str) -> String {
    format!("mcp_{}_resources", sanitize_mcp_token(server_id))
}

fn prompt_tool_name(server_id: &str, prompt: &str) -> String {
    format!(
        "mcp_{}_prompt_{}",
        sanitize_mcp_token(server_id),
        sanitize_mcp_token(prompt)
    )
}

pub fn encode_rpc_message(value: &Value) -> Result<Vec<u8>, String> {
    let body =
        serde_json::to_vec(value).map_err(|error| format!("序列化 MCP 消息失败: {error}"))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut framed = header.into_bytes();
    framed.extend_from_slice(&body);
    Ok(framed)
}

pub fn remote_mcp_shell_command(server: &McpServerConfig) -> Result<String, String> {
    if server.command.trim().is_empty() {
        return Err("MCP 启动命令不能为空".to_string());
    }
    let mut parts = Vec::new();
    for env in &server.env {
        let key = env.key.trim();
        if key.is_empty() {
            continue;
        }
        if !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(format!("MCP 环境变量名不合法: {key}"));
        }
        parts.push(format!("{key}={}", shell_escape_single_quoted(&env.value)));
    }
    parts.push(shell_escape_single_quoted(server.command.trim()));
    for arg in &server.args {
        parts.push(shell_escape_single_quoted(arg));
    }
    Ok(format!("exec {}", parts.join(" ")))
}

impl McpSession {
    pub fn empty() -> Self {
        Self {
            servers: Vec::new(),
            handlers: Arc::new(McpHostHandlers::default()),
        }
    }

    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        let mut specs = Vec::new();
        for server in &self.servers {
            for tool in &server.tools {
                let description = if tool.description.trim().is_empty() {
                    format!("MCP tool {} from {}", tool.name, server.name)
                } else {
                    format!("[MCP:{}] {}", server.name, tool.description.trim())
                };
                specs.push(ToolSpec {
                    name: mcp_tool_name(&server.id, &tool.name),
                    description,
                    parameters: tool.input_schema.clone(),
                });
            }
            if !server.resources.is_empty() {
                let listing: Vec<String> = server
                    .resources
                    .iter()
                    .take(20)
                    .map(|resource| {
                        if resource.description.is_empty() {
                            format!("{} ({})", resource.name, resource.uri)
                        } else {
                            format!("{} ({}) — {}", resource.name, resource.uri, resource.description)
                        }
                    })
                    .collect();
                specs.push(ToolSpec {
                    name: resources_tool_name(&server.id),
                    description: format!(
                        "[MCP:{}] Read a resource by uri, or omit uri to list all {} resources. Known: {}",
                        server.name,
                        server.resources.len(),
                        listing.join("; ")
                    ),
                    parameters: json!({
                        "type": "object",
                        "properties": {"uri": {"type": "string"}}
                    }),
                });
            }
            for prompt in &server.prompts {
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();
                for (name, description, is_required) in &prompt.arguments {
                    properties.insert(
                        name.clone(),
                        json!({"type": "string", "description": description}),
                    );
                    if *is_required {
                        required.push(Value::String(name.clone()));
                    }
                }
                specs.push(ToolSpec {
                    name: prompt_tool_name(&server.id, &prompt.name),
                    description: format!(
                        "[MCP:{}] Prompt template `{}`{}. Returns the rendered prompt messages for you to follow.",
                        server.name,
                        prompt.name,
                        if prompt.description.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", prompt.description)
                        }
                    ),
                    parameters: json!({
                        "type": "object",
                        "properties": Value::Object(properties),
                        "required": required,
                    }),
                });
            }
        }
        specs
    }

    pub fn tool_contracts(&self) -> Vec<ToolContract> {
        let mut contracts = Vec::new();
        for server in &self.servers {
            for tool in &server.tools {
                contracts.push(ToolContract::for_mcp(
                    &mcp_tool_name(&server.id, &tool.name),
                    tool.read_only_hint,
                    tool.destructive_hint,
                ));
            }
            if !server.resources.is_empty() {
                contracts.push(ToolContract::for_mcp(
                    &resources_tool_name(&server.id),
                    true,
                    false,
                ));
            }
            for prompt in &server.prompts {
                contracts.push(ToolContract::for_mcp(
                    &prompt_tool_name(&server.id, &prompt.name),
                    true,
                    false,
                ));
            }
        }
        contracts
    }

    pub fn contract_for(&self, name: &str) -> Option<ToolContract> {
        self.tool_contracts()
            .into_iter()
            .find(|contract| contract.name == name)
    }

    fn locate(&self, name: &str) -> Option<(usize, McpCallKind)> {
        for (index, server) in self.servers.iter().enumerate() {
            if let Some(tool) = server
                .tools
                .iter()
                .find(|tool| mcp_tool_name(&server.id, &tool.name) == name)
            {
                return Some((index, McpCallKind::Tool(tool.name.clone())));
            }
            if !server.resources.is_empty() && resources_tool_name(&server.id) == name {
                return Some((index, McpCallKind::Resources));
            }
            if let Some(prompt) = server
                .prompts
                .iter()
                .find(|prompt| prompt_tool_name(&server.id, &prompt.name) == name)
            {
                return Some((index, McpCallKind::Prompt(prompt.name.clone())));
            }
        }
        None
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.locate(name).is_some()
    }

    pub fn server_id_for_tool(&self, name: &str) -> Option<String> {
        self.locate(name)
            .map(|(index, _)| self.servers[index].id.clone())
    }

    pub async fn call(&mut self, name: &str, arguments: &str) -> Result<String, String> {
        let (index, kind) = self
            .locate(name)
            .ok_or_else(|| format!("unknown tool: {name}"))?;
        let args: Value = if arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(arguments)
                .map_err(|error| format!("MCP 工具参数不是合法 JSON: {error}"))?
        };
        let handlers = self.handlers.clone();
        let server = &mut self.servers[index];
        match kind {
            McpCallKind::Tool(tool_name) => {
                let response = server
                    .request(
                        "tools/call",
                        json!({"name": tool_name, "arguments": args}),
                        CALL_TIMEOUT,
                        &handlers,
                    )
                    .await?;
                format_tool_result(&response)
            }
            McpCallKind::Resources => {
                match args.get("uri").and_then(Value::as_str).map(str::trim) {
                    Some(uri) if !uri.is_empty() => {
                        let response = server
                            .request(
                                "resources/read",
                                json!({"uri": uri}),
                                CALL_TIMEOUT,
                                &handlers,
                            )
                            .await?;
                        format_resource_read(&response)
                    }
                    _ => Ok(format_resource_list(&server.resources)),
                }
            }
            McpCallKind::Prompt(prompt_name) => {
                let response = server
                    .request(
                        "prompts/get",
                        json!({"name": prompt_name, "arguments": args}),
                        CALL_TIMEOUT,
                        &handlers,
                    )
                    .await?;
                format_prompt_result(&response)
            }
        }
    }

    pub async fn shutdown(&mut self) {
        for server in &mut self.servers {
            server.shutdown().await;
        }
        self.servers.clear();
    }
}

enum McpCallKind {
    Tool(String),
    Resources,
    Prompt(String),
}

fn format_resource_list(resources: &[McpResource]) -> String {
    resources
        .iter()
        .map(|resource| {
            format!(
                "- {} ({}){}",
                resource.name,
                resource.uri,
                if resource.description.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", resource.description)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_resource_read(response: &Value) -> Result<String, String> {
    if let Some(error) = response.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("resources/read 失败")
            .to_string());
    }
    let contents = response
        .pointer("/result/contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut parts = Vec::new();
    for item in contents {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            parts.push(text.to_string());
        } else if let Some(blob) = item.get("blob").and_then(Value::as_str) {
            parts.push(format!(
                "[binary {} bytes base64, mimeType={}]",
                blob.len(),
                item.get("mimeType").and_then(Value::as_str).unwrap_or("?")
            ));
        }
    }
    if parts.is_empty() {
        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(Value::Null)
            .to_string())
    } else {
        Ok(parts.join("\n"))
    }
}

fn format_prompt_result(response: &Value) -> Result<String, String> {
    if let Some(error) = response.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("prompts/get 失败")
            .to_string());
    }
    let messages = response
        .pointer("/result/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut parts = Vec::new();
    if let Some(description) = response
        .pointer("/result/description")
        .and_then(Value::as_str)
        .filter(|item| !item.trim().is_empty())
    {
        parts.push(format!("({description})"));
    }
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("user");
        let text = match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Object(map)) => map
                .get("text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| Value::Object(map.clone()).to_string()),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        parts.push(format!("[{role}] {text}"));
    }
    Ok(parts.join("\n\n"))
}

impl McpLiveServer {
    fn next_request_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    /// 发请求并等待同 id 的响应；中途收到的服务端请求 / 通知就地处理。
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        handlers: &McpHostHandlers,
    ) -> Result<Value, String> {
        let id = self.next_request_id();
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let expected = Value::from(id);
        tokio::time::timeout(timeout, self.exchange(request, &expected, handlers))
            .await
            .map_err(|_| format!("等待 MCP 响应超时（{method}）"))?
    }

    async fn exchange(
        &mut self,
        request: Value,
        expected: &Value,
        handlers: &McpHostHandlers,
    ) -> Result<Value, String> {
        self.send(&request).await?;
        loop {
            let message = self.receive().await?;
            match classify_message(&message) {
                Incoming::Response => {
                    if message.get("id") == Some(expected) {
                        return Ok(message);
                    }
                    // 迟到的其它响应：忽略。
                }
                Incoming::Notification(method) => {
                    if method == "notifications/message" {
                        if let Some(text) = message.pointer("/params/data") {
                            eprintln!("[MCP:{}] {}", self.name, text);
                        }
                    }
                }
                Incoming::Request(method) => {
                    let response = handle_server_request(&method, &message, handlers).await;
                    self.send(&response).await?;
                }
            }
        }
    }

    async fn notify(&mut self, notification: Value) -> Result<(), String> {
        self.send(&notification).await
    }

    async fn send(&mut self, message: &Value) -> Result<(), String> {
        match &mut self.transport {
            McpTransport::Local { stdin, .. } => {
                let framed = encode_rpc_message(message)?;
                stdin
                    .write_all(&framed)
                    .await
                    .map_err(|error| format!("写入 MCP 请求失败: {error}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|error| format!("刷新 MCP 请求失败: {error}"))
            }
            McpTransport::Ssh { stream, .. } => {
                let framed = encode_rpc_message(message)?;
                stream
                    .write_stdin(&framed)
                    .await
                    .map_err(|error| format!("写入 MCP 请求失败: {error}"))
            }
            McpTransport::Http {
                client,
                url,
                headers,
                session_id,
                bearer,
                backlog,
            } => {
                let mut request = client
                    .post(url.as_str())
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("mcp-protocol-version", MCP_PROTOCOL_VERSION);
                for (key, value) in headers.iter() {
                    request = request.header(key.as_str(), value.as_str());
                }
                if let Some(token) = bearer.as_deref() {
                    request = request.bearer_auth(token);
                }
                if let Some(session) = session_id.as_deref() {
                    request = request.header("mcp-session-id", session);
                }
                let response = request
                    .json(message)
                    .send()
                    .await
                    .map_err(|error| format!("MCP HTTP 请求失败: {error}"))?;
                if let Some(session) = response
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|value| value.to_str().ok())
                {
                    *session_id = Some(session.to_string());
                }
                let status = response.status();
                if status.as_u16() == 202 {
                    return Ok(());
                }
                if !status.is_success() {
                    let text = response.text().await.unwrap_or_default();
                    return Err(format!(
                        "MCP HTTP {}: {}",
                        status.as_u16(),
                        text.chars().take(300).collect::<String>()
                    ));
                }
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| format!("读取 MCP HTTP 响应失败: {error}"))?;
                if bytes.len() > MAX_HTTP_BODY {
                    return Err("MCP HTTP 响应过大".to_string());
                }
                let text = String::from_utf8_lossy(&bytes);
                if content_type.contains("text/event-stream") {
                    for value in parse_sse_json_messages(&text) {
                        backlog.push(value);
                    }
                } else if !text.trim().is_empty() {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(Value::Array(items)) => backlog.extend(items),
                        Ok(value) => backlog.push(value),
                        Err(error) => return Err(format!("解析 MCP HTTP 响应失败: {error}")),
                    }
                }
                Ok(())
            }
            McpTransport::Sse {
                client,
                post_url,
                headers,
                bearer,
                ..
            } => {
                let mut request = client
                    .post(post_url.as_str())
                    .header("content-type", "application/json");
                for (key, value) in headers.iter() {
                    request = request.header(key.as_str(), value.as_str());
                }
                if let Some(token) = bearer.as_deref() {
                    request = request.bearer_auth(token);
                }
                let response = request
                    .json(message)
                    .send()
                    .await
                    .map_err(|error| format!("MCP SSE POST 失败: {error}"))?;
                if !response.status().is_success() {
                    return Err(format!("MCP SSE POST HTTP {}", response.status().as_u16()));
                }
                Ok(())
            }
        }
    }

    async fn receive(&mut self) -> Result<Value, String> {
        match &mut self.transport {
            McpTransport::Local { stdout, .. } => read_rpc_message(stdout).await,
            McpTransport::Ssh { stream, pending } => read_rpc_from_ssh(stream, pending).await,
            McpTransport::Http { backlog, .. } => {
                if backlog.is_empty() {
                    // 服务端没有在 POST 响应里返回消息（例如只回了 202）。
                    return Err("MCP HTTP 响应为空".to_string());
                }
                Ok(backlog.remove(0))
            }
            McpTransport::Sse { incoming, .. } => incoming
                .recv()
                .await
                .ok_or_else(|| "MCP SSE 事件流已关闭".to_string()),
        }
    }

    async fn shutdown(&mut self) {
        match &mut self.transport {
            McpTransport::Local { child, stdin, .. } => {
                let _ = stdin.shutdown().await;
                terminate_local_child(child);
            }
            McpTransport::Ssh { stream, .. } => {
                let _ = stream.eof().await;
                let _ = stream.close().await;
            }
            McpTransport::Http {
                client,
                url,
                headers,
                session_id,
                bearer,
                ..
            } => {
                if let Some(session) = session_id.as_deref() {
                    let mut request = client
                        .delete(url.as_str())
                        .header("mcp-session-id", session);
                    for (key, value) in headers.iter() {
                        request = request.header(key.as_str(), value.as_str());
                    }
                    if let Some(token) = bearer.as_deref() {
                        request = request.bearer_auth(token);
                    }
                    let _ = request.send().await;
                }
            }
            McpTransport::Sse { incoming, .. } => {
                incoming.close();
            }
        }
    }
}

enum Incoming {
    Response,
    Notification(String),
    Request(String),
}

fn classify_message(message: &Value) -> Incoming {
    match (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) {
        (Some(method), Some(_)) => Incoming::Request(method.to_string()),
        (Some(method), None) => Incoming::Notification(method.to_string()),
        (None, _) => Incoming::Response,
    }
}

/// 处理服务端发来的请求，返回要回写的 JSON-RPC 响应。
async fn handle_server_request(
    method: &str,
    message: &Value,
    handlers: &McpHostHandlers,
) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let result: Result<Value, String> = match method {
        "ping" => Ok(json!({})),
        "roots/list" => Ok(json!({
            "roots": handlers
                .roots
                .iter()
                .map(|root| json!({"uri": root, "name": root.rsplit('/').next().unwrap_or(root)}))
                .collect::<Vec<_>>()
        })),
        "elicitation/create" => match &handlers.elicit {
            Some(elicit) => {
                let text = params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP 服务器需要你补充信息")
                    .to_string();
                let schema = params.get("requestedSchema").cloned().unwrap_or(json!({}));
                elicit(text, schema).await
            }
            None => Ok(json!({"action": "decline"})),
        },
        "sampling/createMessage" => match &handlers.sample {
            Some(sample) => sample(params).await,
            None => Err("当前会话不支持 sampling".to_string()),
        },
        _ => Err(format!("不支持的服务端请求: {method}")),
    };
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": error}
        }),
    }
}

/// 从 SSE 文本里取出所有 `data:` JSON 对象（忽略非 JSON 的心跳）。
pub fn parse_sse_json_messages(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    let flush = |data: &mut String, out: &mut Vec<Value>| {
        let trimmed = data.trim();
        if !trimmed.is_empty() {
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Array(items)) => out.extend(items),
                Ok(value) => out.push(value),
                Err(_) => {}
            }
        }
        data.clear();
    };
    for line in text.lines() {
        if line.is_empty() {
            flush(&mut data, &mut out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    flush(&mut data, &mut out);
    out
}

async fn read_rpc_from_ssh(
    stream: &mut SshCommandStream,
    pending: &mut Vec<u8>,
) -> Result<Value, String> {
    loop {
        if let Some(parsed) = take_rpc_message(pending) {
            return parsed;
        }
        match stream.next().await {
            Some(SshStreamEvent::Stdout(chunk)) => pending.extend_from_slice(&chunk),
            Some(SshStreamEvent::Stderr(chunk)) => {
                let warning = String::from_utf8_lossy(&chunk);
                if !warning.trim().is_empty() {
                    eprintln!("[MCP] 远程 stderr: {}", warning.trim());
                }
            }
            Some(SshStreamEvent::Exit(code)) => {
                return Err(format!("远程 MCP 已退出: {code}"));
            }
            Some(SshStreamEvent::Closed) | None => {
                return Err("远程 MCP 已断开".to_string());
            }
        }
    }
}

fn take_rpc_message(pending: &mut Vec<u8>) -> Option<Result<Value, String>> {
    if pending.is_empty() {
        return None;
    }
    if pending[0] == b'{' || pending[0] == b'[' {
        let newline = pending.iter().position(|&byte| byte == b'\n')?;
        let mut line: Vec<u8> = pending.drain(..=newline).collect();
        while matches!(line.last().copied(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        return Some(
            serde_json::from_slice(&line).map_err(|error| format!("解析 MCP JSON 失败: {error}")),
        );
    }

    let header_end = pending
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            pending
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })?;
    if header_end > 8_192 {
        pending.clear();
        return Some(Err("MCP 响应头过长".to_string()));
    }
    let header_text = String::from_utf8_lossy(&pending[..header_end]);
    let length = match header_text.lines().find_map(|line| {
        let line = line.trim();
        line.split_once(':').and_then(|(key, value)| {
            if key.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
    }) {
        Some(length) => length,
        None => {
            pending.drain(..header_end);
            return Some(Err("MCP 响应缺少 Content-Length".to_string()));
        }
    };
    if length > 8 * 1024 * 1024 {
        pending.clear();
        return Some(Err("MCP 响应过大".to_string()));
    }
    if pending.len() < header_end + length {
        return None;
    }
    let body = pending[header_end..header_end + length].to_vec();
    pending.drain(..header_end + length);
    Some(serde_json::from_slice(&body).map_err(|error| format!("解析 MCP JSON 失败: {error}")))
}

async fn read_rpc_message(stdout: &mut BufReader<ChildStdout>) -> Result<Value, String> {
    let mut first = [0u8; 1];
    stdout
        .read_exact(&mut first)
        .await
        .map_err(|error| format!("读取 MCP 响应失败: {error}"))?;
    if first[0] == b'{' || first[0] == b'[' {
        let mut line = vec![first[0]];
        stdout
            .read_until(b'\n', &mut line)
            .await
            .map_err(|error| format!("读取 MCP 行协议失败: {error}"))?;
        while matches!(line.last().copied(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        return serde_json::from_slice(&line)
            .map_err(|error| format!("解析 MCP JSON 失败: {error}"));
    }

    let mut header = vec![first[0]];
    loop {
        let mut byte = [0u8; 1];
        stdout
            .read_exact(&mut byte)
            .await
            .map_err(|error| format!("读取 MCP 头失败: {error}"))?;
        header.push(byte[0]);
        if header.windows(4).any(|w| w == b"\r\n\r\n") || header.windows(2).any(|w| w == b"\n\n") {
            break;
        }
        if header.len() > 8_192 {
            return Err("MCP 响应头过长".to_string());
        }
    }
    let header_text = String::from_utf8_lossy(&header);
    let length = header_text
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.split_once(':').and_then(|(key, value)| {
                if key.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| "MCP 响应缺少 Content-Length".to_string())?;
    if length > 8 * 1024 * 1024 {
        return Err("MCP 响应过大".to_string());
    }
    let mut body = vec![0u8; length];
    stdout
        .read_exact(&mut body)
        .await
        .map_err(|error| format!("读取 MCP 响应体失败: {error}"))?;
    serde_json::from_slice(&body).map_err(|error| format!("解析 MCP JSON 失败: {error}"))
}

fn format_tool_result(response: &Value) -> Result<String, String> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("MCP 工具调用失败");
        return Err(message.to_string());
    }
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let mut parts = Vec::new();
        for item in content {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            } else {
                parts.push(item.to_string());
            }
        }
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(if parts.is_empty() {
                "MCP 工具返回错误".to_string()
            } else {
                parts.join("\n")
            });
        }
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Ok(result.to_string())
}

fn terminate_local_child(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            unsafe {
                let _ = libc::killpg(pid as i32, libc::SIGTERM);
            }
        }
    }
    let _ = child.start_kill();
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

fn parse_listed_tools(listed: &Value) -> Vec<McpListedTool> {
    listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let annotations = item.get("annotations");
            let read_only_hint = annotations
                .and_then(|value| value.get("readOnlyHint"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let destructive_hint = annotations
                .and_then(|value| value.get("destructiveHint"))
                .and_then(Value::as_bool)
                .unwrap_or(!read_only_hint);
            Some(McpListedTool {
                name,
                description: item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                input_schema: item
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                read_only_hint,
                destructive_hint,
            })
        })
        .collect()
}

fn rpc_error_message(response: &Value, fallback: &str) -> Option<String> {
    response.get("error").map(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_string()
    })
}

async fn handshake(server: &mut McpLiveServer, handlers: &McpHostHandlers) -> Result<(), String> {
    let init = server
        .request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "roots": {"listChanged": false},
                    "elicitation": {},
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "noxcode",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
            HANDSHAKE_TIMEOUT,
            handlers,
        )
        .await?;
    if let Some(message) = rpc_error_message(&init, "initialize 失败") {
        return Err(message);
    }
    server.capabilities = init
        .pointer("/result/capabilities")
        .cloned()
        .unwrap_or(json!({}));
    server
        .notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }))
        .await?;
    let listed = server
        .request("tools/list", json!({}), HANDSHAKE_TIMEOUT, handlers)
        .await?;
    if let Some(message) = rpc_error_message(&listed, "tools/list 失败") {
        return Err(message);
    }
    server.tools = parse_listed_tools(&listed);
    if server.capabilities.get("resources").is_some() {
        if let Ok(listed) = server
            .request("resources/list", json!({}), HANDSHAKE_TIMEOUT, handlers)
            .await
        {
            server.resources = listed
                .pointer("/result/resources")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| {
                    let uri = item.get("uri")?.as_str()?.to_string();
                    Some(McpResource {
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(&uri)
                            .to_string(),
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        uri,
                    })
                })
                .collect();
        }
    }
    if server.capabilities.get("prompts").is_some() {
        if let Ok(listed) = server
            .request("prompts/list", json!({}), HANDSHAKE_TIMEOUT, handlers)
            .await
        {
            server.prompts = listed
                .pointer("/result/prompts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|argument| {
                                    Some((
                                        argument.get("name")?.as_str()?.to_string(),
                                        argument
                                            .get("description")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        argument
                                            .get("required")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(false),
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(McpPrompt {
                        name,
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        arguments,
                    })
                })
                .collect();
        }
    }
    Ok(())
}

fn blank_server(server: &McpServerConfig, transport: McpTransport) -> McpLiveServer {
    McpLiveServer {
        id: server.id.clone(),
        name: server.name.clone(),
        tools: Vec::new(),
        resources: Vec::new(),
        prompts: Vec::new(),
        capabilities: json!({}),
        transport,
        next_id: 0,
    }
}

async fn spawn_local(
    server: &McpServerConfig,
    extra_env: &[(String, String)],
) -> Result<McpLiveServer, String> {
    if server.command.trim().is_empty() {
        return Err("MCP 启动命令不能为空".to_string());
    }
    let mut command = Command::new(server.command.trim());
    command.args(&server.args);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut seen = HashMap::new();
    for env in &server.env {
        let key = env.key.trim();
        if key.is_empty() || seen.contains_key(key) {
            continue;
        }
        seen.insert(key.to_string(), ());
        command.env(key, &env.value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_tokio_command(&mut command);
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 MCP 失败: {error}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "MCP stdin 不可用".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "MCP stdout 不可用".to_string())?;
    Ok(blank_server(
        server,
        McpTransport::Local {
            child: Box::new(child),
            stdin,
            stdout: BufReader::new(stdout),
        },
    ))
}

async fn spawn_remote<R: Runtime>(
    app: &AppHandle<R>,
    ssh_config: &SshConfigRecord,
    server: &McpServerConfig,
) -> Result<McpLiveServer, String> {
    let remote = remote_mcp_shell_command(server)?;
    let stream = spawn_ssh_command(app, ssh_config, &remote, true).await?;
    Ok(blank_server(
        server,
        McpTransport::Ssh {
            stream,
            pending: Vec::new(),
        },
    ))
}

fn header_pairs(server: &McpServerConfig) -> Vec<(String, String)> {
    server
        .headers
        .iter()
        .filter(|header| !header.key.trim().is_empty())
        .map(|header| (header.key.trim().to_string(), header.value.clone()))
        .collect()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))
}

async fn connect_http(server: &McpServerConfig, bearer: Option<String>) -> Result<McpLiveServer, String> {
    let url = server
        .url
        .clone()
        .ok_or_else(|| "http 传输缺少 url".to_string())?;
    Ok(blank_server(
        server,
        McpTransport::Http {
            client: http_client()?,
            url,
            headers: header_pairs(server),
            session_id: None,
            bearer,
            backlog: Vec::new(),
        },
    ))
}

/// 旧版 SSE：先 GET 事件流，等 `endpoint` 事件拿到 POST 地址，再把后续 `message` 事件
/// 送进通道。
async fn connect_sse(server: &McpServerConfig, bearer: Option<String>) -> Result<McpLiveServer, String> {
    let url = server
        .url
        .clone()
        .ok_or_else(|| "sse 传输缺少 url".to_string())?;
    let headers = header_pairs(server);
    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;
    let mut request = client
        .get(&url)
        .header("accept", "text/event-stream");
    for (key, value) in &headers {
        request = request.header(key.as_str(), value.as_str());
    }
    if let Some(token) = bearer.as_deref() {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("MCP SSE 连接失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("MCP SSE HTTP {}", response.status().as_u16()));
    }
    let (tx, rx) = mpsc::channel::<Value>(64);
    let (endpoint_tx, endpoint_rx) = tokio::sync::oneshot::channel::<String>();
    let base_url = url.clone();
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut endpoint_tx = Some(endpoint_tx);
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find("\n\n") {
                let block = buffer[..index].to_string();
                buffer.drain(..index + 2);
                let mut event = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(rest) = line.strip_prefix("event:") {
                        event = rest.trim().to_string();
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(rest.trim_start());
                    }
                }
                if event == "endpoint" {
                    let post_url = if data.starts_with("http") {
                        data.clone()
                    } else {
                        match reqwest::Url::parse(&base_url).and_then(|base| base.join(data.trim())) {
                            Ok(url) => url.to_string(),
                            Err(_) => data.clone(),
                        }
                    };
                    if let Some(tx) = endpoint_tx.take() {
                        let _ = tx.send(post_url);
                    }
                } else if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
                    if tx.send(value).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let post_url = tokio::time::timeout(HANDSHAKE_TIMEOUT, endpoint_rx)
        .await
        .map_err(|_| "等待 SSE endpoint 事件超时".to_string())?
        .map_err(|_| "SSE 事件流在给出 endpoint 前已关闭".to_string())?;
    Ok(blank_server(
        server,
        McpTransport::Sse {
            client,
            post_url,
            headers,
            bearer,
            incoming: rx,
        },
    ))
}

pub async fn connect_mcp_servers<R: Runtime>(
    app: &AppHandle<R>,
    servers: &[McpServerConfig],
    ssh_config: Option<&SshConfigRecord>,
    cancel: &CancelFlag,
) -> McpConnectResult {
    let mut session = McpSession::empty();
    let mut warnings = Vec::new();
    let mut connected = Vec::new();
    let local_extra_env = if ssh_config.is_none() {
        match load_network_settings(app) {
            Ok(settings) => proxy_env_vars(&settings),
            Err(error) => {
                warnings.push(format!("[MCP] 读取网络设置失败：{error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let handlers = session.handlers.clone();
    for server in servers {
        if cancel.is_cancelled() {
            warnings.push("[MCP] 已取消，剩余服务器未连接".to_string());
            break;
        }
        let spawned = match server.transport.as_str() {
            MCP_TRANSPORT_HTTP | MCP_TRANSPORT_SSE => {
                let bearer = match crate::native::mcp_oauth::bearer_token_for(app, server).await {
                    Ok(token) => token,
                    Err(error) => {
                        warnings.push(format!("[MCP] {} 的 OAuth 令牌不可用：{error}", server.name));
                        None
                    }
                };
                if server.transport == MCP_TRANSPORT_HTTP {
                    connect_http(server, bearer).await
                } else {
                    connect_sse(server, bearer).await
                }
            }
            _ => {
                if let Some(ssh_config) = ssh_config {
                    spawn_remote(app, ssh_config, server).await
                } else {
                    spawn_local(server, &local_extra_env).await
                }
            }
        };
        let mut live = match spawned {
            Ok(live) => live,
            Err(error) => {
                warnings.push(format!(
                    "[MCP] 无法连接 {}：{}（已跳过，不回退到其他位置）",
                    server.name, error
                ));
                continue;
            }
        };
        match handshake(&mut live, &handlers).await {
            Ok(()) => {
                let extras = live.resources.len() + live.prompts.len();
                connected.push(if extras > 0 {
                    format!(
                        "{}（{} 个工具，{} 个资源，{} 个提示模板）",
                        live.name,
                        live.tools.len(),
                        live.resources.len(),
                        live.prompts.len()
                    )
                } else {
                    format!("{}（{} 个工具）", live.name, live.tools.len())
                });
                session.servers.push(live);
            }
            Err(error) => {
                live.shutdown().await;
                warnings.push(format!(
                    "[MCP] 握手失败 {}：{}（已跳过）",
                    server.name, error
                ));
            }
        }
    }
    McpConnectResult {
        session,
        warnings,
        connected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::McpEnvVar;

    fn sample_server() -> McpServerConfig {
        McpServerConfig {
            id: "fs.tools".to_string(),
            name: "Filesystem".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "pkg".to_string()],
            env: vec![McpEnvVar {
                key: "TOKEN".to_string(),
                value: "a b".to_string(),
            }],
            enabled: true,
            notes: None,
            scope: "all".to_string(),
            workspace_ids: Vec::new(),
            transport: "stdio".to_string(),
            url: None,
            headers: Vec::new(),
            oauth: None,
        }
    }

    #[test]
    fn tool_names_are_stable_and_sanitized() {
        assert_eq!(
            mcp_tool_name("fs.tools", "list-files"),
            "mcp_fs_tools_list_files"
        );
        assert_eq!(sanitize_mcp_token("123ab"), "_123ab");
        assert_eq!(resources_tool_name("fs.tools"), "mcp_fs_tools_resources");
        assert_eq!(
            prompt_tool_name("fs.tools", "review-pr"),
            "mcp_fs_tools_prompt_review_pr"
        );
    }

    #[test]
    fn remote_command_uses_exec_and_escapes() {
        let command = remote_mcp_shell_command(&sample_server()).expect("cmd");
        assert!(command.starts_with("exec "));
        assert!(command.contains("TOKEN='a b'"));
        assert!(command.contains("'npx'"));
        assert!(command.contains("'pkg'"));
        assert!(!command.contains("&&"));
    }

    #[test]
    fn encode_rpc_uses_content_length() {
        let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize"});
        let framed = encode_rpc_message(&body).expect("frame");
        let text = String::from_utf8(framed).expect("utf8");
        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n{"));
        assert!(text.contains("initialize"));
    }

    #[test]
    fn format_tool_result_joins_text_blocks() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [
                    {"type": "text", "text": "hello"},
                    {"type": "text", "text": "world"}
                ]
            }
        });
        assert_eq!(format_tool_result(&response).unwrap(), "hello\nworld");
    }

    #[test]
    fn format_tool_result_surfaces_is_error() {
        let response = json!({
            "result": {
                "isError": true,
                "content": [{"type": "text", "text": "boom"}]
            }
        });
        assert_eq!(format_tool_result(&response).unwrap_err(), "boom");
    }

    #[test]
    fn take_rpc_message_reads_content_length_and_json_line() {
        let framed = encode_rpc_message(&json!({"jsonrpc":"2.0","id":1})).expect("frame");
        let mut pending = framed;
        let value = take_rpc_message(&mut pending)
            .expect("ready")
            .expect("json");
        assert_eq!(value["id"], 1);
        assert!(pending.is_empty());

        let mut line = br#"{"ok":true}
"#
        .to_vec();
        let value = take_rpc_message(&mut line).expect("ready").expect("json");
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn sse_messages_and_server_request_classification() {
        let text = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n: keepalive\n\ndata: [{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":9}]\n\n";
        let messages = parse_sse_json_messages(text);
        assert_eq!(messages.len(), 2);
        assert!(matches!(classify_message(&messages[0]), Incoming::Response));
        assert!(matches!(
            classify_message(&messages[1]),
            Incoming::Request(ref method) if method == "ping"
        ));
        assert!(matches!(
            classify_message(&json!({"jsonrpc":"2.0","method":"notifications/message"})),
            Incoming::Notification(_)
        ));
    }

    #[tokio::test]
    async fn server_requests_are_answered_by_host_handlers() {
        let handlers = McpHostHandlers {
            elicit: Some(Arc::new(|message, _schema| {
                Box::pin(async move {
                    Ok(json!({"action": "accept", "content": {"answer": message}}))
                })
            })),
            sample: None,
            roots: vec!["file:///repo".to_string()],
        };
        let ping = handle_server_request("ping", &json!({"id": 1, "method": "ping"}), &handlers).await;
        assert_eq!(ping["result"], json!({}));
        let roots =
            handle_server_request("roots/list", &json!({"id": 2, "method": "roots/list"}), &handlers)
                .await;
        assert_eq!(roots["result"]["roots"][0]["uri"], "file:///repo");
        let elicited = handle_server_request(
            "elicitation/create",
            &json!({"id": 3, "method": "elicitation/create", "params": {"message": "选一个"}}),
            &handlers,
        )
        .await;
        assert_eq!(elicited["result"]["content"]["answer"], "选一个");
        let sampled = handle_server_request(
            "sampling/createMessage",
            &json!({"id": 4, "method": "sampling/createMessage", "params": {}}),
            &handlers,
        )
        .await;
        assert!(sampled["error"]["message"]
            .as_str()
            .unwrap()
            .contains("sampling"));
    }

    #[test]
    fn prompt_and_resource_results_are_rendered() {
        let prompt = json!({"result": {"description": "审查", "messages": [
            {"role": "user", "content": {"type": "text", "text": "请审查这个 diff"}},
            {"role": "assistant", "content": [{"type": "text", "text": "好的"}]}
        ]}});
        let rendered = format_prompt_result(&prompt).unwrap();
        assert!(rendered.contains("(审查)"));
        assert!(rendered.contains("[user] 请审查这个 diff"));
        assert!(rendered.contains("[assistant] 好的"));
        let resource = json!({"result": {"contents": [
            {"uri": "file:///a", "text": "hello"},
            {"uri": "file:///b", "blob": "QUJD", "mimeType": "image/png"}
        ]}});
        let text = format_resource_read(&resource).unwrap();
        assert!(text.contains("hello"));
        assert!(text.contains("image/png"));
        let listed = format_resource_list(&[McpResource {
            uri: "file:///a".to_string(),
            name: "A".to_string(),
            description: "first".to_string(),
        }]);
        assert_eq!(listed, "- A (file:///a) — first");
    }
}
