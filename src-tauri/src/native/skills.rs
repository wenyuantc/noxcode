#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::native::commands::{parse_frontmatter, parse_list_field};
use crate::native::plugins::{load_enabled_plugins, plugin_skill_dirs, NativePlugin};
use crate::native::tools::ssh::SshToolRuntime;

pub const LEGACY_GLOBAL_SKILLS_DIR_NAME: &str = "native-skills";
pub const SKILLS_STATE_FILE: &str = "native-skills-state.json";
pub const MAX_SKILLS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    WorkspaceNoxcode,
    WorkspaceZcode,
    WorkspaceAgents,
    WorkspaceClaude,
    Plugin,
    Global,
}

impl SkillSource {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::WorkspaceNoxcode => "工作区 .noxcode",
            Self::WorkspaceZcode => "工作区 .zcode",
            Self::WorkspaceAgents => "工作区 .agents",
            Self::WorkspaceClaude => "工作区 .claude",
            Self::Plugin => "插件",
            Self::Global => "全局",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::WorkspaceNoxcode => 0,
            Self::WorkspaceZcode => 1,
            Self::WorkspaceAgents => 2,
            Self::WorkspaceClaude => 3,
            Self::Plugin => 4,
            Self::Global => 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSkill {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub dir: String,
    pub skill_md_path: String,
    pub body: String,
    pub extra_files: Vec<String>,
    /// frontmatter `allowed-tools`：加载技能后建议限定的工具集（提示模型，不做硬限制）。
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// frontmatter `argument-hint`：`/skill <name> <参数>` 的参数提示。
    #[serde(default)]
    pub argument_hint: Option<String>,
    /// frontmatter `when_to_use`：何时应主动使用该技能。
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// 来自插件时的插件名。
    #[serde(default)]
    pub plugin: Option<String>,
}

/// 从 SKILL.md 解析出的元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
    pub argument_hint: Option<String>,
    pub when_to_use: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDiagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeGlobalSkills {
    pub dir: String,
    pub skills: Vec<NativeSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeSkillsView {
    pub global_dir: String,
    pub workspace_root: Option<String>,
    pub skills: Vec<NativeSkill>,
    pub disabled_paths: Vec<String>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SkillsState {
    #[serde(default)]
    disabled_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateNativeSkillInput {
    pub scope: String,
    pub name: String,
    pub description: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportExternalSkillsInput {
    pub workspace_id: Option<String>,
    pub target: String,
    pub mode: String,
    pub items: Vec<ImportExternalSkillItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportExternalSkillItem {
    pub source_path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSkillScan {
    pub groups: Vec<ExternalSkillGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSkillGroup {
    pub id: String,
    pub label: String,
    pub scope: String,
    pub skills: Vec<ExternalSkillItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalSkillItem {
    pub name: String,
    pub description: String,
    pub source_path: String,
    pub importable: bool,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportExternalSkillsResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

pub fn parse_skill_meta(raw: &str, fallback_name: &str) -> SkillMeta {
    let (fields, body) = parse_frontmatter(raw);
    let mut name = fields.get("name").cloned().unwrap_or_default();
    if name.trim().is_empty() {
        name = fallback_name.to_string();
    }
    let mut description = fields.get("description").cloned().unwrap_or_default();
    if description.trim().is_empty() {
        description = first_non_empty_line(if fields.is_empty() { raw } else { &body });
    }
    let non_empty = |value: Option<&String>| {
        value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    SkillMeta {
        name: name.trim().to_string(),
        description: truncate_description(&description),
        allowed_tools: fields
            .get("allowed-tools")
            .map(|value| parse_list_field(value))
            .unwrap_or_default(),
        argument_hint: non_empty(fields.get("argument-hint")),
        when_to_use: non_empty(fields.get("when-to-use")),
    }
}

pub fn parse_skill_markdown(raw: &str, fallback_name: &str) -> (String, String) {
    let meta = parse_skill_meta(raw, fallback_name);
    (meta.name, meta.description)
}

pub fn user_global_skills_dir() -> Result<PathBuf, String> {
    crate::app::ssh::shell::user_home_dir()
        .map(|home| home.join(".noxcode").join("skills"))
        .ok_or_else(|| "无法解析用户主目录".to_string())
}

pub fn ensure_user_global_skills_dir() -> Result<PathBuf, String> {
    let dir = user_global_skills_dir()?;
    fs::create_dir_all(&dir).map_err(|error| format!("创建全局技能目录失败: {error}"))?;
    Ok(dir)
}

pub fn validate_skill_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("技能名不能为空".to_string());
    }
    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if !valid || name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err("技能名只能包含小写字母、数字和连字符".to_string());
    }
    Ok(name.to_string())
}

pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => left == right,
    }
}

fn collect_workspace_skill_roots(root: &Path, out: &mut Vec<NativeSkill>) {
    collect_local_skill_dir(
        root.join(".noxcode/skills"),
        SkillSource::WorkspaceNoxcode,
        None,
        out,
    );
    collect_local_skill_dir(
        root.join(".zcode/skills"),
        SkillSource::WorkspaceZcode,
        None,
        out,
    );
    collect_local_skill_dir(
        root.join(".agents/skills"),
        SkillSource::WorkspaceAgents,
        None,
        out,
    );
    collect_local_skill_dir(
        root.join(".claude/skills"),
        SkillSource::WorkspaceClaude,
        None,
        out,
    );
}

pub fn discover_local_workspace_skills(cwd: &str) -> Vec<NativeSkill> {
    let root = Path::new(cwd);
    let mut out = Vec::new();
    collect_workspace_skill_roots(root, &mut out);
    if let Some(git_root) = find_git_root(root) {
        if !same_path(&git_root, root) {
            collect_workspace_skill_roots(&git_root, &mut out);
        }
    }
    out
}

pub fn discover_global_skills_from(
    primary: Option<&Path>,
    legacy: Option<&Path>,
) -> Vec<NativeSkill> {
    let mut out = Vec::new();
    if let Some(dir) = primary {
        collect_local_skill_dir(dir.to_path_buf(), SkillSource::Global, None, &mut out);
    }
    if let Some(dir) = legacy {
        collect_local_skill_dir(dir.to_path_buf(), SkillSource::Global, None, &mut out);
    }
    out
}

pub fn discover_global_skills(legacy_config_dir: Option<&Path>) -> Vec<NativeSkill> {
    let primary = user_global_skills_dir().ok();
    let legacy = legacy_config_dir.map(|dir| dir.join(LEGACY_GLOBAL_SKILLS_DIR_NAME));
    discover_global_skills_from(primary.as_deref(), legacy.as_deref())
}

/// 已启用插件贡献的技能目录。
pub fn discover_plugin_skills(plugins: &[NativePlugin]) -> Vec<NativeSkill> {
    let mut out = Vec::new();
    for (plugin, dir) in plugin_skill_dirs(plugins) {
        collect_local_skill_dir(dir, SkillSource::Plugin, Some(&plugin), &mut out);
    }
    out
}

pub async fn load_session_skills(
    cwd: &str,
    ssh: Option<&SshToolRuntime>,
    config_dir: Option<&Path>,
    plugins: &[NativePlugin],
) -> Vec<NativeSkill> {
    let mut items = if let Some(ssh) = ssh {
        discover_ssh_workspace_skills(ssh).await
    } else {
        discover_local_workspace_skills(cwd)
    };
    items.extend(discover_plugin_skills(plugins));
    items.extend(discover_global_skills(config_dir));
    let merged = merge_skills(items);
    filter_disabled_skills(&merged, config_dir)
}

pub fn merge_skills(items: Vec<NativeSkill>) -> Vec<NativeSkill> {
    merge_skills_detailed(items).0
}

pub fn merge_skills_detailed(
    mut items: Vec<NativeSkill>,
) -> (Vec<NativeSkill>, Vec<SkillDiagnostic>) {
    items.sort_by(|left, right| {
        left.source.rank().cmp(&right.source.rank()).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    let mut diagnostics = Vec::new();
    for item in items {
        let key = item.name.to_ascii_lowercase();
        if let Some(winner) = seen.get(&key) {
            diagnostics.push(SkillDiagnostic {
                code: "skill_duplicate_name".to_string(),
                path: item.skill_md_path.clone(),
                message: format!("同名技能已被忽略（已加载 {winner}）"),
                level: "warning".to_string(),
            });
            continue;
        }
        if item.description.trim().is_empty() {
            diagnostics.push(SkillDiagnostic {
                code: "skill_missing_description".to_string(),
                path: item.skill_md_path.clone(),
                message: "frontmatter 缺少 description 字段".to_string(),
                level: "warning".to_string(),
            });
        }
        seen.insert(key, item.skill_md_path.clone());
        out.push(item);
        if out.len() >= MAX_SKILLS {
            break;
        }
    }
    (out, diagnostics)
}

pub fn format_skills_prompt(skills: &[NativeSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "# 可用技能".to_string(),
        "需要完整说明或附属文件时调用 Skill 工具（参数 name）。不要编造未列出的技能。".to_string(),
    ];
    for skill in skills {
        let mut line = format!(
            "- `{}`：{}（{}）",
            skill.name,
            skill.description,
            skill.source.label_zh()
        );
        if let Some(when) = skill.when_to_use.as_deref() {
            line.push_str(&format!(" 何时使用：{when}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}

pub fn render_skill(skill: &NativeSkill, ssh_session: bool) -> String {
    let mut extra = if skill.extra_files.is_empty() {
        "(无附属文件)".to_string()
    } else {
        skill
            .extra_files
            .iter()
            .map(|path| format!("- {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if ssh_session && skill.source == SkillSource::Global {
        extra.push_str(
            "\n\n附属文件不在远端工作区；不要用 Read 读取这些本机路径，继续用 Skill 查看。",
        );
    }
    let mut header = format!(
        "# {}（{}）\n目录: {}",
        skill.name,
        skill.source.label_zh(),
        skill.dir
    );
    if !skill.allowed_tools.is_empty() {
        header.push_str(&format!(
            "\n建议工具（allowed-tools）: {}",
            skill.allowed_tools.join(", ")
        ));
    }
    if let Some(hint) = skill.argument_hint.as_deref() {
        header.push_str(&format!("\n参数提示: {hint}"));
    }
    format!(
        "{header}\n\n## SKILL.md\n{}\n\n## 目录文件\n{extra}",
        skill.body.trim()
    )
}

pub fn find_skill<'a>(skills: &'a [NativeSkill], name: &str) -> Result<&'a NativeSkill, String> {
    let needle = name.trim();
    if needle.is_empty() {
        return Err("name 不能为空".to_string());
    }
    skills
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(needle))
        .ok_or_else(|| format!("未找到技能：{needle}"))
}

pub async fn discover_ssh_workspace_skills(ssh: &SshToolRuntime) -> Vec<NativeSkill> {
    let listing = ssh
        .bash(
            "find .noxcode/skills .zcode/skills .agents/skills .claude/skills -mindepth 2 -maxdepth 2 -name SKILL.md 2>/dev/null | head -n 80",
        )
        .await
        .unwrap_or_default();
    if listing.trim().is_empty() || listing.trim() == "(no output)" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in listing.lines() {
        let path = line.trim().trim_start_matches("./");
        if path.is_empty() {
            continue;
        }
        let source = if path.starts_with(".noxcode/") {
            SkillSource::WorkspaceNoxcode
        } else if path.starts_with(".zcode/") {
            SkillSource::WorkspaceZcode
        } else if path.starts_with(".agents/") {
            SkillSource::WorkspaceAgents
        } else if path.starts_with(".claude/") {
            SkillSource::WorkspaceClaude
        } else {
            continue;
        };
        let Ok(body) = ssh.read(path).await else {
            continue;
        };
        if body.trim().is_empty() || body.trim() == "(no output)" {
            continue;
        }
        let dir = path
            .rsplit_once('/')
            .map(|(left, _)| left.to_string())
            .unwrap_or_else(|| path.to_string());
        let fallback = dir
            .rsplit_once('/')
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| dir.clone());
        let meta = parse_skill_meta(&body, &fallback);
        let extra = ssh_list_extra_files(ssh, &dir).await;
        out.push(NativeSkill {
            name: meta.name,
            description: meta.description,
            source,
            dir,
            skill_md_path: path.to_string(),
            body,
            extra_files: extra,
            allowed_tools: meta.allowed_tools,
            argument_hint: meta.argument_hint,
            when_to_use: meta.when_to_use,
            plugin: None,
        });
    }
    out
}

async fn ssh_list_extra_files(ssh: &SshToolRuntime, dir: &str) -> Vec<String> {
    let listing = ssh
        .bash(&format!(
            "find {} -maxdepth 2 -type f 2>/dev/null | head -n 40",
            crate::app::ssh::shell::shell_escape_single_quoted(dir)
        ))
        .await
        .unwrap_or_default();
    if listing.trim().is_empty() || listing.trim() == "(no output)" {
        return Vec::new();
    }
    listing
        .lines()
        .map(|line| line.trim().trim_start_matches("./").to_string())
        .filter(|path| {
            !path.is_empty()
                && path != "(no output)"
                && !path.ends_with("/SKILL.md")
                && path != "SKILL.md"
        })
        .take(20)
        .collect()
}

fn collect_local_skill_dir(
    dir: PathBuf,
    source: SkillSource,
    plugin: Option<&str>,
    out: &mut Vec<NativeSkill>,
) {
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for skill_dir in dirs {
        let skill_md = skill_dir.join("SKILL.md");
        let Ok(body) = fs::read_to_string(&skill_md) else {
            continue;
        };
        if body.trim().is_empty() {
            continue;
        }
        let fallback = skill_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("skill");
        let meta = parse_skill_meta(&body, fallback);
        out.push(NativeSkill {
            name: meta.name,
            description: meta.description,
            source,
            dir: skill_dir.to_string_lossy().into_owned(),
            skill_md_path: skill_md.to_string_lossy().into_owned(),
            body,
            extra_files: list_local_extra_files(&skill_dir),
            allowed_tools: meta.allowed_tools,
            argument_hint: meta.argument_hint,
            when_to_use: meta.when_to_use,
            plugin: plugin.map(str::to_string),
        });
    }
}

fn list_local_extra_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_extra(dir, dir, &mut files);
    files.sort();
    files.truncate(20);
    files
}

fn walk_extra(root: &Path, current: &Path, out: &mut Vec<String>) {
    if out.len() >= 20 {
        return;
    }
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_extra(root, &path, out);
            continue;
        }
        if entry.file_name() == "SKILL.md" {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
        if out.len() >= 20 {
            return;
        }
    }
}

fn first_non_empty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "---")
        .unwrap_or("")
        .to_string()
}

fn truncate_description(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= MAX_DESCRIPTION_CHARS {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(MAX_DESCRIPTION_CHARS).collect();
    format!("{prefix}…")
}

fn skills_state_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SKILLS_STATE_FILE)
}

fn load_skills_state(config_dir: Option<&Path>) -> SkillsState {
    let Some(config_dir) = config_dir else {
        return SkillsState::default();
    };
    let path = skills_state_path(config_dir);
    let Ok(raw) = fs::read_to_string(path) else {
        return SkillsState::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_skills_state(config_dir: &Path, state: &SkillsState) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|error| format!("写入技能状态失败: {error}"))?;
    let raw = serde_json::to_string_pretty(state)
        .map_err(|error| format!("序列化技能状态失败: {error}"))?;
    fs::write(skills_state_path(config_dir), raw)
        .map_err(|error| format!("写入技能状态失败: {error}"))
}

fn normalize_skill_path(path: &str) -> String {
    PathBuf::from(path.trim())
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_disabled_path(disabled: &[String], skill_md_path: &str) -> bool {
    let needle = normalize_skill_path(skill_md_path);
    disabled
        .iter()
        .any(|item| normalize_skill_path(item) == needle)
}

fn filter_disabled_skills(skills: &[NativeSkill], config_dir: Option<&Path>) -> Vec<NativeSkill> {
    let state = load_skills_state(config_dir);
    skills
        .iter()
        .filter(|skill| !is_disabled_path(&state.disabled_paths, &skill.skill_md_path))
        .cloned()
        .collect()
}

fn existing_skill_names(dirs: &[&Path]) -> HashSet<String> {
    let mut names = HashSet::new();
    for dir in dirs {
        let mut found = Vec::new();
        collect_local_skill_dir(dir.to_path_buf(), SkillSource::Global, None, &mut found);
        for skill in found {
            names.insert(skill.name.to_ascii_lowercase());
        }
    }
    names
}

fn is_within_dir(root: &Path, path: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    path.starts_with(root)
}

fn managed_skill_dir(path: &Path, workspace_root: Option<&Path>) -> Result<PathBuf, String> {
    let skill_dir = if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        path.parent()
            .ok_or_else(|| "无法解析技能目录".to_string())?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    let global = user_global_skills_dir()?;
    if is_within_dir(&global, &skill_dir) {
        return Ok(skill_dir);
    }
    if let Some(workspace) = workspace_root {
        let project = workspace.join(".noxcode").join("skills");
        if is_within_dir(&project, &skill_dir) {
            return Ok(skill_dir);
        }
    }
    Err("只能修改 ~/.noxcode/skills 或工作区 .noxcode/skills 下的技能".to_string())
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| format!("复制技能失败: {error}"))?;
    let entries = fs::read_dir(from).map_err(|error| format!("读取技能目录失败: {error}"))?;
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).map_err(|error| format!("复制技能失败: {error}"))?;
        }
    }
    Ok(())
}

fn symlink_dir(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(from, to).map_err(|error| format!("创建技能软链失败: {error}"))
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(from, to)
            .map_err(|error| format!("创建技能软链失败: {error}"))
    }
}

fn skill_template(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\n{description}\n")
}

fn write_skill_dir(root: &Path, name: &str, description: &str) -> Result<NativeSkill, String> {
    let skill_dir = root.join(name);
    if skill_dir.exists() {
        return Err(format!("技能目录已存在：{name}"));
    }
    fs::create_dir_all(&skill_dir).map_err(|error| format!("创建技能目录失败: {error}"))?;
    let skill_md = skill_dir.join("SKILL.md");
    let body = skill_template(name, description);
    fs::write(&skill_md, &body).map_err(|error| format!("写入 SKILL.md 失败: {error}"))?;
    Ok(NativeSkill {
        name: name.to_string(),
        description: truncate_description(description),
        source: SkillSource::Global,
        dir: skill_dir.to_string_lossy().into_owned(),
        skill_md_path: skill_md.to_string_lossy().into_owned(),
        body,
        extra_files: Vec::new(),
        allowed_tools: Vec::new(),
        argument_hint: None,
        when_to_use: None,
        plugin: None,
    })
}

async fn workspace_root_for<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    crate::native::permission_rules::local_workspace_root(app, workspace_id).await
}

fn config_dir_of<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

fn collect_discovered_skills(
    workspace_root: Option<&Path>,
    config_dir: Option<&Path>,
) -> (Vec<NativeSkill>, Vec<SkillDiagnostic>) {
    let mut items = Vec::new();
    if let Some(root) = workspace_root {
        items.extend(discover_local_workspace_skills(&root.to_string_lossy()));
    }
    let plugins = load_enabled_plugins(config_dir, workspace_root);
    items.extend(discover_plugin_skills(&plugins));
    items.extend(discover_global_skills(config_dir));
    merge_skills_detailed(items)
}

fn open_path<R: Runtime>(app: &AppHandle<R>, dir: &Path) -> Result<(), String> {
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| format!("打开技能目录失败: {error}"))
}

