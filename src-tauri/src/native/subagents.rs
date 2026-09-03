#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::app::network_settings::load_network_settings;
use crate::app::shared::{new_id, sqlite_pool};
use crate::native::api_logs::sqlite_call_log_sink;
use crate::native::channels::{fetch_channel_record, require_channel_api_key};
use crate::native::model::call_log::{CallLogContext, CALL_KIND_SUBAGENT};
use crate::native::model::{ModelClient, ModelClientConfig};
use crate::native::model_catalog::{apply_catalog_defaults, fill_from_catalog};
use crate::native::protocol::record_to_channel;

const SETTINGS_FILE_NAME: &str = "native-subagents.json";
pub const MAX_NATIVE_SUBAGENTS: usize = 32;
pub const MAX_SUBAGENT_NAME_CHARS: usize = 64;
pub const CUSTOM_TOOL_NAMES: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "Bash",
    "Edit",
    "Write",
    "WebFetch",
    "WebSearch",
    "TodoWrite",
    "ApplyPatch",
    "Skill",
];
const RESERVED_NAMES: &[&str] = &["general", "explore", "general-purpose"];
pub const MODEL_MODE_INHERIT: &str = "inherit";
pub const MODEL_MODE_CHANNEL: &str = "channel";
pub const TOOL_MODE_ALL: &str = "all";
pub const TOOL_MODE_CUSTOM: &str = "custom";
pub const SCOPE_ALL: &str = "all";
pub const SCOPE_WORKSPACES: &str = "workspaces";

fn default_scope_all() -> String {
    SCOPE_ALL.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSubagent {
    pub id: String,
    pub name: String,
    pub description: String,
    pub model_mode: String,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub tool_mode: String,
    pub tools: Vec<String>,
    pub system_prompt: String,
    pub inject_agents_md: bool,
    #[serde(default = "default_scope_all")]
    pub scope: String,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    /// 子 Agent 自己的权限模式（default / edit / build / yolo）；`None` 继承父会话。
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// 从可见工具里剔除的工具名（对 `tool_mode=all` 也生效）。
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// `json`（设置页维护）或 `file`（`.md` 档案，只读）。
    #[serde(default = "default_subagent_source")]
    pub source: String,
    /// `.md` 档案的绝对路径。
    #[serde(default)]
    pub path: Option<String>,
    /// 覆盖子 Agent 的最大工具轮次；`None` 用全局设置。
    #[serde(default)]
    pub max_turns: Option<i32>,
    /// 只对子 Agent 开放的技能名；空表示继承父会话全部技能。
    #[serde(default)]
    pub skills: Vec<String>,
}

pub const SUBAGENT_SOURCE_JSON: &str = "json";
pub const SUBAGENT_SOURCE_FILE: &str = "file";

fn default_subagent_source() -> String {
    SUBAGENT_SOURCE_JSON.to_string()
}

/// `.md` 档案目录：工作区 `.noxcode/agents`、`.claude/agents`，全局 `$APPCONFIG/agents`。
pub const WORKSPACE_AGENT_DIRS: &[&str] = &[".noxcode/agents", ".claude/agents"];
pub const GLOBAL_AGENT_DIR: &str = "agents";

fn split_frontmatter(text: &str) -> Option<(Vec<(String, String)>, String)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let header = &rest[..end];
    let body = rest[end + 4..].trim_matches('\n').to_string();
    let fields = header
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((
                key.trim().to_string(),
                value.trim().trim_matches(['"', '\'']).to_string(),
            ))
        })
        .collect();
    Some((fields, body))
}

