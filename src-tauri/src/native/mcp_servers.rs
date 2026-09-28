use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tokio::process::Command;

use crate::app::shared::{new_id, sqlite_pool};
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::app::ssh::exec::{spawn_ssh_command, SshStreamEvent};
use crate::app::ssh::shell::{remote_shell_bootstrap, shell_escape_single_quoted};
use crate::db::models::{McpEnvVar, McpServerConfig, McpServersDocument, UpdateMcpServersPayload};
use crate::engine::context::resolve_workspace_execution_context_with_pool;
use crate::native::subagents::{SCOPE_ALL, SCOPE_WORKSPACES};
use crate::native::tools::command_path;
use crate::process_spawn::configure_tokio_command;

const MCP_SERVERS_FILE_NAME: &str = "mcp-servers.json";
const PLAYWRIGHT_PROBE_ARGS: &[&str] = &[
    "--offline",
    "--no-install",
    "@playwright/mcp@0.0.82",
    "--version",
];
const PLAYWRIGHT_INSTALL_ARGS: &[&str] = &[
    "exec",
    "--yes",
    "--ignore-scripts",
    "--package=@playwright/mcp@0.0.82",
    "--",
    "playwright-mcp",
    "--version",
];
const PLAYWRIGHT_BROWSER_INSTALL_ARGS: &[&str] = &[
    "exec",
    "--yes",
    "--ignore-scripts",
    "--package=@playwright/mcp@0.0.82",
    "--",
    "playwright",
    "install",
    "chromium",
];

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

fn mcp_servers_file_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(MCP_SERVERS_FILE_NAME))
}

pub fn default_mcp_servers() -> McpServersDocument {
    McpServersDocument {
        servers: vec![McpServerConfig {
            id: "example-filesystem".to_string(),
            name: "示例：Filesystem（请按需启用）".to_string(),
            command: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                "/tmp".to_string(),
            ],
            env: vec![],
            enabled: false,
            notes: Some(
                "这是占位示例。启用前请确认本机已安装 Node/npx，并按实际仓库路径修改 args。"
                    .to_string(),
            ),
            scope: SCOPE_ALL.to_string(),
            workspace_ids: Vec::new(),
            transport: "stdio".to_string(),
            url: None,
            headers: Vec::new(),
            oauth: None,
        }],
    }
}

fn normalize_server(mut server: McpServerConfig) -> Result<McpServerConfig, String> {
    server.name = server.name.trim().to_string();
    server.command = server.command.trim().to_string();
    if server.name.is_empty() {
        return Err("MCP 服务器名称不能为空".to_string());
    }
    server.transport = match server.transport.trim().to_ascii_lowercase().as_str() {
        crate::db::models::MCP_TRANSPORT_HTTP | "streamable-http" | "streamable_http" => {
            crate::db::models::MCP_TRANSPORT_HTTP.to_string()
        }
        crate::db::models::MCP_TRANSPORT_SSE => crate::db::models::MCP_TRANSPORT_SSE.to_string(),
        _ => crate::db::models::MCP_TRANSPORT_STDIO.to_string(),
    };
    server.url = server
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if server.transport == crate::db::models::MCP_TRANSPORT_STDIO {
        if server.command.is_empty() {
            return Err("MCP 启动命令不能为空".to_string());
        }
    } else {
        let url = server
            .url
            .as_deref()
            .ok_or_else(|| "http / sse 传输需要填写 url".to_string())?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("MCP url 必须是 http(s) 地址: {url}"));
        }
    }
    server.headers = server
        .headers
        .into_iter()
        .filter_map(|header| {
            let key = header.key.trim().to_string();
            (!key.is_empty()).then_some(McpEnvVar {
                key,
                value: header.value,
            })
        })
        .collect();
    if let Some(oauth) = server.oauth.as_mut() {
        oauth.client_id = oauth.client_id.trim().to_string();
        oauth.authorize_url = oauth.authorize_url.trim().to_string();
        oauth.token_url = oauth.token_url.trim().to_string();
        if oauth.client_id.is_empty()
            || oauth.authorize_url.is_empty()
            || oauth.token_url.is_empty()
        {
            return Err("OAuth 配置需要 client_id、authorize_url 与 token_url".to_string());
        }
    }
    if server.id.trim().is_empty() {
        server.id = new_id();
    }
    server.args = server
        .args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect();
    server.env = server
        .env
        .into_iter()
        .filter_map(|env| {
            let key = env.key.trim().to_string();
            if key.is_empty() {
                None
            } else {
                Some(McpEnvVar {
                    key,
                    value: env.value,
                })
            }
        })
        .collect();
    if let Some(notes) = server.notes.as_mut() {
        let trimmed = notes.trim();
        if trimmed.is_empty() {
            server.notes = None;
        } else {
            *notes = trimmed.to_string();
        }
    }
    server.scope = if server.scope.trim() == SCOPE_WORKSPACES {
        SCOPE_WORKSPACES.to_string()
    } else {
        SCOPE_ALL.to_string()
    };
    let mut workspace_ids = Vec::new();
    for workspace_id in server.workspace_ids {
        let workspace_id = workspace_id.trim();
        if !workspace_id.is_empty() && !workspace_ids.iter().any(|item| item == workspace_id) {
            workspace_ids.push(workspace_id.to_string());
        }
    }
    server.workspace_ids = if server.scope == SCOPE_WORKSPACES {
        workspace_ids
    } else {
        Vec::new()
    };
    Ok(server)
}

