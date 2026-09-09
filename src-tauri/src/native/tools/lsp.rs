//! 本地 Language Server 客户端：按文件扩展名懒启动，提供导航与诊断。
//! SSH 工作区不启动 language server。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};

use crate::native::tools::command_path::resolve_program;
use crate::process_spawn::tokio_command;

use super::paths::resolve_under_workspace;

#[derive(Clone)]
pub struct LspHub {
    root: PathBuf,
    enabled: bool,
    inner: Arc<Mutex<HubState>>,
}

#[derive(Default)]
struct HubState {
    servers: HashMap<String, Arc<LanguageServer>>,
}

struct LanguageServer {
    language: String,
    command: String,
    next_id: AtomicI64,
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    diagnostics: Mutex<HashMap<String, Vec<LspDiagnostic>>>,
    opened: Mutex<HashMap<String, i64>>,
    _child: Child,
}

#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub path: String,
    pub line: u64,
    pub character: u64,
    pub severity: String,
    pub message: String,
}

impl LspHub {
    pub fn new(root: PathBuf, enabled: bool) -> Self {
        Self {
            root,
            enabled,
            inner: Arc::new(Mutex::new(HubState::default())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub async fn shutdown(&self) {
        let mut state = self.inner.lock().await;
        state.servers.clear();
    }

    pub async fn query(&self, arguments: &str) -> Result<String, String> {
        if !self.enabled {
            return Err("LSP 已在设置中关闭".to_string());
        }
        let args: Value = if arguments.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(arguments)
                .map_err(|error| format!("工具参数不是合法 JSON: {error}"))?
        };
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("diagnostics");
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("");
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let line = args.get("line").and_then(Value::as_u64).unwrap_or(1);
        let character = args.get("character").and_then(Value::as_u64).unwrap_or(1);
        if operation == "workspaceSymbol" {
            let language = infer_language_from_query(query).unwrap_or("rust");
            let server = self.ensure_server(language).await?;
            let result = server
                .request("workspace/symbol", json!({ "query": query }))
                .await?;
            return Ok(format_lsp_json("workspaceSymbol", &result));
        }
        if file_path.is_empty() {
            return Err("file_path 不能为空".to_string());
        }
        let resolved = resolve_under_workspace(&self.root, file_path)?;
        let language = language_for_path(&resolved)
            .ok_or_else(|| format!("不支持的文件类型: {}", resolved.display()))?;
        let server = self.ensure_server(language).await?;
        server.did_open(&resolved).await?;
        let uri = path_uri(&resolved);
        let pos = json!({
            "line": line.saturating_sub(1),
            "character": character.saturating_sub(1)
        });
        let result = match operation {
            "goToDefinition" => {
                server
                    .request(
                        "textDocument/definition",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "findReferences" => {
                server
                    .request(
                        "textDocument/references",
                        json!({
                            "textDocument": { "uri": uri },
                            "position": pos,
                            "context": { "includeDeclaration": true }
                        }),
                    )
                    .await?
            }
            "hover" => {
                server
                    .request(
                        "textDocument/hover",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "documentSymbol" => {
                server
                    .request(
                        "textDocument/documentSymbol",
                        json!({ "textDocument": { "uri": uri } }),
                    )
                    .await?
            }
            "goToImplementation" => {
                server
                    .request(
                        "textDocument/implementation",
                        json!({ "textDocument": { "uri": uri }, "position": pos }),
                    )
                    .await?
            }
            "diagnostics" => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                return Ok(format_diagnostics(&server.diagnostics_for(&resolved).await));
            }
            other => return Err(format!("未知 LSP operation: {other}")),
        };
        Ok(format_lsp_json(operation, &result))
    }

    pub async fn diagnostics_for_paths(&self, paths: &[String]) -> String {
        if !self.enabled || paths.is_empty() {
            return String::new();
        }
        let mut collected = Vec::new();
        for path in paths {
            let Ok(resolved) = resolve_under_workspace(&self.root, path) else {
                continue;
            };
            let Some(language) = language_for_path(&resolved) else {
                continue;
            };
            let Ok(server) = self.ensure_server(language).await else {
                continue;
            };
            if server.did_open(&resolved).await.is_err() {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            collected.extend(server.diagnostics_for(&resolved).await);
        }
        if collected.is_empty() {
            String::new()
        } else {
            format!("\n\n[LSP 诊断]\n{}", format_diagnostics(&collected))
        }
    }

    async fn ensure_server(&self, language: &str) -> Result<Arc<LanguageServer>, String> {
        let mut state = self.inner.lock().await;
        if let Some(server) = state.servers.get(language) {
            return Ok(server.clone());
        }
        let spec = server_spec(language)
            .ok_or_else(|| format!("没有为 {language} 配置 language server"))?;
        let server = LanguageServer::start(&self.root, language, spec).await?;
        state.servers.insert(language.to_string(), server.clone());
        Ok(server)
    }
}

struct ServerSpec {
    command: &'static str,
    args: &'static [&'static str],
}

fn server_spec(language: &str) -> Option<ServerSpec> {
    match language {
        "rust" => Some(ServerSpec {
            command: "rust-analyzer",
            args: &[],
        }),
        "typescript" | "javascript" => Some(ServerSpec {
            command: "typescript-language-server",
            args: &["--stdio"],
        }),
        "python" => Some(ServerSpec {
            command: "pyright-langserver",
            args: &["--stdio"],
        }),
        "go" => Some(ServerSpec {
            command: "gopls",
            args: &[],
        }),
        "cpp" => Some(ServerSpec {
            command: "clangd",
            args: &[],
        }),
        _ => None,
    }
}

pub fn language_for_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" => Some("cpp"),
        _ => None,
    }
}

fn infer_language_from_query(query: &str) -> Option<&'static str> {
    if query.contains("::") || query.contains("fn ") {
        Some("rust")
    } else {
        None
    }
}

impl LanguageServer {
    async fn start(root: &Path, language: &str, spec: ServerSpec) -> Result<Arc<Self>, String> {
        let program = resolve_program(spec.command).map_err(|_| {
            format!(
                "找不到 {language} language server `{}`。请安装后重试，或在设置中关闭 LSP。",
                spec.command
            )
        })?;
        let mut cmd = tokio_command(&program);
        cmd.args(spec.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("启动 {} 失败: {error}", spec.command))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "language server stdin 不可用".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "language server stdout 不可用".to_string())?;
        let server = Arc::new(Self {
            language: language.to_string(),
            command: spec.command.to_string(),
            next_id: AtomicI64::new(1),
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            opened: Mutex::new(HashMap::new()),
            _child: child,
        });
        let reader_server = server.clone();
        tauri::async_runtime::spawn(async move {
            read_loop(stdout, reader_server).await;
        });
        let root_uri = path_uri(root);
        let init = server
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {
                            "hover": { "contentFormat": ["markdown", "plaintext"] },
                            "publishDiagnostics": {}
                        },
                        "workspace": { "symbol": {} }
                    }
                }),
            )
            .await;
        if let Err(error) = init {
            return Err(format!("{} 初始化失败: {error}", spec.command));
        }
        let _ = server.notify("initialized", json!({})).await;
        Ok(server)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        tokio::time::timeout(Duration::from_secs(12), rx)
            .await
            .map_err(|_| format!("{} 请求超时: {method}", self.command))?
            .map_err(|_| format!("{} 已关闭: {method}", self.command))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn write(&self, body: &Value) -> Result<(), String> {
        let bytes = encode_rpc(body);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| format!("写入 {} 失败: {error}", self.command))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("写入 {} 失败: {error}", self.command))
    }

    async fn did_open(&self, path: &Path) -> Result<(), String> {
        let uri = path_uri(path);
        let mut opened = self.opened.lock().await;
        if opened.contains_key(&uri) {
            return Ok(());
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        opened.insert(uri.clone(), 1);
        drop(opened);
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": self.language,
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await
    }

    async fn diagnostics_for(&self, path: &Path) -> Vec<LspDiagnostic> {
        let uri = path_uri(path);
        self.diagnostics
            .lock()
            .await
            .get(&uri)
            .cloned()
            .unwrap_or_default()
    }
}

