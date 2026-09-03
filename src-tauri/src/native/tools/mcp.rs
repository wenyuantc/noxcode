use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::app::network_settings::{load_network_settings, proxy_env_vars};
use crate::app::ssh::exec::{spawn_ssh_command, SshCommandStream, SshStreamEvent};
use crate::app::ssh::shell::shell_escape_single_quoted;
use crate::db::models::{McpServerConfig, SshConfigRecord};
use crate::native::model::types::ToolSpec;
use crate::process_spawn::configure_tokio_command;

use super::cancel::CancelFlag;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct McpSession {
    servers: Vec<McpLiveServer>,
}

struct McpLiveServer {
    id: String,
    name: String,
    tools: Vec<McpListedTool>,
    transport: McpTransport,
    next_id: i64,
}

struct McpListedTool {
    name: String,
    description: String,
    input_schema: Value,
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

    pub async fn call(&self, name: &str, arguments: &str) -> Result<String, String> {
        match &self.inner {
            Some(inner) => inner.lock().await.call(name, arguments).await,
            None => Err(format!("unknown tool: {name}")),
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
        }
        specs
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.servers.iter().any(|server| {
            server
                .tools
                .iter()
                .any(|tool| mcp_tool_name(&server.id, &tool.name) == name)
        })
    }

    pub fn server_id_for_tool(&self, name: &str) -> Option<String> {
        self.servers.iter().find_map(|server| {
            server
                .tools
                .iter()
                .any(|tool| mcp_tool_name(&server.id, &tool.name) == name)
                .then(|| server.id.clone())
        })
    }

    pub async fn call(&mut self, name: &str, arguments: &str) -> Result<String, String> {
        let index = self
            .servers
            .iter()
            .position(|server| {
                server
                    .tools
                    .iter()
                    .any(|tool| mcp_tool_name(&server.id, &tool.name) == name)
            })
            .ok_or_else(|| format!("unknown tool: {name}"))?;
        let tool_name = self.servers[index]
            .tools
            .iter()
            .find(|tool| mcp_tool_name(&self.servers[index].id, &tool.name) == name)
            .map(|tool| tool.name.clone())
            .ok_or_else(|| format!("unknown tool: {name}"))?;
        let args: Value = if arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(arguments)
                .map_err(|error| format!("MCP 工具参数不是合法 JSON: {error}"))?
        };
        let request_id = {
            let server = &mut self.servers[index];
            server.next_id += 1;
            server.next_id
        };
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args,
            }
        });
        let response = self.servers[index].request(request, CALL_TIMEOUT).await?;
        format_tool_result(&response)
    }

    pub async fn shutdown(&mut self) {
        for server in &mut self.servers {
            server.shutdown().await;
        }
        self.servers.clear();
    }
}

impl McpLiveServer {
    async fn request(&mut self, request: Value, timeout: Duration) -> Result<Value, String> {
        let framed = encode_rpc_message(&request)?;
        let expected_id = request.get("id").cloned();
        match &mut self.transport {
            McpTransport::Local { stdin, stdout, .. } => {
                write_and_read(stdin, stdout, framed, expected_id, timeout).await
            }
            McpTransport::Ssh { stream, pending } => {
                write_and_read_ssh(stream, pending, framed, expected_id, timeout).await
            }
        }
    }

    async fn notify(&mut self, notification: Value) -> Result<(), String> {
        let framed = encode_rpc_message(&notification)?;
        match &mut self.transport {
            McpTransport::Local { stdin, .. } => stdin
                .write_all(&framed)
                .await
                .map_err(|error| format!("写入 MCP 通知失败: {error}")),
            McpTransport::Ssh { stream, .. } => stream
                .write_stdin(&framed)
                .await
                .map_err(|error| format!("写入 MCP 通知失败: {error}")),
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
        }
    }
}