fn load_document_from_disk(path: &PathBuf) -> Result<McpServersDocument, String> {
    if !path.exists() {
        return Ok(default_mcp_servers());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("读取 MCP 配置失败 {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(default_mcp_servers());
    }
    serde_json::from_str(&raw).map_err(|error| format!("解析 MCP 配置失败: {error}"))
}

fn write_document(path: &PathBuf, document: &McpServersDocument) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败: {error}"))?;
    }
    let json = serde_json::to_string_pretty(document)
        .map_err(|error| format!("序列化 MCP 配置失败: {error}"))?;
    fs::write(path, json).map_err(|error| format!("写入 MCP 配置失败: {error}"))
}

pub fn load_mcp_document<R: Runtime>(app: &AppHandle<R>) -> Result<McpServersDocument, String> {
    let path = mcp_servers_file_path(app)?;
    load_document_from_disk(&path)
}

pub fn resolve_effective_mcp_servers<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
) -> Result<Vec<McpServerConfig>, String> {
    let document = load_mcp_document(app)?;
    Ok(document
        .servers
        .into_iter()
        .filter(|server| server.enabled && server_matches_workspace(server, workspace_id))
        .collect())
}

/// 项目级 MCP 文件：`.noxcode/mcp.json`（本项目形状 `{ servers: [...] }`）与 `.mcp.json`
/// （Claude Code 形状 `{ mcpServers: { name: {...} } }`）。
pub const PROJECT_MCP_FILES: &[&str] = &[".noxcode/mcp.json", ".mcp.json"];

/// 读取工作区根目录下的项目级 MCP 服务器；文件缺失或非法返回空。id 前缀 `project-`。
pub fn load_project_mcp_servers(workspace_root: &Path) -> Vec<McpServerConfig> {
    let mut out: Vec<McpServerConfig> = Vec::new();
    for file in PROJECT_MCP_FILES {
        let path = workspace_root.join(file);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("[MCP] 项目级配置不是合法 JSON，已跳过: {}", path.display());
            continue;
        };
        for mut server in crate::native::plugins::parse_plugin_mcp_servers(&value, "project") {
            // parse_plugin_mcp_servers 会加 `plugin-project-` 前缀与 `project/` 名称前缀，这里改回项目级标识。
            server.id = server
                .id
                .strip_prefix("plugin-project-")
                .map(|rest| format!("project-{rest}"))
                .unwrap_or(server.id);
            server.name = server
                .name
                .strip_prefix("project/")
                .map(str::to_string)
                .unwrap_or(server.name);
            server.notes = Some(format!("来自 {file}"));
            server.scope = SCOPE_ALL.to_string();
            server.workspace_ids.clear();
            if server.enabled
                && !out
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&server.name))
            {
                out.push(server);
            }
        }
    }
    out
}