#[tauri::command]
pub async fn list_native_global_skills<R: Runtime>(
    app: AppHandle<R>,
) -> Result<NativeGlobalSkills, String> {
    let dir = ensure_user_global_skills_dir()?;
    Ok(NativeGlobalSkills {
        dir: dir.to_string_lossy().into_owned(),
        skills: discover_global_skills(config_dir_of(&app).as_deref()),
    })
}

#[tauri::command]
pub async fn list_native_skills<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<NativeSkillsView, String> {
    let global_dir = ensure_user_global_skills_dir()?;
    let workspace_root = workspace_root_for(&app, workspace_id.as_deref()).await?;
    let config = config_dir_of(&app);
    let (skills, diagnostics) =
        collect_discovered_skills(workspace_root.as_deref(), config.as_deref());
    let state = load_skills_state(config.as_deref());
    Ok(NativeSkillsView {
        global_dir: global_dir.to_string_lossy().into_owned(),
        workspace_root: workspace_root.map(|path| path.to_string_lossy().into_owned()),
        skills,
        disabled_paths: state.disabled_paths,
        diagnostics,
    })
}

#[tauri::command]
pub async fn open_native_skills_dir<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = ensure_user_global_skills_dir()?;
    open_path(&app, &dir)
}

#[tauri::command]
pub async fn open_native_skill_path<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<(), String> {
    let path = PathBuf::from(path.trim());
    if !path.exists() {
        return Err("技能路径不存在".to_string());
    }
    let target = if path.is_file() {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    };
    open_path(&app, &target)
}

