use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::new_id;
use crate::db::models::{McpEnvVar, McpServerConfig, McpServersDocument, UpdateMcpServersPayload};

const MCP_SERVERS_FILE_NAME: &str = "mcp-servers.json";

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
        }],
    }
}

fn normalize_server(mut server: McpServerConfig) -> Result<McpServerConfig, String> {
    server.name = server.name.trim().to_string();
    server.command = server.command.trim().to_string();
    if server.name.is_empty() {
        return Err("MCP 服务器名称不能为空".to_string());
    }
    if server.command.is_empty() {
        return Err("MCP 启动命令不能为空".to_string());
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
) -> Result<Vec<McpServerConfig>, String> {
    let document = load_mcp_document(app)?;
    Ok(document
        .servers
        .into_iter()
        .filter(|server| server.enabled)
        .collect())
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
        })
        .expect("normalize");
        assert!(!ok.id.is_empty());
        assert_eq!(ok.args, vec!["-y".to_string()]);
        assert!(ok.env.is_empty());
        assert!(ok.notes.is_none());
    }
}
