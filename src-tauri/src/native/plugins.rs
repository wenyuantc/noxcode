//! 插件系统：一个目录 + 清单文件，向会话贡献技能 / 斜杠命令 / 子 Agent 档案 / 钩子 / MCP 服务器。
//!
//! - 清单：`noxcode-plugin.json`（兼容读取 `.noxcode-plugin/plugin.json`、`.zcode-plugin/plugin.json`、
//!   `.claude-plugin/plugin.json`），字段 `name, version, description, description_i18n,
//!   skills, commands, agents, hooks, mcpServers, userConfig`。`skills / commands / agents` 为目录
//!   （字符串或数组，相对插件根），缺省分别为 `skills/`、`commands/`、`agents/`；`hooks` 与
//!   `mcpServers` 可以是内联 JSON 或指向文件的相对路径（缺省 `hooks/hooks.json`、`.mcp.json`）。
//! - 位置：全局 `$APPCONFIG/plugins/<name>/`，工作区 `.noxcode/plugins/<name>/`。
//! - 启用状态与用户配置：`$APPCONFIG/plugins.json`
//!   `{ "disabled": ["name"], "user_config": { "name": { "key": "value" } } }`。
//! - 变量替换：`${NOXCODE_PLUGIN_ROOT}`（别名 `${CLAUDE_PLUGIN_ROOT}`、`${ZCODE_PLUGIN_ROOT}`）、
//!   `${NOXCODE_PROJECT_DIR}`、`${NOXCODE_PLUGIN_DATA}`、`${user_config.<key>}`，作用于钩子命令 / URL
//!   与 MCP 服务器的 command / args / env / url / headers。
//! - 安装源：本地目录（复制）或 git URL（经 `git/runner.rs` 浅克隆）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use crate::db::models::{
    McpEnvVar, McpOAuthConfig, McpServerConfig, NativeHook, MCP_TRANSPORT_HTTP, MCP_TRANSPORT_SSE,
    MCP_TRANSPORT_STDIO,
};
use crate::git::runner::{git, GitTarget, IndexMode};
use crate::native::hooks_config::{parse_claude_settings_hooks, parse_native_hooks_file};
use crate::native::settings::{normalize_native_hooks, HOOK_SOURCE_PLUGIN};

pub const PLUGINS_DIR_NAME: &str = "plugins";
pub const PLUGINS_STATE_FILE: &str = "plugins.json";
pub const PLUGIN_DATA_DIR_NAME: &str = "plugin-data";
pub const WORKSPACE_PLUGINS_DIR: &str = ".noxcode/plugins";
const MANIFEST_CANDIDATES: &[&str] = &[
    "noxcode-plugin.json",
    ".noxcode-plugin/plugin.json",
    ".zcode-plugin/plugin.json",
    ".claude-plugin/plugin.json",
];
const DEFAULT_HOOKS_FILES: &[&str] = &["hooks/hooks.json", "hooks.json"];
const DEFAULT_MCP_FILES: &[&str] = &[".mcp.json", "mcp.json"];
const MAX_PLUGINS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSource {
    Global,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PluginUserConfigField {
    pub key: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// 已解析并可直接使用的插件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePlugin {
    pub name: String,
    pub version: Option<String>,
    pub description: String,
    pub source: PluginSource,
    pub root: String,
    pub manifest_path: String,
    pub enabled: bool,
    pub skill_dirs: Vec<String>,
    pub command_dirs: Vec<String>,
    pub agent_dirs: Vec<String>,
    pub hooks: Vec<NativeHook>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub user_config_fields: Vec<PluginUserConfigField>,
    pub user_config: BTreeMap<String, String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginsState {
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub user_config: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePluginsView {
    pub dir: String,
    pub plugins: Vec<NativePlugin>,
}

// ---------------------------------------------------------------------------
// 清单
// ---------------------------------------------------------------------------

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::String(text) => vec![text],
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

#[derive(Debug, Default, Deserialize)]
struct PluginManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, alias = "descriptionI18n")]
    description_i18n: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    skills: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    commands: Vec<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    agents: Vec<String>,
    #[serde(default)]
    hooks: Option<Value>,
    #[serde(default, alias = "mcpServers", alias = "mcp_servers")]
    mcp_servers: Option<Value>,
    #[serde(default, alias = "userConfig", alias = "user_config")]
    user_config: BTreeMap<String, Value>,
}