#[tauri::command]
pub async fn create_native_skill<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateNativeSkillInput,
) -> Result<NativeSkill, String> {
    let name = validate_skill_name(&payload.name)?;
    let description = payload.description.trim();
    if description.is_empty() {
        return Err("技能描述不能为空".to_string());
    }
    let scope = payload.scope.trim();
    let root = if scope == "project" {
        let workspace = workspace_root_for(&app, payload.workspace_id.as_deref())
            .await?
            .ok_or_else(|| "项目技能需要本地工作区".to_string())?;
        let dir = workspace.join(".noxcode").join("skills");
        fs::create_dir_all(&dir).map_err(|error| format!("创建项目技能目录失败: {error}"))?;
        dir
    } else if scope == "global" {
        ensure_user_global_skills_dir()?
    } else {
        return Err("scope 只能是 global 或 project".to_string());
    };
    let mut skill = write_skill_dir(&root, &name, description)?;
    skill.source = if scope == "project" {
        SkillSource::WorkspaceNoxcode
    } else {
        SkillSource::Global
    };
    Ok(skill)
}

#[tauri::command]
pub async fn delete_native_skill<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let workspace = workspace_root_for(&app, workspace_id.as_deref()).await?;
    let skill_dir = managed_skill_dir(Path::new(path.trim()), workspace.as_deref())?;
    fs::remove_dir_all(&skill_dir).map_err(|error| format!("删除技能失败: {error}"))
}

