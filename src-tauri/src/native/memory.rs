//! 持久记忆（对齐 ZCode 的 MEMORY.md 目录）。
//!
//! 每个工作区一个目录 `$APPCONFIG/memory/<project_key>/`：`MEMORY.md` 是索引
//! （每行一条 `- [名称](文件.md) — 一句话描述`，上限 200 行），其余 `*.md` 是事实文件，
//! 带 frontmatter `name / description / type / created_at / updated_at`。
//!
//! 三条管线：`extract`（会话结束后用轻量模型抽取候选并落盘）、`recall`（每回合按
//! 关键词命中前几条注入用户消息）、`dream`（周期性让模型合并 / 去重 / 重写索引）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::now_sqlite;
use crate::native::model::call_log::{
    MODEL_ROLE_LITE, MODEL_ROLE_MAIN, OPERATION_MEMORY_DREAM, OPERATION_MEMORY_EXTRACT,
};
use crate::native::model::client::ChatRequest;
use crate::native::model::types::{Message, Role};
use crate::native::model::ModelClient;

pub const MEMORY_DIR_NAME: &str = "memory";
pub const MEMORY_INDEX_FILE: &str = "MEMORY.md";
pub const MEMORY_INDEX_MAX_LINES: usize = 200;
const MEMORY_INDEX_MAX_CHARS: usize = 16_000;
const MEMORY_BODY_MAX_CHARS: usize = 6_000;
const MEMORY_STATE_FILE: &str = ".state.json";
const EXTRACT_TRANSCRIPT_CHARS: usize = 24_000;
const EXTRACT_MAX_ENTRIES: usize = 8;
const DREAM_MAX_ENTRIES: usize = 60;
pub const MEMORY_TYPES: &[&str] = &["user", "feedback", "project", "reference"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEntry {
    pub file_name: String,
    pub name: String,
    pub description: String,
    /// `user` / `feedback` / `project` / `reference`。
    pub kind: String,
    pub created_at: String,
    pub updated_at: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryHit {
    pub file_name: String,
    pub name: String,
    pub description: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MemoryState {
    #[serde(default)]
    extractions: u32,
    #[serde(default)]
    dreams: u32,
    #[serde(default)]
    last_extracted_at: Option<String>,
    #[serde(default)]
    last_dreamed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryView {
    pub dir: String,
    pub index: String,
    pub entries: Vec<MemoryEntry>,
    pub extractions: u32,
    pub dreams: u32,
}

/// 工作区目录 → 稳定的项目键：最后一段路径名 + 8 位哈希。
pub fn project_key(workspace_root: &str) -> String {
    let normalized = workspace_root.trim().trim_end_matches(['/', '\\']);
    let leaf = normalized
        .rsplit(['/', '\\'])
        .next()
        .filter(|item| !item.is_empty())
        .unwrap_or("workspace");
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let hash = hasher.finish();
    let slug: String = leaf
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    format!(
        "{}-{:08x}",
        if slug.is_empty() { "workspace" } else { &slug },
        hash & 0xffff_ffff
    )
}

pub fn memory_dir(app_config_dir: &Path, workspace_root: &str) -> PathBuf {
    app_config_dir
        .join(MEMORY_DIR_NAME)
        .join(project_key(workspace_root))
}

pub fn normalize_kind(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "user" => "user",
        "feedback" => "feedback",
        "reference" => "reference",
        _ => "project",
    }
}

pub fn slugify(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_lowercase().next().unwrap_or(ch)
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(64)
        .collect();
    if slug.is_empty() {
        format!("memory-{}", crate::native::artifacts::unix_millis())
    } else {
        slug
    }
}

fn cap_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max).collect();
    format!("{prefix}…")
}

fn parse_frontmatter(text: &str) -> (Vec<(String, String)>, String) {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (Vec::new(), text.to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), text.to_string());
    };
    let header = &rest[..end];
    let body = rest[end + 4..].trim_matches('\n').to_string();
    let fields = header
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    (fields, body)
}