async fn read_loop(stdout: tokio::process::ChildStdout, server: Arc<LanguageServer>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_rpc(&mut reader).await {
            Ok(Some(value)) => handle_rpc(&server, value).await,
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

async fn handle_rpc(server: &LanguageServer, value: Value) {
    if let Some(id) = value.get("id").and_then(Value::as_i64) {
        if let Some(result) = value.get("result").cloned() {
            if let Some(tx) = server.pending.lock().await.remove(&id) {
                let _ = tx.send(result);
            }
            return;
        }
        if let Some(error) = value.get("error") {
            if let Some(tx) = server.pending.lock().await.remove(&id) {
                let _ = tx.send(json!({ "error": error }));
            }
        }
        return;
    }
    if value.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
        if let Some(params) = value.get("params") {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let items = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let parsed = items
                .iter()
                .filter_map(|item| parse_diagnostic(&uri, item))
                .collect();
            server.diagnostics.lock().await.insert(uri, parsed);
        }
    }
}

fn parse_diagnostic(uri: &str, item: &Value) -> Option<LspDiagnostic> {
    let message = item.get("message")?.as_str()?.to_string();
    let severity = match item.get("severity").and_then(Value::as_u64).unwrap_or(1) {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    };
    let line = item
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    let character = item
        .pointer("/range/start/character")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    Some(LspDiagnostic {
        path: uri_to_path(uri),
        line,
        character,
        severity: severity.to_string(),
        message,
    })
}

pub fn encode_rpc(body: &Value) -> Vec<u8> {
    let json = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!("Content-Length: {}\r\n\r\n", json.len());
    let mut out = header.into_bytes();
    out.extend(json);
    out
}

async fn read_rpc<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Value>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut buf = vec![0_u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&buf)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn path_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.starts_with("file:") {
        return raw.into_owned();
    }
    format!("file://{raw}")
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

fn format_diagnostics(items: &[LspDiagnostic]) -> String {
    if items.is_empty() {
        return "没有诊断。".to_string();
    }
    items
        .iter()
        .take(40)
        .map(|item| {
            format!(
                "{}:{}:{} [{}] {}",
                item.path, item.line, item.character, item.severity, item.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_lsp_json(operation: &str, value: &Value) -> String {
    format!(
        "{operation}: {}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    )
}

pub fn mutation_paths(name: &str, arguments: &str) -> Vec<String> {
    let Ok(args) = serde_json::from_str::<Value>(arguments) else {
        return Vec::new();
    };
    match name {
        "Write" | "Edit" => args
            .get("file_path")
            .and_then(Value::as_str)
            .map(|path| vec![path.to_string()])
            .unwrap_or_default(),
        "ApplyPatch" => super::patch::parse_patch(
            &super::patch::extract_patch_text(arguments).unwrap_or_default(),
        )
        .map(|actions| {
            actions
                .into_iter()
                .map(|action| match action {
                    super::patch::PatchAction::Add { path, .. }
                    | super::patch::PatchAction::Delete { path }
                    | super::patch::PatchAction::Update { path, .. } => path,
                })
                .collect()
        })
        .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_mapping() {
        assert_eq!(language_for_path(Path::new("src/lib.rs")), Some("rust"));
        assert_eq!(language_for_path(Path::new("app.tsx")), Some("typescript"));
        assert_eq!(language_for_path(Path::new("main.py")), Some("python"));
        assert_eq!(language_for_path(Path::new("readme.md")), None);
    }

    #[test]
    fn encode_rpc_has_content_length() {
        let bytes = encode_rpc(&json!({"jsonrpc":"2.0","id":1}));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("Content-Length:"));
        assert!(text.contains("\r\n\r\n{"));
    }

    #[test]
    fn mutation_paths_read_write_and_patch() {
        assert_eq!(
            mutation_paths("Write", r#"{"file_path":"a.rs","content":"x"}"#),
            vec!["a.rs"]
        );
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-a\n+b\n*** End Patch\n";
        let args = serde_json::json!({ "patch": patch }).to_string();
        assert!(mutation_paths("ApplyPatch", &args).contains(&"src/a.rs".to_string()));
    }
}