fn frontmatter_list(value: &str) -> Vec<String> {
    let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// 解析一份 `.md` 子 Agent 档案：frontmatter 里 `name` 必填，`description`、`tools`、
/// `disallowedTools`、`model`、`permissionMode`、`maxTurns`、`skills` 可选；正文即系统提示。
pub fn parse_subagent_markdown(text: &str, path: &Path) -> Result<NativeSubagent, String> {
    let (fields, body) =
        split_frontmatter(text).ok_or_else(|| format!("{} 缺少 frontmatter", path.display()))?;
    let field = |key: &str| -> Option<String> {
        fields
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.clone())
            .filter(|value| !value.trim().is_empty())
    };
    let name = field("name")
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .ok_or_else(|| format!("{} 缺少 name", path.display()))?;
    let name = normalize_subagent_name(&name)?;
    let description = field("description").unwrap_or_else(|| format!("{name} 子 Agent"));
    let tools = field("tools")
        .map(|value| frontmatter_list(&value))
        .unwrap_or_default();
    let tool_mode = if tools.is_empty() || tools.iter().any(|item| item == "*") {
        TOOL_MODE_ALL.to_string()
    } else {
        TOOL_MODE_CUSTOM.to_string()
    };
    let tools = if tool_mode == TOOL_MODE_ALL {
        Vec::new()
    } else {
        normalize_custom_tools(&tools)?
    };
    let disallowed_tools = field("disallowedTools")
        .or_else(|| field("disallowed_tools"))
        .map(|value| normalize_disallowed_tools(&frontmatter_list(&value)))
        .unwrap_or_default();
    // `model` 只接受「继承」；指定渠道模型需要渠道 id，文件档案不携带，留给设置页。
    let permission_mode = normalize_subagent_permission_mode(
        field("permissionMode")
            .or_else(|| field("permission_mode"))
            .as_deref(),
    );
    let max_turns = field("maxTurns")
        .or_else(|| field("max_turns"))
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| (1..=500).contains(value));
    let skills = field("skills")
        .map(|value| frontmatter_list(&value))
        .unwrap_or_default();
    let inject_agents_md = field("injectAgentsMd")
        .or_else(|| field("inject_agents_md"))
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no"
            )
        })
        .unwrap_or(true);
    Ok(NativeSubagent {
        id: format!("md:{}", path.to_string_lossy()),
        name,
        description: cap_description(&description),
        model_mode: MODEL_MODE_INHERIT.to_string(),
        channel_id: None,
        model: None,
        tool_mode,
        tools,
        system_prompt: body.trim().to_string(),
        inject_agents_md,
        scope: SCOPE_ALL.to_string(),
        workspace_ids: Vec::new(),
        permission_mode,
        disallowed_tools,
        source: SUBAGENT_SOURCE_FILE.to_string(),
        path: Some(path.to_string_lossy().into_owned()),
        max_turns,
        skills,
    })
}

fn cap_description(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= 200 {
        trimmed.to_string()
    } else {
        trimmed.chars().take(200).collect()
    }
}

fn load_agent_dir(dir: &Path) -> Vec<NativeSubagent> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<NativeSubagent> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;
            match parse_subagent_markdown(&text, &path) {
                Ok(item) => Some(item),
                Err(error) => {
                    eprintln!("[native] 忽略无效的子 Agent 档案: {error}");
                    None
                }
            }
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

/// 读取工作区、已启用插件与全局目录里的 `.md` 档案；按该顺序优先，同名只保留先出现的。
pub fn load_markdown_subagents(
    workspace_root: Option<&Path>,
    config_dir: Option<&Path>,
) -> Vec<NativeSubagent> {
    let plugins = crate::native::plugins::load_enabled_plugins(config_dir, workspace_root);
    load_markdown_subagents_with(
        workspace_root,
        config_dir,
        &crate::native::plugins::plugin_agent_dirs(&plugins),
    )
}

pub fn load_markdown_subagents_with(
    workspace_root: Option<&Path>,
    config_dir: Option<&Path>,
    plugin_dirs: &[std::path::PathBuf],
) -> Vec<NativeSubagent> {
    let mut out: Vec<NativeSubagent> = Vec::new();
    let mut push_all = |items: Vec<NativeSubagent>| {
        for item in items {
            if !out
                .iter()
                .any(|existing| names_equal(&existing.name, &item.name))
            {
                out.push(item);
            }
        }
    };
    if let Some(root) = workspace_root {
        for dir in WORKSPACE_AGENT_DIRS {
            push_all(load_agent_dir(&root.join(dir)));
        }
    }
    for dir in plugin_dirs {
        push_all(load_agent_dir(dir));
    }
    if let Some(config) = config_dir {
        push_all(load_agent_dir(&config.join(GLOBAL_AGENT_DIR)));
    }
    out
}