fn render_entry(entry: &MemoryEntry) -> String {
    format!(
        "---\nname: {}\ndescription: {}\ntype: {}\ncreated_at: {}\nupdated_at: {}\n---\n{}\n",
        entry.name.replace('\n', " "),
        entry.description.replace('\n', " "),
        entry.kind,
        entry.created_at,
        entry.updated_at,
        entry.body.trim()
    )
}

pub fn read_entry(dir: &Path, file_name: &str) -> Option<MemoryEntry> {
    if !file_name.ends_with(".md") || file_name == MEMORY_INDEX_FILE || file_name.contains('/') {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(file_name)).ok()?;
    let (fields, body) = parse_frontmatter(&text);
    let field = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let name = {
        let value = field("name");
        if value.is_empty() {
            file_name.trim_end_matches(".md").to_string()
        } else {
            value
        }
    };
    Some(MemoryEntry {
        file_name: file_name.to_string(),
        name,
        description: field("description"),
        kind: normalize_kind(&field("type")).to_string(),
        created_at: field("created_at"),
        updated_at: field("updated_at"),
        body,
    })
}

pub fn list_entries(dir: &Path) -> Vec<MemoryEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<MemoryEntry> = read
        .flatten()
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            read_entry(dir, &name)
        })
        .collect();
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.name.cmp(&b.name)));
    entries
}

/// 按事实文件重建索引（≤ 200 行）。
pub fn rebuild_index(dir: &Path) -> Result<String, String> {
    let entries = list_entries(dir);
    let mut lines = vec![
        "# MEMORY.md".to_string(),
        String::new(),
        "每行一条记忆：`- [名称](文件) — 一句话描述`。详情用 Read 打开对应文件。".to_string(),
        String::new(),
    ];
    for entry in entries.iter().take(MEMORY_INDEX_MAX_LINES) {
        lines.push(format!(
            "- [{}]({}) — {} ({})",
            entry.name,
            entry.file_name,
            if entry.description.is_empty() {
                "（无描述）"
            } else {
                &entry.description
            },
            entry.kind
        ));
    }
    let text = format!("{}\n", lines.join("\n"));
    std::fs::create_dir_all(dir).map_err(|error| format!("创建记忆目录失败: {error}"))?;
    std::fs::write(dir.join(MEMORY_INDEX_FILE), &text)
        .map_err(|error| format!("写入 MEMORY.md 失败: {error}"))?;
    Ok(text)
}

pub fn load_index(dir: &Path) -> String {
    let path = dir.join(MEMORY_INDEX_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    let capped_lines: Vec<&str> = text.lines().take(MEMORY_INDEX_MAX_LINES + 8).collect();
    cap_chars(&capped_lines.join("\n"), MEMORY_INDEX_MAX_CHARS)
}

/// 新建或更新一条记忆并刷新索引。
pub fn save_entry(
    dir: &Path,
    name: &str,
    kind: &str,
    description: &str,
    body: &str,
) -> Result<MemoryEntry, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("记忆名称不能为空".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|error| format!("创建记忆目录失败: {error}"))?;
    let file_name = format!("{}.md", slugify(name));
    let now = now_sqlite();
    let existing = read_entry(dir, &file_name);
    let entry = MemoryEntry {
        file_name: file_name.clone(),
        name: name.to_string(),
        description: cap_chars(description.trim(), 200),
        kind: normalize_kind(kind).to_string(),
        created_at: existing
            .as_ref()
            .map(|item| item.created_at.clone())
            .filter(|item| !item.is_empty())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
        body: cap_chars(body.trim(), MEMORY_BODY_MAX_CHARS),
    };
    std::fs::write(dir.join(&file_name), render_entry(&entry))
        .map_err(|error| format!("写入记忆文件失败: {error}"))?;
    rebuild_index(dir)?;
    Ok(entry)
}

pub fn delete_entry(dir: &Path, file_name: &str) -> Result<bool, String> {
    if read_entry(dir, file_name).is_none() {
        return Ok(false);
    }
    std::fs::remove_file(dir.join(file_name)).map_err(|error| format!("删除记忆失败: {error}"))?;
    rebuild_index(dir)?;
    Ok(true)
}