/// 会话实际连接的服务器：全局（按工作区过滤）+ 已启用插件 + 项目级文件；
/// 同名以项目级为准，其次插件，最后全局。
pub fn resolve_session_mcp_servers<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> Result<Vec<McpServerConfig>, String> {
    let global = resolve_effective_mcp_servers(app, workspace_id)?;
    let config_dir = app_config_dir(app).ok();
    let plugins =
        crate::native::plugins::load_enabled_plugins(config_dir.as_deref(), workspace_root);
    let plugin_servers = crate::native::plugins::plugin_mcp_servers(&plugins);
    let project_servers = workspace_root
        .map(load_project_mcp_servers)
        .unwrap_or_default();
    Ok(merge_mcp_sources(global, plugin_servers, project_servers))
}

pub fn merge_mcp_sources(
    global: Vec<McpServerConfig>,
    plugin: Vec<McpServerConfig>,
    project: Vec<McpServerConfig>,
) -> Vec<McpServerConfig> {
    let mut out: Vec<McpServerConfig> = Vec::new();
    for server in project.into_iter().chain(plugin).chain(global) {
        if !server.enabled {
            continue;
        }
        if out
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&server.name))
        {
            continue;
        }
        out.push(server);
    }
    out
}

fn server_matches_workspace(server: &McpServerConfig, workspace_id: Option<&str>) -> bool {
    if server.scope != SCOPE_WORKSPACES {
        return true;
    }
    let Some(workspace_id) = workspace_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    server.workspace_ids.iter().any(|id| id == workspace_id)
}

#[tauri::command]
pub async fn get_mcp_servers<R: Runtime>(app: AppHandle<R>) -> Result<McpServersDocument, String> {
    load_mcp_document(&app)
}

#[tauri::command]
pub async fn update_mcp_servers<R: Runtime>(
    app: AppHandle<R>,
    payload: UpdateMcpServersPayload,
) -> Result<McpServersDocument, String> {
    let mut servers = Vec::with_capacity(payload.servers.len());
    for server in payload.servers {
        servers.push(normalize_server(server)?);
    }
    let document = McpServersDocument { servers };
    write_document(&mcp_servers_file_path(&app)?, &document)?;
    Ok(document)
}

#[tauri::command]
pub async fn reset_mcp_servers<R: Runtime>(
    app: AppHandle<R>,
) -> Result<McpServersDocument, String> {
    let document = default_mcp_servers();
    write_document(&mcp_servers_file_path(&app)?, &document)?;
    Ok(document)
}