#[tauri::command]
pub async fn copy_native_skill_to_global<R: Runtime>(
    _app: AppHandle<R>,
    path: String,
) -> Result<NativeSkill, String> {
    let source = PathBuf::from(path.trim());
    let skill_dir = if source.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        source
            .parent()
            .ok_or_else(|| "无法解析技能目录".to_string())?
            .to_path_buf()
    } else {
        source
    };
    if !skill_dir.join("SKILL.md").exists() {
        return Err("未找到 SKILL.md".to_string());
    }
    let name = skill_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "无法解析技能名".to_string())?
        .to_string();
    let name = validate_skill_name(&name).unwrap_or(name);
    let global = ensure_user_global_skills_dir()?;
    let dest = global.join(&name);
    if dest.exists() {
        return Err(format!("全局已存在同名技能：{name}"));
    }
    copy_dir_recursive(&skill_dir, &dest)?;
    let mut found = Vec::new();
    collect_local_skill_dir(global, SkillSource::Global, None, &mut found);
    found
        .into_iter()
        .find(|skill| skill.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| "复制后未能读取技能".to_string())
}

#[tauri::command]
pub async fn set_native_skill_enabled<R: Runtime>(
    app: AppHandle<R>,
    skill_md_path: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    let config = config_dir_of(&app).ok_or_else(|| "无法读取应用配置目录".to_string())?;
    let mut state = load_skills_state(Some(&config));
    let path = normalize_skill_path(&skill_md_path);
    state
        .disabled_paths
        .retain(|item| normalize_skill_path(item) != path);
    if !enabled {
        state.disabled_paths.push(path);
    }
    save_skills_state(&config, &state)?;
    Ok(state.disabled_paths)
}