fn load_state(dir: &Path) -> MemoryState {
    std::fs::read_to_string(dir.join(MEMORY_STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_state(dir: &Path, state: &MemoryState) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join(MEMORY_STATE_FILE), json);
    }
}

/// 查询词：ASCII 词（≥ 2 字符）+ CJK 双字。
fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut ascii = String::new();
    let chars: Vec<char> = query.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_ascii_alphanumeric() || *ch == '_' {
            ascii.push(ch.to_ascii_lowercase());
            continue;
        }
        if ascii.len() >= 2 {
            terms.push(std::mem::take(&mut ascii));
        } else {
            ascii.clear();
        }
        if is_cjk(*ch) {
            if let Some(next) = chars.get(index + 1).filter(|next| is_cjk(**next)) {
                terms.push(format!("{ch}{next}"));
            }
        }
    }
    if ascii.len() >= 2 {
        terms.push(ascii);
    }
    terms.sort();
    terms.dedup();
    terms
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

/// 关键词回忆：名称命中 ×3、描述 ×2、正文 ×1。
pub fn recall(dir: &Path, query: &str, limit: usize) -> Vec<MemoryHit> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<MemoryHit> = list_entries(dir)
        .into_iter()
        .filter_map(|entry| {
            let name = entry.name.to_lowercase();
            let description = entry.description.to_lowercase();
            let body = entry.body.to_lowercase();
            let mut score = 0u32;
            for term in &terms {
                if name.contains(term.as_str()) {
                    score += 3;
                }
                if description.contains(term.as_str()) {
                    score += 2;
                }
                if body.contains(term.as_str()) {
                    score += 1;
                }
            }
            (score > 0).then_some(MemoryHit {
                file_name: entry.file_name,
                name: entry.name,
                description: entry.description,
                score,
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
    hits.truncate(limit.max(1));
    hits
}

/// 回忆块：附在用户消息后，提示模型可用 Read 查看详情。
pub fn format_recall_block(dir: &Path, hits: &[MemoryHit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut lines = vec![format!(
        "[记忆回忆] 与本次输入相关的已保存记忆（目录 {}），需要细节时用 Read 打开：",
        dir.display()
    )];
    for hit in hits {
        let entry = read_entry(dir, &hit.file_name);
        let preview = entry
            .map(|item| cap_chars(&item.body.replace('\n', " "), 240))
            .unwrap_or_default();
        lines.push(format!(
            "- {} — {}{}",
            hit.name,
            if hit.description.is_empty() {
                hit.file_name.clone()
            } else {
                hit.description.clone()
            },
            if preview.is_empty() {
                String::new()
            } else {
                format!("｜{preview}")
            }
        ));
    }
    lines.join("\n")
}

/// 系统提示里的记忆块：索引 + 维护约定。
pub fn memory_prompt_block(dir: &Path) -> String {
    let index = load_index(dir);
    let mut block = format!(
        "# 记忆（MEMORY.md）\n目录：{}\n约定：发现值得跨会话保留的信息（用户偏好、纠正过的做法、项目决策、外部参考）时，用 Write 在该目录新建一个带 frontmatter（name / description / type: user|feedback|project|reference）的 `.md` 文件，并在 MEMORY.md 追加一行 `- [名称](文件) — 描述`；不要把一次性任务细节写进记忆。回答前如索引里有相关条目，先用 Read 查看对应文件。",
        dir.display()
    );
    if index.trim().is_empty() {
        block.push_str("\n（当前还没有记忆条目）");
    } else {
        block.push_str("\n\n");
        block.push_str(index.trim());
    }
    block
}

fn transcript_digest(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        let label = match message.role {
            Role::User => "用户",
            Role::Assistant => "助手",
            _ => continue,
        };
        let text = message.content.trim();
        if text.is_empty() || text.starts_with("[记忆回忆]") {
            continue;
        }
        parts.push(format!("{label}：{}", cap_chars(text, 1_500)));
    }
    let joined = parts.join("\n\n");
    if joined.chars().count() <= EXTRACT_TRANSCRIPT_CHARS {
        return joined;
    }
    // 保留尾部：越晚的对话越接近最终决策。
    let tail: String = joined
        .chars()
        .rev()
        .take(EXTRACT_TRANSCRIPT_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{tail}")
}

fn parse_entries_json(text: &str) -> Vec<(String, String, String, String)> {
    let trimmed = text.trim();
    let candidate = if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        &trimmed[start..=end]
    } else {
        trimmed
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(candidate) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let field = |key: &str| {
                item.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            let body = field("body");
            if body.is_empty() {
                return None;
            }
            Some((name, field("type"), field("description"), body))
        })
        .collect()
}

fn lite_client(client: &ModelClient, operation: &str, role: &str) -> ModelClient {
    match client.call_log_context() {
        Some(context) => client.clone().with_call_log_context(
            context
                .clone()
                .with_operation(operation)
                .with_model_role(role),
        ),
        None => client.clone(),
    }
}

fn pick_model<'a>(main_model: &'a str, lite_model: Option<&'a str>) -> (&'a str, &'static str) {
    match lite_model.map(str::trim).filter(|item| !item.is_empty()) {
        Some(lite) => (lite, MODEL_ROLE_LITE),
        None => (main_model, MODEL_ROLE_MAIN),
    }
}

/// 会话结束后抽取记忆。返回新增 / 更新的条目数。
pub async fn extract_memories(
    client: &ModelClient,
    main_model: &str,
    lite_model: Option<&str>,
    messages: &[Message],
    dir: &Path,
) -> Result<usize, String> {
    let digest = transcript_digest(messages);
    if digest.trim().is_empty() {
        return Ok(0);
    }
    let (model, role) = pick_model(main_model, lite_model);
    let existing = list_entries(dir);
    let existing_names: Vec<String> = existing.iter().map(|item| item.name.clone()).collect();
    let prompt = vec![
        Message::system(
            "你从编程助手的会话里抽取值得跨会话保留的记忆。只保留四类：user（用户身份、偏好、习惯）、feedback（用户纠正过的做法、明确的要求）、project（项目决策、约定、架构事实）、reference（外部资料、链接、命令）。忽略一次性任务细节、临时状态、可从仓库直接读到的事实。每条 body 不超过 300 字。只输出 JSON 数组：[{\"name\":\"简短名称\",\"type\":\"user|feedback|project|reference\",\"description\":\"一句话\",\"body\":\"正文\"}]，没有可保留的就输出 []。",
        ),
        Message::user(format!(
            "已有记忆名称（重复的请沿用相同名称以便更新）：{}\n\n会话摘录：\n{digest}",
            if existing_names.is_empty() {
                "（无）".to_string()
            } else {
                existing_names.join("、")
            }
        )),
    ];
    let (message, _usage) = lite_client(client, OPERATION_MEMORY_EXTRACT, role)
        .chat(ChatRequest {
            messages: &prompt,
            tools: &[],
            model,
            effort: None,
            max_output_tokens: Some(2_048),
            thinking_enabled: false,
        })
        .await?;
    let mut saved = 0;
    for (name, kind, description, body) in parse_entries_json(&message.content)
        .into_iter()
        .take(EXTRACT_MAX_ENTRIES)
    {
        let file_name = format!("{}.md", slugify(&name));
        let unchanged =
            read_entry(dir, &file_name).is_some_and(|item| item.body.trim() == body.trim());
        if unchanged {
            continue;
        }
        save_entry(dir, &name, &kind, &description, &body)?;
        saved += 1;
    }
    let mut state = load_state(dir);
    state.extractions = state.extractions.saturating_add(1);
    state.last_extracted_at = Some(now_sqlite());
    save_state(dir, &state);
    Ok(saved)
}

/// 是否到了该做 dream 的时候：每 `interval` 次抽取一次；`0` 表示从不。
pub fn dream_due(dir: &Path, interval: u32) -> bool {
    if interval == 0 {
        return false;
    }
    let state = load_state(dir);
    state.extractions > 0
        && state.extractions.is_multiple_of(interval)
        && !list_entries(dir).is_empty()
}

/// dream：把全部记忆交给模型合并、去重、丢弃过时项，然后按结果重写目录。
pub async fn dream(
    client: &ModelClient,
    main_model: &str,
    lite_model: Option<&str>,
    dir: &Path,
) -> Result<String, String> {
    let entries = list_entries(dir);
    if entries.is_empty() {
        return Ok("没有记忆可整理".to_string());
    }
    let (model, role) = pick_model(main_model, lite_model);
    let dump: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "### {} [{}]\n描述：{}\n{}",
                entry.name,
                entry.kind,
                entry.description,
                cap_chars(&entry.body, 1_200)
            )
        })
        .collect();
    let prompt = vec![
        Message::system(
            "你整理编程助手的长期记忆：合并重复项、删除互相矛盾里过时的一方、去掉一次性细节、让描述更精确。保持四类 type 不变。只输出 JSON 数组 [{\"name\",\"type\",\"description\",\"body\"}]，条目数不超过 60；输出即为整理后的全部记忆，未包含的条目会被删除。",
        ),
        Message::user(format!("当前记忆：\n\n{}", dump.join("\n\n"))),
    ];
    let (message, _usage) = lite_client(client, OPERATION_MEMORY_DREAM, role)
        .chat(ChatRequest {
            messages: &prompt,
            tools: &[],
            model,
            effort: None,
            max_output_tokens: Some(8_192),
            thinking_enabled: false,
        })
        .await?;
    let parsed = parse_entries_json(&message.content);
    if parsed.is_empty() {
        return Err("模型没有返回可用的整理结果，记忆保持不变".to_string());
    }
    let keep: Vec<String> = parsed
        .iter()
        .take(DREAM_MAX_ENTRIES)
        .map(|(name, _, _, _)| format!("{}.md", slugify(name)))
        .collect();
    let before = entries.len();
    for entry in &entries {
        if !keep.contains(&entry.file_name) {
            let _ = std::fs::remove_file(dir.join(&entry.file_name));
        }
    }
    for (name, kind, description, body) in parsed.into_iter().take(DREAM_MAX_ENTRIES) {
        save_entry(dir, &name, &kind, &description, &body)?;
    }
    rebuild_index(dir)?;
    let mut state = load_state(dir);
    state.dreams = state.dreams.saturating_add(1);
    state.last_dreamed_at = Some(now_sqlite());
    save_state(dir, &state);
    Ok(format!("记忆整理完成：{before} → {} 条", keep.len()))
}