fn json_quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn render_mcp_export_snippet(document: &McpServersDocument) -> String {
    let mut lines = vec![
        "# 可将以下 JSON 片段合并到 Claude Desktop / Cursor 等 MCP 配置中".to_string(),
        "# 仅导出 enabled = true 的服务器".to_string(),
        "{".to_string(),
        "  \"mcpServers\": {".to_string(),
    ];

    let enabled: Vec<_> = document
        .servers
        .iter()
        .filter(|server| server.enabled)
        .collect();
    for (index, server) in enabled.iter().enumerate() {
        let args = server
            .args
            .iter()
            .map(|arg| format!("\"{}\"", json_quoted(arg)))
            .collect::<Vec<_>>()
            .join(", ");
        let env_pairs = server
            .env
            .iter()
            .map(|env| {
                format!(
                    "\"{}\": \"{}\"",
                    json_quoted(&env.key),
                    json_quoted(&env.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "    \"{}\": {{\n      \"command\": \"{}\",\n      \"args\": [{}],\n      \"env\": {{{}}}\n    }}{}",
            json_quoted(&server.name),
            json_quoted(&server.command),
            args,
            env_pairs,
            if index + 1 == enabled.len() {
                ""
            } else {
                ","
            }
        ));
    }
    lines.push("  }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

#[tauri::command]
pub async fn export_mcp_servers_snippet<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let document = load_mcp_document(&app)?;
    Ok(render_mcp_export_snippet(&document))
}

fn saved_test_server(
    document: &McpServersDocument,
    server_id: &str,
    workspace_id: &str,
) -> Result<McpServerConfig, String> {
    let server = document
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| "MCP 服务器不存在，请先保存配置".to_string())?;
    if !server_matches_workspace(server, Some(workspace_id)) {
        return Err("此 MCP 服务器不适用于所选工作区".to_string());
    }
    Ok(server.clone())
}

/// 在所选工作区的实际执行主机握手，不运行前端提交的临时命令。
#[tauri::command]
pub async fn test_mcp_server<R: Runtime>(
    app: AppHandle<R>,
    server_id: String,
    workspace_id: String,
) -> Result<String, String> {
    let pool = sqlite_pool(&app).await?;
    let context = resolve_workspace_execution_context_with_pool(&pool, &workspace_id).await?;
    let server = saved_test_server(&load_mcp_document(&app)?, &server_id, &workspace_id)?;
    let ssh_config = match context.ssh_config_id.as_deref() {
        Some(id) => Some(fetch_ssh_config_record_by_id(&pool, id).await?),
        None => None,
    };
    let summary =
        crate::native::tools::mcp::test_mcp_server_connection(&app, &server, ssh_config.as_ref())
            .await?;
    let host = if server.transport == "stdio" {
        context.target_host_label.as_deref().unwrap_or("本机")
    } else {
        server.url.as_deref().unwrap_or("HTTP MCP 端点")
    };
    Ok(format!("{host}：{summary}"))
}

async fn workspace_ssh_host<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: &str,
) -> Result<(Option<crate::db::models::SshConfigRecord>, String), String> {
    let pool = sqlite_pool(app).await?;
    let context = resolve_workspace_execution_context_with_pool(&pool, workspace_id).await?;
    let ssh = match context.ssh_config_id.as_deref() {
        Some(id) => Some(fetch_ssh_config_record_by_id(&pool, id).await?),
        None => None,
    };
    Ok((
        ssh,
        context
            .target_host_label
            .unwrap_or_else(|| "本机".to_string()),
    ))
}

async fn run_on_host<R: Runtime>(
    app: &AppHandle<R>,
    ssh: Option<&crate::db::models::SshConfigRecord>,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let (ok, stdout, stderr) = if let Some(ssh) = ssh {
        let command = std::iter::once(program)
            .chain(args.iter().copied())
            .map(shell_escape_single_quoted)
            .collect::<Vec<_>>()
            .join(" ");
        let remote = format!("{}exec {command}", remote_shell_bootstrap());
        let mut stream = spawn_ssh_command(app, ssh, &remote, true).await?;
        let result = tokio::time::timeout(timeout, async {
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let code = loop {
                match stream.next().await {
                    Some(SshStreamEvent::Stdout(bytes)) => stdout.extend_from_slice(&bytes),
                    Some(SshStreamEvent::Stderr(bytes)) => stderr.extend_from_slice(&bytes),
                    Some(SshStreamEvent::Exit(code)) => break code,
                    Some(SshStreamEvent::Closed) | None => break -1,
                }
                if stdout.len() + stderr.len() > 1024 * 1024 {
                    break -1;
                }
            };
            (code == 0, stdout, stderr)
        })
        .await;
        match result {
            Ok((true, stdout, stderr)) => {
                let _ = stream.close().await;
                (true, stdout, stderr)
            }
            Ok((false, stdout, stderr)) => {
                stream.terminate().await;
                (false, stdout, stderr)
            }
            Err(_) => {
                stream.terminate().await;
                return Err("远端命令超时".to_string());
            }
        }
    } else {
        let path = command_path::resolve_program(program)?;
        let mut command = Command::new(path);
        command.args(args).kill_on_drop(true);
        command_path::apply_augmented_path(&mut command);
        configure_tokio_command(&mut command);
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| "本机命令超时".to_string())?
            .map_err(|error| format!("运行 {program} 失败: {error}"))?;
        (output.status.success(), output.stdout, output.stderr)
    };
    let text = if stdout.is_empty() { &stderr } else { &stdout };
    let summary = String::from_utf8_lossy(text)
        .chars()
        .take(600)
        .collect::<String>();
    if ok {
        Ok(summary.trim().to_string())
    } else {
        Err(if summary.trim().is_empty() {
            format!("{program} 执行失败")
        } else {
            summary.trim().to_string()
        })
    }
}