/// 插件目录里的清单路径（按候选顺序取第一个存在的）。
pub fn find_manifest(root: &Path) -> Option<PathBuf> {
    MANIFEST_CANDIDATES
        .iter()
        .map(|candidate| root.join(candidate))
        .find(|path| path.is_file())
}

pub fn normalize_plugin_name(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("插件名称不能为空".to_string());
    }
    if trimmed.len() > 64 {
        return Err("插件名称过长（最多 64 个字符）".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        || trimmed.starts_with('.')
    {
        return Err(format!(
            "插件名称只能包含字母、数字、`-`、`_`、`.` 且不能以 `.` 开头：{trimmed}"
        ));
    }
    Ok(trimmed.to_string())
}

/// 变量替换上下文。
pub struct PluginVars<'a> {
    pub root: &'a Path,
    pub project_dir: Option<&'a Path>,
    pub data_dir: &'a Path,
    pub user_config: &'a BTreeMap<String, String>,
}

pub fn substitute_plugin_vars(text: &str, vars: &PluginVars<'_>) -> String {
    let root = vars.root.to_string_lossy();
    let data = vars.data_dir.to_string_lossy();
    let project = vars
        .project_dir
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = text
        .replace("${NOXCODE_PLUGIN_ROOT}", &root)
        .replace("${CLAUDE_PLUGIN_ROOT}", &root)
        .replace("${ZCODE_PLUGIN_ROOT}", &root)
        .replace("${NOXCODE_PLUGIN_DATA}", &data)
        .replace("${CLAUDE_PLUGIN_DATA}", &data)
        .replace("${NOXCODE_PROJECT_DIR}", &project)
        .replace("${CLAUDE_PROJECT_DIR}", &project);
    // ${user_config.key}
    while let Some(start) = out.find("${user_config.") {
        let Some(end_rel) = out[start..].find('}') else {
            break;
        };
        let end = start + end_rel;
        let key = out[start + "${user_config.".len()..end].to_string();
        let value = vars.user_config.get(&key).cloned().unwrap_or_default();
        out.replace_range(start..=end, &value);
    }
    out
}