pub fn memory_view(dir: &Path) -> NativeMemoryView {
    let state = load_state(dir);
    NativeMemoryView {
        dir: dir.to_string_lossy().into_owned(),
        index: load_index(dir),
        entries: list_entries(dir),
        extractions: state.extractions,
        dreams: state.dreams,
    }
}

async fn resolve_memory_dir<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: &str,
) -> Result<PathBuf, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let pool = crate::app::shared::sqlite_pool(app).await?;
    let context =
        crate::engine::context::resolve_workspace_execution_context_with_pool(&pool, workspace_id)
            .await?;
    let root = context
        .working_dir
        .ok_or_else(|| "工作区缺少目录".to_string())?;
    Ok(memory_dir(&config_dir, &root))
}

#[tauri::command]
pub async fn list_native_memories<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<NativeMemoryView, String> {
    let dir = resolve_memory_dir(&app, &workspace_id).await?;
    Ok(memory_view(&dir))
}

#[tauri::command]
pub async fn save_native_memory<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    name: String,
    kind: String,
    description: String,
    body: String,
) -> Result<MemoryEntry, String> {
    let dir = resolve_memory_dir(&app, &workspace_id).await?;
    save_entry(&dir, &name, &kind, &description, &body)
}

#[tauri::command]
pub async fn delete_native_memory<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    file_name: String,
) -> Result<bool, String> {
    let dir = resolve_memory_dir(&app, &workspace_id).await?;
    delete_entry(&dir, &file_name)
}