fn scan_external_dir(dir: &Path, existing: &HashSet<String>) -> Vec<ExternalSkillItem> {
    let mut found = Vec::new();
    collect_local_skill_dir(dir.to_path_buf(), SkillSource::Global, None, &mut found);
    found
        .into_iter()
        .map(|skill| {
            let exists = existing.contains(&skill.name.to_ascii_lowercase());
            ExternalSkillItem {
                name: skill.name,
                description: skill.description,
                source_path: skill.dir,
                importable: !exists,
                skip_reason: exists.then(|| "sameNameExists".to_string()),
            }
        })
        .collect()
}

#[tauri::command]
pub async fn scan_external_skills<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<ExternalSkillScan, String> {
    let workspace = workspace_root_for(&app, workspace_id.as_deref()).await?;
    let global = user_global_skills_dir().ok();
    let project = workspace
        .as_ref()
        .map(|root| root.join(".noxcode").join("skills"));
    let dirs: Vec<&Path> = [global.as_deref(), project.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let existing = existing_skill_names(&dirs);
    let home = crate::app::ssh::shell::user_home_dir();
    let mut groups = Vec::new();
    let user_sources = [
        ("claude_user", "Claude Code 用户", ".claude/skills"),
        ("agents_user", "Agents 用户", ".agents/skills"),
        ("codex_user", "Codex CLI 用户", ".codex/skills"),
        ("opencode_user", "OpenCode 用户", ".opencode/skills"),
        ("cursor_user", "Cursor 用户", ".cursor/skills"),
        ("zcode_user", "ZCode 用户", ".zcode/skills"),
    ];
    if let Some(home) = home.as_ref() {
        for (id, label, rel) in user_sources {
            let dir = home.join(rel);
            let skills = scan_external_dir(&dir, &existing);
            if !skills.is_empty() {
                groups.push(ExternalSkillGroup {
                    id: id.to_string(),
                    label: label.to_string(),
                    scope: "global".to_string(),
                    skills,
                });
            }
        }
    }
    if let Some(root) = workspace.as_ref() {
        let project_sources = [
            ("claude_project", "Claude Code 项目", ".claude/skills"),
            ("agents_project", "Agents 项目", ".agents/skills"),
            ("zcode_project", "ZCode 项目", ".zcode/skills"),
            ("cursor_project", "Cursor 项目", ".cursor/skills"),
        ];
        for (id, label, rel) in project_sources {
            let dir = root.join(rel);
            let skills = scan_external_dir(&dir, &existing);
            if !skills.is_empty() {
                groups.push(ExternalSkillGroup {
                    id: id.to_string(),
                    label: label.to_string(),
                    scope: "project".to_string(),
                    skills,
                });
            }
        }
    }
    Ok(ExternalSkillScan { groups })
}

#[tauri::command]
pub async fn import_external_skills<R: Runtime>(
    app: AppHandle<R>,
    payload: ImportExternalSkillsInput,
) -> Result<ImportExternalSkillsResult, String> {
    let target = payload.target.trim();
    let dest_root = if target == "project" {
        let workspace = workspace_root_for(&app, payload.workspace_id.as_deref())
            .await?
            .ok_or_else(|| "导入到项目需要本地工作区".to_string())?;
        let dir = workspace.join(".noxcode").join("skills");
        fs::create_dir_all(&dir).map_err(|error| format!("创建项目技能目录失败: {error}"))?;
        dir
    } else if target == "global" {
        ensure_user_global_skills_dir()?
    } else {
        return Err("target 只能是 global 或 project".to_string());
    };
    let mode = payload.mode.trim();
    if mode != "copy" && mode != "symlink" {
        return Err("mode 只能是 copy 或 symlink".to_string());
    }
    let existing = existing_skill_names(&[&dest_root]);
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    for item in payload.items {
        let name = match validate_skill_name(&item.name) {
            Ok(name) => name,
            Err(error) => {
                failed.push(format!("{}: {error}", item.name));
                continue;
            }
        };
        if existing.contains(&name.to_ascii_lowercase()) || dest_root.join(&name).exists() {
            skipped.push(name);
            continue;
        }
        let source = PathBuf::from(item.source_path.trim());
        let source_dir = if source.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            source.parent().map(Path::to_path_buf).unwrap_or(source)
        } else {
            source
        };
        if !source_dir.join("SKILL.md").exists() {
            failed.push(format!("{name}: 未找到 SKILL.md"));
            continue;
        }
        let dest = dest_root.join(&name);
        let result = if mode == "symlink" {
            symlink_dir(&source_dir, &dest)
        } else {
            copy_dir_recursive(&source_dir, &dest)
        };
        match result {
            Ok(()) => imported.push(name),
            Err(error) => failed.push(format!("{name}: {error}")),
        }
    }
    Ok(ImportExternalSkillsResult {
        imported,
        skipped,
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(prefix: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            crate::native::artifacts::unique_suffix()
        ));
        fs::create_dir_all(&root).expect("mkdir");
        root
    }

    fn write_skill(dir: &Path, name: &str, description: &str) {
        fs::create_dir_all(dir).expect("mkdir skill");
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nbody\n"),
        )
        .expect("write skill");
    }

    #[test]
    fn parse_frontmatter_and_fallback() {
        let (name, desc) = parse_skill_markdown(
            "---\nname: code-review\ndescription: \"Review diffs\"\n---\n# Hello\n",
            "folder",
        );
        assert_eq!(name, "code-review");
        assert_eq!(desc, "Review diffs");
        let (name, desc) = parse_skill_markdown("# just a title\n\nbody", "folder");
        assert_eq!(name, "folder");
        assert_eq!(desc, "# just a title");
    }

    #[test]
    fn user_global_dir_is_under_home_noxcode() {
        let dir = user_global_skills_dir().expect("home");
        assert!(dir.ends_with(Path::new(".noxcode").join("skills")));
    }

    #[test]
    fn validate_skill_name_rejects_invalid() {
        assert!(validate_skill_name("code-review").is_ok());
        assert!(validate_skill_name("A").is_err());
        assert!(validate_skill_name("-bad").is_err());
        assert!(validate_skill_name("bad-").is_err());
        assert!(validate_skill_name("").is_err());
    }

    #[test]
    fn discover_and_merge_prefers_workspace() {
        let root = temp_root("noxcode-skills");
        fs::create_dir_all(root.join(".agents/skills/demo")).expect("mkdir agents");
        fs::create_dir_all(root.join(".claude/skills/demo")).expect("mkdir claude");
        fs::write(
            root.join(".agents/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: from agents\n---\nagents body\n",
        )
        .expect("write agents");
        fs::write(
            root.join(".claude/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: from claude\n---\nclaude body\n",
        )
        .expect("write claude");
        fs::write(root.join(".agents/skills/demo/notes.md"), "extra").expect("extra");
        let primary = temp_root("noxcode-skills-global");
        write_skill(&primary.join("demo"), "demo", "from global");
        write_skill(&primary.join("other"), "other", "global other");
        let merged = merge_skills(
            discover_local_workspace_skills(&root.to_string_lossy())
                .into_iter()
                .chain(discover_global_skills_from(Some(&primary), None))
                .collect(),
        );
        assert_eq!(merged.len(), 2);
        let demo = find_skill(&merged, "demo").expect("demo");
        assert_eq!(demo.source, SkillSource::WorkspaceAgents);
        assert_eq!(demo.description, "from agents");
        assert!(demo.extra_files.iter().any(|item| item == "notes.md"));
        assert!(find_skill(&merged, "other").is_ok());
        let prompt = format_skills_prompt(&merged);
        assert!(prompt.contains("可用技能"));
        assert!(prompt.contains("`demo`"));
        let rendered = render_skill(demo, true);
        assert!(rendered.contains("agents body"));
        assert!(!rendered.contains("附属文件不在远端"));
        let global = find_skill(&merged, "other").expect("other");
        assert!(render_skill(global, true).contains("附属文件不在远端"));
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(primary);
    }

    #[test]
    fn discover_zcode_and_git_root() {
        let repo = temp_root("noxcode-skills-git");
        fs::create_dir_all(repo.join(".git")).expect("git");
        write_skill(
            &repo.join(".zcode/skills/from-root"),
            "from-root",
            "repo root",
        );
        let nested = repo.join("apps/web");
        fs::create_dir_all(nested.join(".noxcode/skills/local")).expect("nested");
        write_skill(&nested.join(".noxcode/skills/local"), "local", "cwd skill");
        write_skill(
            &nested.join(".zcode/skills/shadow"),
            "from-root",
            "cwd zcode",
        );
        let found = discover_local_workspace_skills(&nested.to_string_lossy());
        let merged = merge_skills(found);
        assert!(find_skill(&merged, "local").is_ok());
        let shadowed = find_skill(&merged, "from-root").expect("from-root");
        assert_eq!(shadowed.source, SkillSource::WorkspaceZcode);
        assert_eq!(shadowed.description, "cwd zcode");
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn merge_reports_duplicate_diagnostics() {
        let root = temp_root("noxcode-skills-dup");
        write_skill(&root.join(".noxcode/skills/demo"), "demo", "ws");
        write_skill(&root.join(".agents/skills/demo"), "demo", "agents");
        let (_, diagnostics) =
            merge_skills_detailed(discover_local_workspace_skills(&root.to_string_lossy()));
        assert!(diagnostics
            .iter()
            .any(|item| item.code == "skill_duplicate_name"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn disabled_paths_are_filtered() {
        let config = temp_root("noxcode-skills-state");
        let primary = config.join("user-skills");
        write_skill(&primary.join("keep"), "keep", "keep me");
        write_skill(&primary.join("drop"), "drop", "drop me");
        let drop_path = primary.join("drop/SKILL.md");
        save_skills_state(
            &config,
            &SkillsState {
                disabled_paths: vec![drop_path.to_string_lossy().into_owned()],
            },
        )
        .expect("save");
        let merged = merge_skills(discover_global_skills_from(Some(&primary), None));
        let filtered = filter_disabled_skills(&merged, Some(&config));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "keep");
        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn create_skill_writes_template() {
        let root = temp_root("noxcode-skills-create");
        let skill = write_skill_dir(&root, "demo-skill", "A demo").expect("create");
        assert_eq!(skill.name, "demo-skill");
        assert!(root.join("demo-skill/SKILL.md").exists());
        assert!(validate_skill_name("Bad Name").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scan_skips_existing_names() {
        let external = temp_root("noxcode-skills-ext");
        write_skill(&external.join("alpha"), "alpha", "from ext");
        write_skill(&external.join("beta"), "beta", "also ext");
        let target = temp_root("noxcode-skills-target");
        write_skill(&target.join("alpha"), "alpha", "already");
        let existing = existing_skill_names(&[&target]);
        let scanned = scan_external_dir(&external, &existing);
        let alpha = scanned
            .iter()
            .find(|item| item.name == "alpha")
            .expect("alpha");
        let beta = scanned
            .iter()
            .find(|item| item.name == "beta")
            .expect("beta");
        assert!(!alpha.importable);
        assert_eq!(alpha.skip_reason.as_deref(), Some("sameNameExists"));
        assert!(beta.importable);
        let _ = fs::remove_dir_all(external);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn missing_skill_errors() {
        let err = find_skill(&[], "nope").unwrap_err();
        assert!(err.contains("未找到技能"));
    }
}