fn substitute_value(value: &Value, vars: &PluginVars<'_>) -> Value {
    match value {
        Value::String(text) => Value::String(substitute_plugin_vars(text, vars)),
        Value::Array(items) => Value::Array(items.iter().map(|item| substitute_value(item, vars)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), substitute_value(item, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn resolve_dirs(root: &Path, configured: &[String], default_dir: &str) -> Vec<String> {
    let candidates: Vec<PathBuf> = if configured.is_empty() {
        vec![root.join(default_dir)]
    } else {
        configured
            .iter()
            .map(|dir| root.join(dir.trim().trim_start_matches("./")))
            .collect()
    };
    candidates
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// `hooks` 字段：内联对象 / 数组，或相对路径字符串；缺省时探测默认文件。
fn resolve_inline_or_file(
    root: &Path,
    field: Option<&Value>,
    default_files: &[&str],
    errors: &mut Vec<String>,
    label: &str,
) -> Option<Value> {
    match field {
        Some(Value::String(rel)) => {
            let path = root.join(rel.trim().trim_start_matches("./"));
            let value = read_json(&path);
            if value.is_none() {
                errors.push(format!("{label} 文件不存在或不是合法 JSON：{}", path.display()));
            }
            value
        }
        Some(Value::Null) | None => default_files
            .iter()
            .map(|file| root.join(file))
            .find(|path| path.is_file())
            .and_then(|path| read_json(&path)),
        Some(inline) => Some(inline.clone()),
    }
}

fn parse_plugin_hooks(value: &Value, plugin_name: &str) -> Vec<NativeHook> {
    let mut hooks = parse_native_hooks_file(value);
    if hooks.is_empty() {
        hooks = parse_claude_settings_hooks(value, &format!("plugin-{plugin_name}"));
    }
    let mut out = Vec::new();
    for (index, mut hook) in hooks.into_iter().enumerate() {
        if hook.id.trim().is_empty() {
            hook.id = format!("plugin-{plugin_name}-{}", index + 1);
        } else if !hook.id.starts_with("plugin-") {
            hook.id = format!("plugin-{plugin_name}-{}", hook.id);
        }
        hook.source = HOOK_SOURCE_PLUGIN.to_string();
        out.push(hook);
    }
    normalize_native_hooks(out)
}

fn env_pairs(value: Option<&Value>) -> Vec<McpEnvVar> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, item)| {
                    let text = match item {
                        Value::String(text) => text.clone(),
                        Value::Null => return None,
                        other => other.to_string(),
                    };
                    Some(McpEnvVar {
                        key: key.clone(),
                        value: text,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 解析 `mcpServers`：Claude Code 形状 `{ name: { command, args, env, type|transport, url, headers } }`
/// 或本项目形状 `{ servers: [ McpServerConfig ] }` / 数组。
pub fn parse_plugin_mcp_servers(value: &Value, plugin_name: &str) -> Vec<McpServerConfig> {
    let mut out = Vec::new();
    let list_items: Option<Vec<Value>> = match value {
        Value::Array(items) => Some(items.clone()),
        Value::Object(map) if map.get("servers").is_some_and(Value::is_array) => {
            map.get("servers").and_then(Value::as_array).cloned()
        }
        _ => None,
    };
    if let Some(items) = list_items {
        for item in items {
            if let Ok(mut server) = serde_json::from_value::<McpServerConfig>(item) {
                if server.id.trim().is_empty() {
                    server.id = format!("plugin-{plugin_name}-{}", server.name.trim());
                } else {
                    server.id = format!("plugin-{plugin_name}-{}", server.id.trim());
                }
                out.push(server);
            }
        }
        return out;
    }
    let map = match value {
        Value::Object(map) => map
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(|| map.clone()),
        _ => return out,
    };
    for (name, item) in map {
        let Some(entry) = item.as_object() else {
            continue;
        };
        let transport_raw = entry
            .get("type")
            .or_else(|| entry.get("transport"))
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let url = entry
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.trim().is_empty());
        let transport = match transport_raw.as_str() {
            "http" | "streamable-http" | "streamable_http" | "streamablehttp" => MCP_TRANSPORT_HTTP,
            "sse" => MCP_TRANSPORT_SSE,
            "stdio" => MCP_TRANSPORT_STDIO,
            _ if url.is_some() => MCP_TRANSPORT_HTTP,
            _ => MCP_TRANSPORT_STDIO,
        };
        let command = entry
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if transport == MCP_TRANSPORT_STDIO && command.is_empty() {
            continue;
        }
        if transport != MCP_TRANSPORT_STDIO && url.is_none() {
            continue;
        }
        let args = entry
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let oauth = entry
            .get("oauth")
            .cloned()
            .and_then(|value| serde_json::from_value::<McpOAuthConfig>(value).ok());
        out.push(McpServerConfig {
            id: format!("plugin-{plugin_name}-{}", name.trim()),
            name: format!("{plugin_name}/{}", name.trim()),
            command,
            args,
            env: env_pairs(entry.get("env")),
            enabled: true,
            notes: Some(format!("来自插件 {plugin_name}")),
            scope: "all".to_string(),
            workspace_ids: Vec::new(),
            transport: transport.to_string(),
            url,
            headers: env_pairs(entry.get("headers")),
            oauth,
        });
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

fn parse_user_config_fields(raw: &BTreeMap<String, Value>) -> Vec<PluginUserConfigField> {
    raw.iter()
        .map(|(key, value)| match value {
            Value::Object(map) => PluginUserConfigField {
                key: key.clone(),
                description: map
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                default: map.get("default").map(|item| match item {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                }),
                required: map.get("required").and_then(Value::as_bool).unwrap_or(false),
            },
            Value::String(text) => PluginUserConfigField {
                key: key.clone(),
                description: text.clone(),
                default: None,
                required: false,
            },
            _ => PluginUserConfigField {
                key: key.clone(),
                ..PluginUserConfigField::default()
            },
        })
        .collect()
}

fn pick_description(manifest: &PluginManifest) -> String {
    manifest
        .description_i18n
        .get("zh-CN")
        .or_else(|| manifest.description_i18n.get("zh"))
        .cloned()
        .or_else(|| manifest.description.clone())
        .or_else(|| manifest.description_i18n.values().next().cloned())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 解析一个插件目录；清单缺失或非法时返回 `Err`。
pub fn load_plugin(
    root: &Path,
    source: PluginSource,
    state: &PluginsState,
    data_root: &Path,
    project_dir: Option<&Path>,
) -> Result<NativePlugin, String> {
    let manifest_path =
        find_manifest(root).ok_or_else(|| format!("{} 缺少插件清单", root.display()))?;
    let raw = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("读取插件清单失败 {}: {error}", manifest_path.display()))?;
    let manifest: PluginManifest = serde_json::from_str(&raw)
        .map_err(|error| format!("解析插件清单失败 {}: {error}", manifest_path.display()))?;
    let fallback_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin");
    let name = normalize_plugin_name(if manifest.name.trim().is_empty() {
        fallback_name
    } else {
        manifest.name.as_str()
    })?;
    let mut errors = Vec::new();
    let user_config_fields = parse_user_config_fields(&manifest.user_config);
    let mut user_config: BTreeMap<String, String> = user_config_fields
        .iter()
        .filter_map(|field| {
            field
                .default
                .clone()
                .map(|value| (field.key.clone(), value))
        })
        .collect();
    if let Some(saved) = state.user_config.get(&name) {
        for (key, value) in saved {
            user_config.insert(key.clone(), value.clone());
        }
    }
    for field in &user_config_fields {
        if field.required
            && user_config
                .get(&field.key)
                .is_none_or(|value| value.trim().is_empty())
        {
            errors.push(format!("缺少必填配置 {}", field.key));
        }
    }
    let data_dir = data_root.join(&name);
    let vars = PluginVars {
        root,
        project_dir,
        data_dir: &data_dir,
        user_config: &user_config,
    };
    let hooks = resolve_inline_or_file(root, manifest.hooks.as_ref(), DEFAULT_HOOKS_FILES, &mut errors, "hooks")
        .map(|value| parse_plugin_hooks(&substitute_value(&value, &vars), &name))
        .unwrap_or_default();
    let mcp_servers = resolve_inline_or_file(
        root,
        manifest.mcp_servers.as_ref(),
        DEFAULT_MCP_FILES,
        &mut errors,
        "mcpServers",
    )
    .map(|value| parse_plugin_mcp_servers(&substitute_value(&value, &vars), &name))
    .unwrap_or_default();
    let enabled = !state.disabled.iter().any(|item| item.eq_ignore_ascii_case(&name));
    Ok(NativePlugin {
        description: pick_description(&manifest),
        version: manifest
            .version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        source,
        root: root.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        enabled,
        skill_dirs: resolve_dirs(root, &manifest.skills, "skills"),
        command_dirs: resolve_dirs(root, &manifest.commands, "commands"),
        agent_dirs: resolve_dirs(root, &manifest.agents, "agents"),
        hooks,
        mcp_servers,
        user_config_fields,
        user_config,
        errors,
        name,
    })
}

fn collect_plugin_root(
    dir: &Path,
    source: PluginSource,
    state: &PluginsState,
    data_root: &Path,
    project_dir: Option<&Path>,
    out: &mut Vec<NativePlugin>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut roots: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    roots.sort();
    for root in roots {
        if out.len() >= MAX_PLUGINS {
            return;
        }
        if find_manifest(&root).is_none() {
            continue;
        }
        match load_plugin(&root, source, state, data_root, project_dir) {
            Ok(plugin) => {
                if !out
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&plugin.name))
                {
                    out.push(plugin);
                }
            }
            Err(error) => eprintln!("[plugins] 跳过 {}：{error}", root.display()),
        }
    }
}

pub fn plugins_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(PLUGINS_DIR_NAME)
}

pub fn load_state(config_dir: &Path) -> PluginsState {
    read_json(&config_dir.join(PLUGINS_STATE_FILE))
        .and_then(|value| serde_json::from_value::<PluginsState>(value).ok())
        .unwrap_or_default()
}

pub fn save_state(config_dir: &Path, state: &PluginsState) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|error| format!("创建配置目录失败: {error}"))?;
    let raw = serde_json::to_string_pretty(state)
        .map_err(|error| format!("序列化插件状态失败: {error}"))?;
    fs::write(config_dir.join(PLUGINS_STATE_FILE), raw)
        .map_err(|error| format!("写入插件状态失败: {error}"))
}

/// 读取全部插件（含已禁用）；工作区插件优先，同名全局插件被跳过。
pub fn load_plugins(config_dir: Option<&Path>, workspace_root: Option<&Path>) -> Vec<NativePlugin> {
    let state = config_dir.map(load_state).unwrap_or_default();
    let data_root = config_dir
        .map(|dir| dir.join(PLUGIN_DATA_DIR_NAME))
        .unwrap_or_else(|| std::env::temp_dir().join("noxcode-plugin-data"));
    let mut out = Vec::new();
    if let Some(root) = workspace_root {
        collect_plugin_root(
            &root.join(WORKSPACE_PLUGINS_DIR),
            PluginSource::Workspace,
            &state,
            &data_root,
            workspace_root,
            &mut out,
        );
    }
    if let Some(dir) = config_dir {
        collect_plugin_root(
            &plugins_dir(dir),
            PluginSource::Global,
            &state,
            &data_root,
            workspace_root,
            &mut out,
        );
    }
    out
}

/// 只返回已启用的插件。
pub fn load_enabled_plugins(
    config_dir: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<NativePlugin> {
    load_plugins(config_dir, workspace_root)
        .into_iter()
        .filter(|plugin| plugin.enabled)
        .collect()
}

pub fn plugin_hooks(plugins: &[NativePlugin]) -> Vec<NativeHook> {
    plugins
        .iter()
        .flat_map(|plugin| plugin.hooks.iter().cloned())
        .collect()
}

pub fn plugin_mcp_servers(plugins: &[NativePlugin]) -> Vec<McpServerConfig> {
    plugins
        .iter()
        .flat_map(|plugin| plugin.mcp_servers.iter().cloned())
        .collect()
}

pub fn plugin_dirs(plugins: &[NativePlugin], pick: fn(&NativePlugin) -> &Vec<String>) -> Vec<(String, PathBuf)> {
    plugins
        .iter()
        .flat_map(|plugin| {
            pick(plugin)
                .iter()
                .map(|dir| (plugin.name.clone(), PathBuf::from(dir)))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn plugin_skill_dirs(plugins: &[NativePlugin]) -> Vec<(String, PathBuf)> {
    plugin_dirs(plugins, |plugin| &plugin.skill_dirs)
}

pub fn plugin_command_dirs(plugins: &[NativePlugin]) -> Vec<(String, PathBuf)> {
    plugin_dirs(plugins, |plugin| &plugin.command_dirs)
}

pub fn plugin_agent_dirs(plugins: &[NativePlugin]) -> Vec<PathBuf> {
    plugin_dirs(plugins, |plugin| &plugin.agent_dirs)
        .into_iter()
        .map(|(_, dir)| dir)
        .collect()
}

// ---------------------------------------------------------------------------
// 安装
// ---------------------------------------------------------------------------

pub fn looks_like_git_source(source: &str) -> bool {
    let trimmed = source.trim();
    trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
        || trimmed.starts_with("ssh://")
        || trimmed.starts_with("git://")
        || trimmed.ends_with(".git")
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| format!("创建目录失败 {}: {error}", to.display()))?;
    let entries =
        fs::read_dir(from).map_err(|error| format!("读取目录失败 {}: {error}", from.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let target = to.join(&name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .map_err(|error| format!("复制文件失败 {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn temp_install_dir(plugins_root: &Path) -> PathBuf {
    plugins_root.join(format!(
        ".installing-{}",
        crate::native::artifacts::unique_suffix()
    ))
}

/// 安装到 `$APPCONFIG/plugins/<name>/`：本地目录复制，git URL 浅克隆；同名已存在则覆盖。
pub async fn install_plugin_from_source(config_dir: &Path, source: &str) -> Result<NativePlugin, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("安装源不能为空".to_string());
    }
    let plugins_root = plugins_dir(config_dir);
    fs::create_dir_all(&plugins_root).map_err(|error| format!("创建插件目录失败: {error}"))?;
    let staging = temp_install_dir(&plugins_root);
    let staged = if looks_like_git_source(source) {
        let staging_name = staging
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "临时目录名无效".to_string())?
            .to_string();
        let target = GitTarget::Local(plugins_root.clone());
        let output = git(
            &target,
            &["clone", "--depth", "1", source, &staging_name],
            &IndexMode::ReadOnly,
        )
        .await
        .map_err(|error| format!("克隆插件仓库失败: {error}"))?;
        if !output.success() {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("克隆插件仓库失败: {}", output.stderr_lossy().trim()));
        }
        // 去掉 .git，插件以快照形式保存。
        let _ = fs::remove_dir_all(staging.join(".git"));
        staging.clone()
    } else {
        let from = PathBuf::from(source);
        if !from.is_dir() {
            return Err(format!("安装源不是目录也不是 git 地址：{source}"));
        }
        copy_dir_recursive(&from, &staging)?;
        staging.clone()
    };
    let state = load_state(config_dir);
    let parsed = load_plugin(
        &staged,
        PluginSource::Global,
        &state,
        &config_dir.join(PLUGIN_DATA_DIR_NAME),
        None,
    );
    let plugin = match parsed {
        Ok(plugin) => plugin,
        Err(error) => {
            let _ = fs::remove_dir_all(&staged);
            return Err(error);
        }
    };
    let final_dir = plugins_root.join(&plugin.name);
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir)
            .map_err(|error| format!("移除旧版本插件失败: {error}"))?;
    }
    fs::rename(&staged, &final_dir).map_err(|error| format!("放置插件目录失败: {error}"))?;
    load_plugin(
        &final_dir,
        PluginSource::Global,
        &state,
        &config_dir.join(PLUGIN_DATA_DIR_NAME),
        None,
    )
}

pub fn set_plugin_enabled(config_dir: &Path, name: &str, enabled: bool) -> Result<(), String> {
    let name = normalize_plugin_name(name)?;
    let mut state = load_state(config_dir);
    state
        .disabled
        .retain(|item| !item.eq_ignore_ascii_case(&name));
    if !enabled {
        state.disabled.push(name);
    }
    save_state(config_dir, &state)
}

pub fn set_plugin_user_config(
    config_dir: &Path,
    name: &str,
    values: BTreeMap<String, String>,
) -> Result<(), String> {
    let name = normalize_plugin_name(name)?;
    let mut state = load_state(config_dir);
    let entry = state.user_config.entry(name).or_default();
    for (key, value) in values {
        if value.trim().is_empty() {
            entry.remove(&key);
        } else {
            entry.insert(key, value);
        }
    }
    save_state(config_dir, &state)
}

pub fn uninstall_plugin_dir(config_dir: &Path, name: &str) -> Result<(), String> {
    let name = normalize_plugin_name(name)?;
    let dir = plugins_dir(config_dir).join(&name);
    if dir.is_dir() {
        fs::remove_dir_all(&dir).map_err(|error| format!("删除插件目录失败: {error}"))?;
    }
    let mut state = load_state(config_dir);
    state
        .disabled
        .retain(|item| !item.eq_ignore_ascii_case(&name));
    state.user_config.remove(&name);
    save_state(config_dir, &state)
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

fn config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

async fn workspace_root_for<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
) -> Option<PathBuf> {
    crate::native::permission_rules::local_workspace_root(app, workspace_id)
        .await
        .ok()
        .flatten()
}

#[tauri::command]
pub async fn list_native_plugins<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<NativePluginsView, String> {
    let config = config_dir(&app)?;
    let dir = plugins_dir(&config);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|error| format!("创建插件目录失败: {error}"))?;
    }
    let workspace_root = workspace_root_for(&app, workspace_id.as_deref()).await;
    Ok(NativePluginsView {
        dir: dir.to_string_lossy().into_owned(),
        plugins: load_plugins(Some(&config), workspace_root.as_deref()),
    })
}

#[tauri::command]
pub async fn install_native_plugin<R: Runtime>(
    app: AppHandle<R>,
    source: String,
) -> Result<NativePlugin, String> {
    let config = config_dir(&app)?;
    install_plugin_from_source(&config, &source).await
}

#[tauri::command]
pub async fn set_native_plugin_enabled<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let config = config_dir(&app)?;
    set_plugin_enabled(&config, &name, enabled)
}

#[tauri::command]
pub async fn set_native_plugin_user_config<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    values: BTreeMap<String, String>,
) -> Result<(), String> {
    let config = config_dir(&app)?;
    set_plugin_user_config(&config, &name, values)
}

#[tauri::command]
pub async fn uninstall_native_plugin<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<(), String> {
    let config = config_dir(&app)?;
    uninstall_plugin_dir(&config, &name)
}

#[tauri::command]
pub async fn open_native_plugins_dir<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let dir = plugins_dir(&config_dir(&app)?);
    fs::create_dir_all(&dir).map_err(|error| format!("创建插件目录失败: {error}"))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| format!("打开插件目录失败: {error}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::settings::HOOK_EVENT_PRE_TOOL_USE;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "noxcode-plugins-{}",
            crate::native::artifacts::unique_suffix()
        ));
        fs::create_dir_all(&root).expect("mkdir");
        root
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write");
    }

    #[test]
    fn substitutes_root_project_data_and_user_config() {
        let mut user = BTreeMap::new();
        user.insert("token".to_string(), "abc".to_string());
        let vars = PluginVars {
            root: Path::new("/plugins/demo"),
            project_dir: Some(Path::new("/work/repo")),
            data_dir: Path::new("/data/demo"),
            user_config: &user,
        };
        let text = "${CLAUDE_PLUGIN_ROOT}/bin ${NOXCODE_PROJECT_DIR} ${NOXCODE_PLUGIN_DATA} ${user_config.token} ${user_config.missing}!";
        assert_eq!(
            substitute_plugin_vars(text, &vars),
            "/plugins/demo/bin /work/repo /data/demo abc !"
        );
    }

    #[test]
    fn plugin_names_are_validated() {
        assert_eq!(normalize_plugin_name(" my-plugin_1.0 ").expect("ok"), "my-plugin_1.0");
        assert!(normalize_plugin_name("").is_err());
        assert!(normalize_plugin_name("../etc").is_err());
        assert!(normalize_plugin_name(".hidden").is_err());
        assert!(normalize_plugin_name("has space").is_err());
    }

    #[test]
    fn loads_manifest_with_all_contribution_kinds() {
        let config = temp_root();
        let root = plugins_dir(&config).join("demo");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{
              "name": "demo",
              "version": "1.2.0",
              "description": "Demo plugin",
              "description_i18n": { "zh-CN": "演示插件" },
              "commands": ["commands", "extra-commands"],
              "hooks": {
                "hooks": {
                  "PreToolUse": [
                    { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] }
                  ]
                }
              },
              "mcpServers": {
                "docs": { "command": "node", "args": ["${CLAUDE_PLUGIN_ROOT}/server.js"], "env": { "TOKEN": "${user_config.token}" } },
                "remote": { "type": "http", "url": "https://mcp.example.com/${user_config.tenant}" }
              },
              "userConfig": {
                "token": { "description": "API token", "required": true },
                "tenant": { "description": "Tenant", "default": "acme" }
              }
            }"#,
        );
        write(&root.join("skills/review/SKILL.md"), "---\nname: review\ndescription: r\n---\nbody");
        write(&root.join("commands/hello.md"), "Say hi to $ARGUMENTS");
        write(&root.join("agents/helper.md"), "---\nname: helper\ndescription: h\n---\nprompt");

        // 未配置必填项时报错但仍加载。
        let plugins = load_plugins(Some(&config), None);
        assert_eq!(plugins.len(), 1);
        let plugin = &plugins[0];
        assert_eq!(plugin.name, "demo");
        assert_eq!(plugin.version.as_deref(), Some("1.2.0"));
        assert_eq!(plugin.description, "演示插件");
        assert!(plugin.enabled);
        assert_eq!(plugin.skill_dirs.len(), 1);
        assert_eq!(plugin.command_dirs.len(), 1, "不存在的 extra-commands 被忽略");
        assert_eq!(plugin.agent_dirs.len(), 1);
        assert_eq!(plugin.hooks.len(), 1);
        assert_eq!(plugin.hooks[0].event, HOOK_EVENT_PRE_TOOL_USE);
        assert_eq!(plugin.hooks[0].source, HOOK_SOURCE_PLUGIN);
        assert!(plugin.hooks[0].id.starts_with("plugin-demo-"));
        assert_eq!(
            plugin.hooks[0].command,
            format!("{}/guard.sh", root.display())
        );
        assert_eq!(plugin.mcp_servers.len(), 2);
        let docs = plugin
            .mcp_servers
            .iter()
            .find(|server| server.name == "demo/docs")
            .expect("docs server");
        assert_eq!(docs.id, "plugin-demo-docs");
        assert_eq!(docs.transport, MCP_TRANSPORT_STDIO);
        assert_eq!(docs.args[0], format!("{}/server.js", root.display()));
        assert_eq!(docs.env[0].value, "", "未配置的 user_config 替换为空串");
        let remote = plugin
            .mcp_servers
            .iter()
            .find(|server| server.name == "demo/remote")
            .expect("remote server");
        assert_eq!(remote.transport, MCP_TRANSPORT_HTTP);
        assert_eq!(remote.url.as_deref(), Some("https://mcp.example.com/acme"));
        assert_eq!(plugin.errors, vec!["缺少必填配置 token".to_string()]);

        // 写入用户配置并禁用后重新加载。
        let mut values = BTreeMap::new();
        values.insert("token".to_string(), "secret".to_string());
        set_plugin_user_config(&config, "demo", values).expect("save config");
        set_plugin_enabled(&config, "demo", false).expect("disable");
        let plugins = load_plugins(Some(&config), None);
        assert!(!plugins[0].enabled);
        assert!(plugins[0].errors.is_empty());
        assert_eq!(plugins[0].mcp_servers.iter().find(|s| s.name == "demo/docs").unwrap().env[0].value, "secret");
        assert!(load_enabled_plugins(Some(&config), None).is_empty());

        set_plugin_enabled(&config, "demo", true).expect("enable");
        assert_eq!(load_enabled_plugins(Some(&config), None).len(), 1);
        uninstall_plugin_dir(&config, "demo").expect("uninstall");
        assert!(load_plugins(Some(&config), None).is_empty());
        let _ = fs::remove_dir_all(&config);
    }

    #[test]
    fn workspace_plugins_shadow_global_ones() {
        let config = temp_root();
        let workspace = temp_root();
        write(
            &plugins_dir(&config).join("shared/noxcode-plugin.json"),
            r#"{ "name": "shared", "description": "global" }"#,
        );
        write(
            &workspace.join(WORKSPACE_PLUGINS_DIR).join("shared/noxcode-plugin.json"),
            r#"{ "name": "shared", "description": "workspace" }"#,
        );
        write(
            &plugins_dir(&config).join("broken/noxcode-plugin.json"),
            "{ not json",
        );
        let plugins = load_plugins(Some(&config), Some(&workspace));
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].description, "workspace");
        assert_eq!(plugins[0].source, PluginSource::Workspace);
        let _ = fs::remove_dir_all(&config);
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn parses_project_style_mcp_document() {
        let value: Value = serde_json::from_str(
            r#"{ "servers": [ { "id": "x", "name": "X", "command": "npx", "enabled": true } ] }"#,
        )
        .expect("json");
        let servers = parse_plugin_mcp_servers(&value, "p");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "plugin-p-x");
        assert_eq!(servers[0].transport, MCP_TRANSPORT_STDIO);
        let empty = parse_plugin_mcp_servers(&serde_json::json!({ "bad": { "type": "http" } }), "p");
        assert!(empty.is_empty(), "缺 url 的 http 服务器被跳过");
    }

    #[tokio::test]
    async fn install_from_local_directory_copies_and_validates() {
        let config = temp_root();
        let source = temp_root();
        write(
            &source.join("noxcode-plugin.json"),
            r#"{ "name": "local-demo", "description": "d" }"#,
        );
        write(&source.join("commands/a.md"), "hi");
        write(&source.join(".git/HEAD"), "ref: refs/heads/main");
        let plugin = install_plugin_from_source(&config, &source.to_string_lossy())
            .await
            .expect("install");
        assert_eq!(plugin.name, "local-demo");
        let installed = plugins_dir(&config).join("local-demo");
        assert!(installed.join("commands/a.md").is_file());
        assert!(!installed.join(".git").exists(), ".git 不复制");
        assert!(install_plugin_from_source(&config, &config.join("missing").to_string_lossy())
            .await
            .is_err());
        assert!(looks_like_git_source("https://github.com/acme/plugin.git"));
        assert!(looks_like_git_source("git@github.com:acme/plugin.git"));
        assert!(!looks_like_git_source("/tmp/plugin"));
        let _ = fs::remove_dir_all(&config);
        let _ = fs::remove_dir_all(&source);
    }
}