/// 合并设置页（json）与档案（file）子 Agent：json 优先，同名档案被跳过。
pub fn merge_subagent_sources(
    json_items: Vec<NativeSubagent>,
    file_items: Vec<NativeSubagent>,
) -> Vec<NativeSubagent> {
    let mut out = json_items;
    for item in file_items {
        if !out
            .iter()
            .any(|existing| names_equal(&existing.name, &item.name))
        {
            out.push(item);
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNativeSubagent {
    pub name: String,
    pub description: String,
    pub model_mode: Option<String>,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub tool_mode: Option<String>,
    pub tools: Option<Vec<String>>,
    pub system_prompt: Option<String>,
    pub inject_agents_md: Option<bool>,
    pub scope: Option<String>,
    pub workspace_ids: Option<Vec<String>>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateNativeSubagent {
    pub name: Option<String>,
    pub description: Option<String>,
    pub model_mode: Option<String>,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub tool_mode: Option<String>,
    pub tools: Option<Vec<String>>,
    pub system_prompt: Option<String>,
    pub inject_agents_md: Option<bool>,
    pub scope: Option<String>,
    pub workspace_ids: Option<Vec<String>>,
    /// `Some(Some(mode))` 设置，`Some(None)` 清空回继承，`None` 不改。
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub permission_mode: Option<Option<String>>,
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,
}

fn deserialize_explicit_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// 子 Agent 权限模式：空 / 非法值视为继承（`None`）。
pub fn normalize_subagent_permission_mode(value: Option<&str>) -> Option<String> {
    let trimmed = value.map(str::trim).unwrap_or("");
    if trimmed.is_empty() || trimmed == "inherit" {
        return None;
    }
    Some(crate::native::settings::normalize_permission_mode(Some(
        trimmed,
    )))
}

pub fn normalize_disallowed_tools(tools: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in tools {
        let trimmed = tool.trim();
        if trimmed.is_empty() || out.iter().any(|existing| names_equal(existing, trimmed)) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct RawNativeSubagentsFile {
    #[serde(default)]
    subagents: Vec<NativeSubagent>,
}

#[derive(Clone)]
pub struct ChildModelSettings {
    pub client: ModelClient,
    pub model: String,
    pub effort: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub thinking_enabled: bool,
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(SETTINGS_FILE_NAME))
}

pub fn normalize_subagent_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.chars().count() > MAX_SUBAGENT_NAME_CHARS {
        return Err("名称最多 64 个字符".to_string());
    }
    if RESERVED_NAMES
        .iter()
        .any(|item| item.eq_ignore_ascii_case(name))
    {
        return Err("名称不能使用内置类型 general / explore".to_string());
    }
    let valid = name.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || ch == '_'
            || ch == '-'
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
    });
    if !valid {
        return Err("名称只能包含字母、数字、下划线、连字符或中文".to_string());
    }
    Ok(name.to_string())
}

fn normalize_description(value: &str) -> Result<String, String> {
    let description = value.trim();
    if description.is_empty() {
        return Err("描述不能为空".to_string());
    }
    Ok(description.to_string())
}

fn normalize_model_mode(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).unwrap_or(MODEL_MODE_INHERIT) {
        "" | MODEL_MODE_INHERIT => Ok(MODEL_MODE_INHERIT.to_string()),
        MODEL_MODE_CHANNEL => Ok(MODEL_MODE_CHANNEL.to_string()),
        other => Err(format!("未知模型模式：{other}，应为 inherit 或 channel")),
    }
}

fn normalize_tool_mode(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).unwrap_or(TOOL_MODE_ALL) {
        "" | TOOL_MODE_ALL => Ok(TOOL_MODE_ALL.to_string()),
        TOOL_MODE_CUSTOM => Ok(TOOL_MODE_CUSTOM.to_string()),
        other => Err(format!("未知工具模式：{other}，应为 all 或 custom")),
    }
}

pub fn normalize_custom_tools(tools: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for tool in tools {
        let name = tool.trim();
        if name.is_empty() {
            continue;
        }
        if !CUSTOM_TOOL_NAMES.contains(&name) {
            return Err(format!(
                "不支持的工具：{name}，可选 {}",
                CUSTOM_TOOL_NAMES.join("、")
            ));
        }
        if !out.iter().any(|item| item == name) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

pub fn effective_custom_tools(tools: &[String]) -> Vec<String> {
    let mut out = tools.to_vec();
    if out.iter().any(|item| item == "TodoWrite") && !out.iter().any(|item| item == "TodoRead") {
        out.push("TodoRead".to_string());
    }
    out
}

pub fn custom_tools_are_read_only(tools: &[String]) -> bool {
    !tools
        .iter()
        .any(|item| item == "Write" || item == "Edit" || item == "Bash" || item == "ApplyPatch")
}

fn names_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn normalize_scope(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).unwrap_or(SCOPE_ALL) {
        "" | SCOPE_ALL => Ok(SCOPE_ALL.to_string()),
        SCOPE_WORKSPACES => Ok(SCOPE_WORKSPACES.to_string()),
        other => Err(format!("未知作用域：{other}，应为 all 或 workspaces")),
    }
}

fn normalize_workspace_ids(ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if !out.iter().any(|item| item == id) {
            out.push(id.to_string());
        }
    }
    out
}