async fn write_and_read_ssh(
    stream: &mut SshCommandStream,
    pending: &mut Vec<u8>,
    framed: Vec<u8>,
    expected_id: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    stream
        .write_stdin(&framed)
        .await
        .map_err(|error| format!("写入 MCP 请求失败: {error}"))?;
    tokio::time::timeout(timeout, read_rpc_from_ssh(stream, pending))
        .await
        .map_err(|_| "等待 MCP 响应超时".to_string())?
        .and_then(|value| match expected_id {
            Some(id) if value.get("id") != Some(&id) => Err("MCP 响应 id 不匹配".to_string()),
            _ => Ok(value),
        })
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

async fn write_and_read(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    framed: Vec<u8>,
    expected_id: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    stdin
        .write_all(&framed)
        .await
        .map_err(|error| format!("写入 MCP 请求失败: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("刷新 MCP 请求失败: {error}"))?;
    tokio::time::timeout(timeout, read_rpc_message(stdout))
        .await
        .map_err(|_| "等待 MCP 响应超时".to_string())?
        .and_then(|value| match expected_id {
            Some(id) if value.get("id") != Some(&id) => Err("MCP 响应 id 不匹配".to_string()),
            _ => Ok(value),
        })
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
                parts.join(
                    "
",
                )
            });
        }
        if !parts.is_empty() {
            return Ok(parts.join(
                "
",
            ));
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

async fn handshake(server: &mut McpLiveServer) -> Result<(), String> {
    let init_id = {
        server.next_id += 1;
        server.next_id
    };
    let init = json!({
        "jsonrpc": "2.0",
        "id": init_id,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "codex-ai",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }
    });
    let _ = server.request(init, HANDSHAKE_TIMEOUT).await?;
    server
        .notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }))
        .await?;
    let list_id = {
        server.next_id += 1;
        server.next_id
    };
    let listed = server
        .request(
            json!({
                "jsonrpc": "2.0",
                "id": list_id,
                "method": "tools/list",
                "params": {},
            }),
            HANDSHAKE_TIMEOUT,
        )
        .await?;
    if let Some(error) = listed.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("tools/list 失败");
        return Err(message.to_string());
    }
    let tools = listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    server.tools = tools
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
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
            })
        })
        .collect();
    Ok(())
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
    Ok(McpLiveServer {
        id: server.id.clone(),
        name: server.name.clone(),
        tools: Vec::new(),
        transport: McpTransport::Local {
            child: Box::new(child),
            stdin,
            stdout: BufReader::new(stdout),
        },
        next_id: 0,
    })
}

async fn spawn_remote<R: Runtime>(
    app: &AppHandle<R>,
    ssh_config: &SshConfigRecord,
    server: &McpServerConfig,
) -> Result<McpLiveServer, String> {
    let remote = remote_mcp_shell_command(server)?;
    let stream = spawn_ssh_command(app, ssh_config, &remote, true).await?;
    Ok(McpLiveServer {
        id: server.id.clone(),
        name: server.name.clone(),
        tools: Vec::new(),
        transport: McpTransport::Ssh {
            stream,
            pending: Vec::new(),
        },
        next_id: 0,
    })
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
    for server in servers {
        if cancel.is_cancelled() {
            warnings.push("[MCP] 已取消，剩余服务器未连接".to_string());
            break;
        }
        let spawned = if let Some(ssh_config) = ssh_config {
            spawn_remote(app, ssh_config, server).await
        } else {
            spawn_local(server, &local_extra_env).await
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
        match handshake(&mut live).await {
            Ok(()) => {
                connected.push(format!("{}（{} 个工具）", live.name, live.tools.len()));
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
        }
    }

    #[test]
    fn tool_names_are_stable_and_sanitized() {
        assert_eq!(
            mcp_tool_name("fs.tools", "list-files"),
            "mcp_fs_tools_list_files"
        );
        assert_eq!(sanitize_mcp_token("123ab"), "_123ab");
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
        assert_eq!(
            format_tool_result(&response).unwrap(),
            "hello
world"
        );
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
}