#[tauri::command]
pub async fn open_native_memory_dir<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<String, String> {
    let dir = resolve_memory_dir(&app, &workspace_id).await?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建记忆目录失败: {error}"))?;
    if !dir.join(MEMORY_INDEX_FILE).exists() {
        rebuild_index(&dir)?;
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| format!("打开记忆目录失败: {error}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "noxcode-memory-{}",
            crate::native::artifacts::unique_suffix()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn project_key_is_stable_and_safe() {
        let a = project_key("/Users/me/Projects/My App");
        let b = project_key("/Users/me/Projects/My App/");
        assert_eq!(a, b);
        assert!(a.starts_with("my-app-"));
        assert_ne!(a, project_key("/Users/me/Projects/other"));
        assert!(project_key("").starts_with("workspace-"));
    }

    #[test]
    fn save_read_recall_and_delete_round_trip() {
        let dir = temp_dir();
        let entry = save_entry(
            &dir,
            "偏好：用 pnpm",
            "user",
            "用户偏好 pnpm 而不是 npm",
            "所有安装命令用 pnpm；CI 也是 pnpm。",
        )
        .expect("save");
        assert_eq!(entry.kind, "user");
        assert!(dir.join(&entry.file_name).exists());
        let index = load_index(&dir);
        assert!(index.contains("偏好：用 pnpm"));
        assert!(index.contains(&entry.file_name));
        save_entry(
            &dir,
            "Deploy notes",
            "project",
            "部署流程",
            "用 `make deploy`。",
        )
        .expect("save 2");
        let hits = recall(&dir, "请用 pnpm 安装依赖", 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "偏好：用 pnpm");
        let block = format_recall_block(&dir, &hits);
        assert!(block.starts_with("[记忆回忆]"));
        assert!(block.contains("pnpm"));
        assert!(recall(&dir, "", 3).is_empty());
        let english = recall(&dir, "how do we deploy?", 3);
        assert_eq!(english[0].name, "Deploy notes");
        let reread = read_entry(&dir, &entry.file_name).expect("read");
        assert_eq!(reread.body, "所有安装命令用 pnpm；CI 也是 pnpm。");
        let updated =
            save_entry(&dir, "偏好：用 pnpm", "feedback", "改", "新正文").expect("update");
        assert_eq!(updated.created_at, entry.created_at);
        assert_eq!(list_entries(&dir).len(), 2);
        assert!(delete_entry(&dir, &entry.file_name).expect("delete"));
        assert!(!delete_entry(&dir, "missing.md").expect("delete missing"));
        assert!(!load_index(&dir).contains("偏好：用 pnpm"));
        let prompt = memory_prompt_block(&dir);
        assert!(prompt.contains("# 记忆（MEMORY.md）"));
        assert!(prompt.contains("Deploy notes"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_model_json_and_query_terms() {
        let parsed = parse_entries_json(
            "结果如下：[{\"name\":\"A\",\"type\":\"project\",\"description\":\"d\",\"body\":\"b\"},{\"name\":\"\",\"body\":\"x\"},{\"name\":\"C\",\"body\":\"\"}] 完",
        );
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "A");
        assert!(parse_entries_json("not json").is_empty());
        let terms = query_terms("用 pnpm 安装 React 依赖");
        assert!(terms.contains(&"pnpm".to_string()));
        assert!(terms.contains(&"react".to_string()));
        assert!(terms.contains(&"安装".to_string()));
        assert!(!terms.contains(&"用".to_string()));
        assert_eq!(normalize_kind("Feedback"), "feedback");
        assert_eq!(normalize_kind("weird"), "project");
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }

    #[test]
    fn dream_due_follows_interval_state() {
        let dir = temp_dir();
        assert!(!dream_due(&dir, 5));
        save_entry(&dir, "x", "project", "d", "b").expect("save");
        save_state(
            &dir,
            &MemoryState {
                extractions: 10,
                ..MemoryState::default()
            },
        );
        assert!(dream_due(&dir, 5));
        assert!(!dream_due(&dir, 3));
        assert!(!dream_due(&dir, 0));
        let _ = std::fs::remove_dir_all(dir);
    }
}