pub fn subagent_matches_workspace(item: &NativeSubagent, workspace_id: Option<&str>) -> bool {
    if item.scope != SCOPE_WORKSPACES {
        return true;
    }
    let Some(workspace_id) = workspace_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    item.workspace_ids.iter().any(|id| id == workspace_id)
}

pub fn filter_native_subagents_for_workspace(
    items: &[NativeSubagent],
    workspace_id: Option<&str>,
) -> Vec<NativeSubagent> {
    items
        .iter()
        .filter(|item| subagent_matches_workspace(item, workspace_id))
        .cloned()
        .collect()
}

pub fn catalog_for_session(
    items: &[NativeSubagent],
    workspace_id: Option<&str>,
    bound: Option<&NativeSubagent>,
) -> Vec<NativeSubagent> {
    let mut out = filter_native_subagents_for_workspace(items, workspace_id);
    if let Some(bound) = bound {
        if !out.iter().any(|item| item.id == bound.id) {
            out.push(bound.clone());
        }
    }
    out
}

fn ensure_unique_name(
    items: &[NativeSubagent],
    name: &str,
    skip_id: Option<&str>,
) -> Result<(), String> {
    if items
        .iter()
        .any(|item| skip_id != Some(item.id.as_str()) && names_equal(&item.name, name))
    {
        return Err(format!("子智能体名称「{name}」已存在"));
    }
    Ok(())
}

fn normalize_record(mut item: NativeSubagent) -> Result<NativeSubagent, String> {
    item.name = normalize_subagent_name(&item.name)?;
    item.description = normalize_description(&item.description)?;
    item.model_mode = normalize_model_mode(Some(&item.model_mode))?;
    item.tool_mode = normalize_tool_mode(Some(&item.tool_mode))?;
    item.system_prompt = item.system_prompt.trim().to_string();
    if item.model_mode == MODEL_MODE_INHERIT {
        item.channel_id = None;
        item.model = None;
    } else {
        let channel_id = item
            .channel_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "指定渠道模型时必须选择渠道".to_string())?;
        let model = item
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "指定渠道模型时必须选择模型".to_string())?;
        item.channel_id = Some(channel_id.to_string());
        item.model = Some(model.to_string());
    }
    if item.tool_mode == TOOL_MODE_ALL {
        item.tools = Vec::new();
    } else {
        item.tools = normalize_custom_tools(&item.tools)?;
        if item.tools.is_empty() {
            return Err("自定义可用工具至少勾选一项".to_string());
        }
    }
    item.scope = normalize_scope(Some(&item.scope))?;
    if item.scope == SCOPE_ALL {
        item.workspace_ids = Vec::new();
    } else {
        item.workspace_ids = normalize_workspace_ids(&item.workspace_ids);
        if item.workspace_ids.is_empty() {
            return Err("指定工作区时至少选择一个工作区".to_string());
        }
    }
    item.permission_mode = normalize_subagent_permission_mode(item.permission_mode.as_deref());
    item.disallowed_tools = normalize_disallowed_tools(&item.disallowed_tools);
    Ok(item)
}

fn load_file(path: &PathBuf) -> Result<Vec<NativeSubagent>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        fs::read_to_string(path).map_err(|error| format!("读取子智能体配置失败: {error}"))?;
    let raw = serde_json::from_str::<RawNativeSubagentsFile>(&text).unwrap_or_default();
    let mut items = Vec::new();
    for item in raw.subagents {
        if let Ok(normalized) = normalize_record(item) {
            if ensure_unique_name(&items, &normalized.name, None).is_ok() {
                items.push(normalized);
            }
        }
    }
    Ok(items)
}

fn save_file(path: &PathBuf, items: &[NativeSubagent]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建子智能体配置目录失败: {error}"))?;
    }
    let raw = RawNativeSubagentsFile {
        subagents: items.to_vec(),
    };
    let json = serde_json::to_string_pretty(&raw)
        .map_err(|error| format!("序列化子智能体配置失败: {error}"))?;
    fs::write(path, json).map_err(|error| format!("写入子智能体配置失败: {error}"))
}

pub fn load_native_subagents<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<NativeSubagent>, String> {
    load_file(&settings_path(app)?)
}

pub fn find_native_subagent<'a>(
    items: &'a [NativeSubagent],
    name: &str,
) -> Option<&'a NativeSubagent> {
    let trimmed = name.trim();
    items.iter().find(|item| names_equal(&item.name, trimmed))
}