#[tauri::command]
pub async fn diagnose_playwright<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<String, String> {
    let (ssh, host) = workspace_ssh_host(&app, &workspace_id).await?;
    let node = run_on_host(
        &app,
        ssh.as_ref(),
        "node",
        &["--version"],
        Duration::from_secs(10),
    )
    .await;
    let Ok(node) = node else {
        return Ok(format!(
            "{host}：未检测到 Node，请先在此主机安装 Node 18 或更新版本"
        ));
    };
    let major = node
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|item| item.parse::<u32>().ok())
        .unwrap_or(0);
    if major < 18 {
        return Ok(format!(
            "{host}：Node {node} 过旧，Playwright MCP 需要 Node 18+"
        ));
    }
    let package = run_on_host(
        &app,
        ssh.as_ref(),
        "npx",
        PLAYWRIGHT_PROBE_ARGS,
        Duration::from_secs(20),
    )
    .await;
    Ok(match package {
        Ok(version) => {
            format!("{host}：Node {node}，Playwright MCP {version}；浏览器请用连接测试验证")
        }
        Err(_) => format!("{host}：Node {node}，未检测到固定版本的 Playwright MCP；可按需安装"),
    })
}

#[tauri::command]
pub async fn install_playwright_mcp<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    browser: bool,
) -> Result<String, String> {
    let (ssh, host) = workspace_ssh_host(&app, &workspace_id).await?;
    let node = run_on_host(
        &app,
        ssh.as_ref(),
        "node",
        &["--version"],
        Duration::from_secs(10),
    )
    .await?;
    let major = node
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|item| item.parse::<u32>().ok())
        .unwrap_or(0);
    if major < 18 {
        return Err(format!("{host} 的 Node {node} 过旧，需要 Node 18+"));
    }
    let args = if browser {
        PLAYWRIGHT_BROWSER_INSTALL_ARGS
    } else {
        PLAYWRIGHT_INSTALL_ARGS
    };
    let result = run_on_host(
        &app,
        ssh.as_ref(),
        "npm",
        args,
        Duration::from_secs(if browser { 600 } else { 120 }),
    )
    .await?;
    Ok(if browser {
        format!("{host}：Chromium 安装完成。{result}")
    } else {
        format!("{host}：Playwright MCP 已安装：{result}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_document_has_disabled_example() {
        let document = default_mcp_servers();
        assert_eq!(document.servers.len(), 1);
        assert!(!document.servers[0].enabled);
    }

    #[test]
    fn normalize_rejects_empty_name_and_fills_id() {
        let err = normalize_server(McpServerConfig {
            id: String::new(),
            name: "  ".to_string(),
            command: "npx".to_string(),
            args: vec![],
            env: vec![],
            enabled: false,
            notes: None,
            scope: SCOPE_ALL.to_string(),
            workspace_ids: Vec::new(),
            transport: "stdio".to_string(),
            url: None,
            headers: Vec::new(),
            oauth: None,
        })
        .expect_err("empty name");
        assert!(err.contains("名称不能为空"));
        let ok = normalize_server(McpServerConfig {
            id: String::new(),
            name: "fs".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "".to_string()],
            env: vec![McpEnvVar {
                key: "  ".to_string(),
                value: "x".to_string(),
            }],
            enabled: true,
            notes: Some("  ".to_string()),
            scope: SCOPE_WORKSPACES.to_string(),
            workspace_ids: vec![" ws-1 ".to_string(), "ws-1".to_string(), String::new()],
            transport: "stdio".to_string(),
            url: None,
            headers: Vec::new(),
            oauth: None,
        })
        .expect("normalize");
        assert!(!ok.id.is_empty());
        assert_eq!(ok.args, vec!["-y".to_string()]);
        assert!(ok.env.is_empty());
        assert!(ok.notes.is_none());
        assert_eq!(ok.workspace_ids, vec!["ws-1"]);
    }

    #[test]
    fn export_snippet_includes_only_enabled_servers() {
        let document = McpServersDocument {
            servers: vec![
                McpServerConfig {
                    id: "a".to_string(),
                    name: "Alpha".to_string(),
                    command: "npx".to_string(),
                    args: vec!["-y".to_string(), "pkg-a".to_string()],
                    env: vec![McpEnvVar {
                        key: "TOKEN".to_string(),
                        value: "secret".to_string(),
                    }],
                    enabled: true,
                    notes: None,
                    scope: SCOPE_ALL.to_string(),
                    workspace_ids: Vec::new(),
                    transport: "stdio".to_string(),
                    url: None,
                    headers: Vec::new(),
                    oauth: None,
                },
                McpServerConfig {
                    id: "b".to_string(),
                    name: "Beta".to_string(),
                    command: "uvx".to_string(),
                    args: vec![],
                    env: vec![],
                    enabled: false,
                    notes: None,
                    scope: SCOPE_ALL.to_string(),
                    workspace_ids: Vec::new(),
                    transport: "stdio".to_string(),
                    url: None,
                    headers: Vec::new(),
                    oauth: None,
                },
            ],
        };
        let snippet = render_mcp_export_snippet(&document);
        assert!(snippet.contains("\"Alpha\""));
        assert!(snippet.contains("\"pkg-a\""));
        assert!(snippet.contains("\"TOKEN\": \"secret\""));
        assert!(!snippet.contains("Beta"));
        assert!(!snippet.contains("uvx"));
    }

    #[test]
    fn missing_scope_defaults_to_all_and_workspace_filter_matches() {
        let legacy = r#"{
            "id":"legacy","name":"Legacy","command":"npx","args":[],
            "env":[],"enabled":true,"notes":null
        }"#;
        let legacy: McpServerConfig = serde_json::from_str(legacy).expect("legacy config");
        assert_eq!(legacy.scope, SCOPE_ALL);
        assert!(legacy.workspace_ids.is_empty());
        assert!(server_matches_workspace(&legacy, None));

        let scoped = McpServerConfig {
            scope: SCOPE_WORKSPACES.to_string(),
            workspace_ids: vec!["ws-1".to_string()],
            ..legacy
        };
        assert!(server_matches_workspace(&scoped, Some("ws-1")));
        assert!(!server_matches_workspace(&scoped, Some("ws-2")));
        assert!(!server_matches_workspace(&scoped, None));
    }

    #[test]
    fn test_server_uses_saved_config_and_workspace_scope() {
        let mut server = default_mcp_servers().servers.remove(0);
        server.id = "browser".to_string();
        server.enabled = true;
        server.scope = SCOPE_WORKSPACES.to_string();
        server.workspace_ids = vec!["ws-ssh".to_string()];
        let document = McpServersDocument {
            servers: vec![server.clone()],
        };

        assert_eq!(
            saved_test_server(&document, "browser", "ws-ssh")
                .unwrap()
                .command,
            server.command
        );
        assert!(saved_test_server(&document, "browser", "ws-local").is_err());
        assert!(saved_test_server(&document, "missing", "ws-ssh").is_err());
    }

    #[test]
    fn playwright_probe_does_not_install_and_install_requires_explicit_action() {
        assert!(PLAYWRIGHT_PROBE_ARGS.contains(&"--no-install"));
        assert!(!PLAYWRIGHT_PROBE_ARGS.contains(&"--yes"));
        assert!(PLAYWRIGHT_INSTALL_ARGS.contains(&"--yes"));
        assert!(PLAYWRIGHT_INSTALL_ARGS.contains(&"--ignore-scripts"));
        assert!(PLAYWRIGHT_BROWSER_INSTALL_ARGS.contains(&"chromium"));
    }
}