pub fn find_native_subagent_by_id<'a>(
    items: &'a [NativeSubagent],
    id: &str,
) -> Option<&'a NativeSubagent> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    items.iter().find(|item| item.id == trimmed)
}

pub fn validate_native_subagent_id<R: Runtime>(
    app: &AppHandle<R>,
    id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(id) = id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let items = load_native_subagents(app)?;
    let Some(item) = find_native_subagent_by_id(&items, id) else {
        return Err("所选子智能体不存在".to_string());
    };
    if let Some(workspace_id) = workspace_id {
        if !subagent_matches_workspace(item, Some(workspace_id)) {
            return Err("该子智能体不适用于当前工作区".to_string());
        }
    }
    Ok(Some(id.to_string()))
}

async fn validate_live_workspace_ids<R: Runtime>(
    app: &AppHandle<R>,
    ids: &[String],
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let pool = sqlite_pool(app).await?;
    let live: Vec<String> = sqlx::query_scalar::<_, String>("SELECT id FROM workspaces")
        .fetch_all(&pool)
        .await
        .map_err(|error| format!("校验工作区失败: {error}"))?;
    let live: HashSet<&str> = live.iter().map(String::as_str).collect();
    for id in ids {
        if !live.contains(id.as_str()) {
            return Err(format!("工作区不存在：{id}"));
        }
    }
    Ok(())
}

async fn validate_channel_model<R: Runtime>(
    app: &AppHandle<R>,
    channel_id: &str,
    model: &str,
) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    let record = fetch_channel_record(&pool, channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    let channel = record_to_channel(record)?;
    if !channel.models.iter().any(|item| item.id == model) {
        return Err(format!("渠道「{}」没有模型 {model}", channel.name));
    }
    Ok(())
}

async fn persist_and_log<R: Runtime>(
    app: &AppHandle<R>,
    items: &[NativeSubagent],
    _action: &str,
    _details: &str,
) -> Result<(), String> {
    save_file(&settings_path(app)?, items)
}

fn extra_headers_map(raw: Option<&str>) -> HashMap<String, String> {
    let Some(text) = raw.filter(|item| !item.trim().is_empty()) else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(text).unwrap_or_default()
}

pub async fn resolve_child_model<R: Runtime>(
    app: &AppHandle<R>,
    channel_id: &str,
    model: &str,
) -> Result<ChildModelSettings, String> {
    let pool = sqlite_pool(app).await?;
    let record = fetch_channel_record(&pool, channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    let api_key = require_channel_api_key(&record)?;
    let channel = record_to_channel(record)?;
    if !channel.models.iter().any(|item| item.id == model) {
        return Err(format!("渠道「{}」没有模型 {model}", channel.name));
    }
    let mut config = channel
        .models
        .iter()
        .find(|item| item.id == model)
        .cloned()
        .unwrap_or_else(|| apply_catalog_defaults(model));
    fill_from_catalog(&mut config);
    let thinking_enabled = config.thinking_enabled.unwrap_or(false);
    let effort = if thinking_enabled {
        config.thinking_level.clone()
    } else {
        None
    };
    let client = ModelClient::new(ModelClientConfig {
        protocol: channel.protocol,
        base_url: channel.base_url,
        api_key,
        extra_headers: extra_headers_map(channel.extra_headers_json.as_deref()),
        retry: crate::native::settings::effective_model_retry_config(app),
        timeout: Duration::from_secs(if thinking_enabled { 300 } else { 120 }),
        network: load_network_settings(app)?,
    })?
    .with_call_log(
        CallLogContext {
            channel_id: Some(channel.id),
            channel_name: Some(channel.name),
            session_id: None,
            profile_id: None,
            workspace_id: None,
            subagent_id: None,
            call_kind: Some(CALL_KIND_SUBAGENT.to_string()),
            execution_target: None,
            operation: None,
            model_role: None,
        },
        sqlite_call_log_sink(pool.clone()),
    );
    Ok(ChildModelSettings {
        client,
        model: model.to_string(),
        effort,
        max_output_tokens: config.max_output_tokens,
        thinking_enabled,
    })
}

#[tauri::command]
pub async fn list_native_subagents<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<Vec<NativeSubagent>, String> {
    let json_items = load_native_subagents(&app)?;
    let config_dir = app.path().app_config_dir().ok();
    let workspace_root =
        crate::native::permission_rules::local_workspace_root(&app, workspace_id.as_deref())
            .await
            .unwrap_or(None);
    let file_items = load_markdown_subagents(workspace_root.as_deref(), config_dir.as_deref());
    Ok(merge_subagent_sources(json_items, file_items))
}

/// 会话可用的全部子 Agent：设置页 json + 工作区 / 全局 `.md` 档案，再按工作区作用域过滤。
pub fn load_session_subagents<R: Runtime>(
    app: &AppHandle<R>,
    workspace_root: Option<&Path>,
) -> Vec<NativeSubagent> {
    let json_items = load_native_subagents(app).unwrap_or_default();
    let config_dir = app.path().app_config_dir().ok();
    let file_items = load_markdown_subagents(workspace_root, config_dir.as_deref());
    merge_subagent_sources(json_items, file_items)
}

#[tauri::command]
pub async fn create_native_subagent<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateNativeSubagent,
) -> Result<NativeSubagent, String> {
    let mut items = load_native_subagents(&app)?;
    if items.len() >= MAX_NATIVE_SUBAGENTS {
        return Err(format!("最多配置 {MAX_NATIVE_SUBAGENTS} 个子智能体"));
    }
    let record = normalize_record(NativeSubagent {
        id: new_id(),
        name: payload.name,
        description: payload.description,
        model_mode: payload
            .model_mode
            .unwrap_or_else(|| MODEL_MODE_INHERIT.to_string()),
        channel_id: payload.channel_id,
        model: payload.model,
        tool_mode: payload
            .tool_mode
            .unwrap_or_else(|| TOOL_MODE_ALL.to_string()),
        tools: payload.tools.unwrap_or_default(),
        system_prompt: payload.system_prompt.unwrap_or_default(),
        inject_agents_md: payload.inject_agents_md.unwrap_or(true),
        scope: payload.scope.unwrap_or_else(|| SCOPE_ALL.to_string()),
        workspace_ids: payload.workspace_ids.unwrap_or_default(),
        permission_mode: payload.permission_mode,
        disallowed_tools: payload.disallowed_tools.unwrap_or_default(),
        source: SUBAGENT_SOURCE_JSON.to_string(),
        path: None,
        max_turns: None,
        skills: Vec::new(),
    })?;
    ensure_unique_name(&items, &record.name, None)?;
    if record.scope == SCOPE_WORKSPACES {
        validate_live_workspace_ids(&app, &record.workspace_ids).await?;
    }
    if record.model_mode == MODEL_MODE_CHANNEL {
        validate_channel_model(
            &app,
            record.channel_id.as_deref().unwrap_or(""),
            record.model.as_deref().unwrap_or(""),
        )
        .await?;
    }
    items.push(record.clone());
    persist_and_log(
        &app,
        &items,
        "native_subagent_created",
        &format!("创建子智能体：{}", record.name),
    )
    .await?;
    Ok(record)
}

#[tauri::command]
pub async fn update_native_subagent<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    payload: UpdateNativeSubagent,
) -> Result<NativeSubagent, String> {
    let mut items = load_native_subagents(&app)?;
    let index = items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "子智能体不存在".to_string())?;
    let mut next = items[index].clone();
    if let Some(name) = payload.name {
        next.name = name;
    }
    if let Some(description) = payload.description {
        next.description = description;
    }
    if let Some(model_mode) = payload.model_mode {
        next.model_mode = model_mode;
    }
    if payload.channel_id.is_some() {
        next.channel_id = payload.channel_id;
    }
    if payload.model.is_some() {
        next.model = payload.model;
    }
    if let Some(tool_mode) = payload.tool_mode {
        next.tool_mode = tool_mode;
    }
    if let Some(tools) = payload.tools {
        next.tools = tools;
    }
    if let Some(system_prompt) = payload.system_prompt {
        next.system_prompt = system_prompt;
    }
    if let Some(inject_agents_md) = payload.inject_agents_md {
        next.inject_agents_md = inject_agents_md;
    }
    if let Some(scope) = payload.scope {
        next.scope = scope;
    }
    if let Some(workspace_ids) = payload.workspace_ids {
        next.workspace_ids = workspace_ids;
    }
    if let Some(permission_mode) = payload.permission_mode {
        next.permission_mode = permission_mode;
    }
    if let Some(disallowed_tools) = payload.disallowed_tools {
        next.disallowed_tools = disallowed_tools;
    }
    let record = normalize_record(next)?;
    ensure_unique_name(&items, &record.name, Some(&id))?;
    if record.scope == SCOPE_WORKSPACES {
        validate_live_workspace_ids(&app, &record.workspace_ids).await?;
    }
    if record.model_mode == MODEL_MODE_CHANNEL {
        validate_channel_model(
            &app,
            record.channel_id.as_deref().unwrap_or(""),
            record.model.as_deref().unwrap_or(""),
        )
        .await?;
    }
    items[index] = record.clone();
    persist_and_log(
        &app,
        &items,
        "native_subagent_updated",
        &format!("更新子智能体：{}", record.name),
    )
    .await?;
    Ok(record)
}

#[tauri::command]
pub async fn delete_native_subagent<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(), String> {
    let mut items = load_native_subagents(&app)?;
    let index = items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "子智能体不存在".to_string())?;
    let name = items[index].name.clone();
    items.remove(index);
    persist_and_log(
        &app,
        &items,
        "native_subagent_deleted",
        &format!("删除子智能体：{name}"),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_profiles_parse_and_merge_after_json() {
        let dir = std::env::temp_dir().join(format!(
            "noxcode-agent-md-{}",
            crate::native::artifacts::unique_suffix()
        ));
        let agents = dir.join(".noxcode/agents");
        fs::create_dir_all(&agents).expect("mkdir");
        fs::write(
            agents.join("reviewer.md"),
            "---\nname: code-reviewer\ndescription: 审查 diff\ntools: Read, Grep, Glob\ndisallowedTools: [Bash]\npermissionMode: yolo\nmaxTurns: 12\nskills: [review]\n---\n你是严格的审查员。\n",
        )
        .expect("write");
        fs::write(agents.join("broken.md"), "no frontmatter").expect("write");
        fs::write(
            agents.join("all-tools.md"),
            "---\nname: fixer\n---\n修 bug。",
        )
        .expect("write");
        let items = load_markdown_subagents(Some(&dir), None);
        assert_eq!(items.len(), 2);
        let reviewer = items
            .iter()
            .find(|item| item.name == "code-reviewer")
            .expect("reviewer");
        assert_eq!(reviewer.source, SUBAGENT_SOURCE_FILE);
        assert_eq!(reviewer.tool_mode, TOOL_MODE_CUSTOM);
        assert_eq!(reviewer.tools, vec!["Read", "Grep", "Glob"]);
        assert_eq!(reviewer.disallowed_tools, vec!["Bash"]);
        assert_eq!(reviewer.permission_mode.as_deref(), Some("yolo"));
        assert_eq!(reviewer.max_turns, Some(12));
        assert_eq!(reviewer.skills, vec!["review"]);
        assert_eq!(reviewer.system_prompt, "你是严格的审查员。");
        assert!(reviewer.path.as_deref().unwrap().ends_with("reviewer.md"));
        let fixer = items
            .iter()
            .find(|item| item.name == "fixer")
            .expect("fixer");
        assert_eq!(fixer.tool_mode, TOOL_MODE_ALL);
        assert_eq!(fixer.description, "fixer 子 Agent");
        // 同名的 json 条目优先。
        let mut json_item = sample();
        json_item.name = "code-reviewer".to_string();
        let merged = merge_subagent_sources(vec![json_item], items);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].source, SUBAGENT_SOURCE_JSON);
        let _ = fs::remove_dir_all(dir);
    }

    fn sample() -> NativeSubagent {
        NativeSubagent {
            id: "1".to_string(),
            name: "code-reviewer".to_string(),
            description: "审查 diff".to_string(),
            model_mode: MODEL_MODE_INHERIT.to_string(),
            channel_id: None,
            model: None,
            tool_mode: TOOL_MODE_CUSTOM.to_string(),
            tools: vec!["Read".to_string(), "Grep".to_string()],
            system_prompt: "你是审查员".to_string(),
            inject_agents_md: true,
            scope: SCOPE_ALL.to_string(),
            workspace_ids: Vec::new(),
            permission_mode: None,
            disallowed_tools: Vec::new(),
            source: SUBAGENT_SOURCE_JSON.to_string(),
            path: None,
            max_turns: None,
            skills: Vec::new(),
        }
    }

    #[test]
    fn name_rejects_reserved_and_empty() {
        assert!(normalize_subagent_name("").is_err());
        assert!(normalize_subagent_name("general").is_err());
        assert!(normalize_subagent_name("Explore").is_err());
        assert!(normalize_subagent_name("general-purpose").is_err());
        assert_eq!(normalize_subagent_name(" 代码审查 ").unwrap(), "代码审查");
        assert_eq!(
            normalize_subagent_name("code-reviewer").unwrap(),
            "code-reviewer"
        );
        assert!(normalize_subagent_name("bad name").is_err());
    }

    #[test]
    fn custom_tools_require_whitelist_and_todo_read() {
        let err = normalize_custom_tools(&["Agent".to_string()]).expect_err("agent");
        assert!(err.contains("不支持的工具"));
        let tools = normalize_custom_tools(&["Read".to_string(), "TodoWrite".to_string()]).unwrap();
        let effective = effective_custom_tools(&tools);
        assert!(effective.contains(&"TodoRead".to_string()));
        assert!(custom_tools_are_read_only(&[
            "Read".to_string(),
            "Grep".to_string()
        ]));
        assert!(!custom_tools_are_read_only(&["Bash".to_string()]));
        assert!(!custom_tools_are_read_only(&["ApplyPatch".to_string()]));
        assert!(custom_tools_are_read_only(&["Skill".to_string()]));
    }

    #[test]
    fn normalize_record_inherit_clears_channel() {
        let mut item = sample();
        item.model_mode = MODEL_MODE_INHERIT.to_string();
        item.channel_id = Some("ch".to_string());
        item.model = Some("m".to_string());
        let out = normalize_record(item).unwrap();
        assert!(out.channel_id.is_none());
        assert!(out.model.is_none());
    }

    #[test]
    fn custom_tools_empty_rejected() {
        let mut item = sample();
        item.tools = Vec::new();
        let err = normalize_record(item).expect_err("empty tools");
        assert!(err.contains("至少勾选"));
    }

    #[test]
    fn channel_mode_requires_ids() {
        let mut item = sample();
        item.model_mode = MODEL_MODE_CHANNEL.to_string();
        item.channel_id = None;
        item.model = Some("m".to_string());
        let err = normalize_record(item).expect_err("channel");
        assert!(err.contains("必须选择渠道"));
    }

    #[test]
    fn unique_name_is_case_insensitive() {
        let items = vec![sample()];
        let err = ensure_unique_name(&items, "Code-Reviewer", None).expect_err("dup");
        assert!(err.contains("已存在"));
        ensure_unique_name(&items, "Code-Reviewer", Some("1")).unwrap();
    }

    #[test]
    fn missing_scope_deserializes_as_all() {
        let json = r#"{
            "id":"1","name":"reviewer","description":"审",
            "model_mode":"inherit","channel_id":null,"model":null,
            "tool_mode":"all","tools":[],"system_prompt":"",
            "inject_agents_md":true
        }"#;
        let item: NativeSubagent = serde_json::from_str(json).unwrap();
        assert_eq!(item.scope, SCOPE_ALL);
        assert!(item.workspace_ids.is_empty());
    }

    #[test]
    fn projects_scope_requires_ids() {
        let mut item = sample();
        item.scope = SCOPE_WORKSPACES.to_string();
        item.workspace_ids = Vec::new();
        let err = normalize_record(item).expect_err("empty projects");
        assert!(err.contains("至少选择一个工作区"));
    }

    #[test]
    fn all_scope_clears_workspace_ids() {
        let mut item = sample();
        item.scope = SCOPE_ALL.to_string();
        item.workspace_ids = vec!["p1".to_string()];
        let out = normalize_record(item).unwrap();
        assert_eq!(out.scope, SCOPE_ALL);
        assert!(out.workspace_ids.is_empty());
    }

    #[test]
    fn filter_keeps_all_and_matching_projects() {
        let mut all = sample();
        all.id = "all".to_string();
        let mut scoped = sample();
        scoped.id = "p".to_string();
        scoped.name = "project-only".to_string();
        scoped.scope = SCOPE_WORKSPACES.to_string();
        scoped.workspace_ids = vec!["alpha".to_string(), "beta".to_string()];
        let items = vec![all.clone(), scoped.clone()];
        let for_alpha = filter_native_subagents_for_workspace(&items, Some("alpha"));
        assert_eq!(for_alpha.len(), 2);
        let for_other = filter_native_subagents_for_workspace(&items, Some("other"));
        assert_eq!(for_other.len(), 1);
        assert_eq!(for_other[0].id, "all");
        let free = filter_native_subagents_for_workspace(&items, None);
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].id, "all");
        let catalog = catalog_for_session(&items, Some("other"), Some(&scoped));
        assert_eq!(catalog.len(), 2);
        assert!(catalog.iter().any(|item| item.id == "p"));
    }
}
