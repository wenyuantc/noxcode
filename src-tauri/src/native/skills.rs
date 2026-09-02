#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::native::tools::ssh::SshToolRuntime;

pub const GLOBAL_SKILLS_DIR_NAME: &str = "native-skills";
pub const MAX_SKILLS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    WorkspaceAgents,
    WorkspaceClaude,
    Global,
}

impl SkillSource {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::WorkspaceAgents => "工作区 .agents",
            Self::WorkspaceClaude => "工作区 .claude",
            Self::Global => "全局",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::WorkspaceAgents => 0,
            Self::WorkspaceClaude => 1,
            Self::Global => 2,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeGlobalSkills {
    pub dir: String,
    pub skills: Vec<NativeSkill>,
}

pub fn parse_skill_markdown(raw: &str, fallback_name: &str) -> (String, String) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (
            fallback_name.to_string(),
            truncate_description(&first_non_empty_line(trimmed)),
        );
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    let Some((front, _)) = rest.split_once("\n---") else {
        return (
            fallback_name.to_string(),
            truncate_description(&first_non_empty_line(trimmed)),
        );
    };
    let mut name = String::new();
    let mut description = String::new();
    for line in front.lines() {
        if let Some(value) = line.trim().strip_prefix("name:") {
            name = unquote(value);
        } else if let Some(value) = line.trim().strip_prefix("description:") {
            description = unquote(value);
        }
    }
    if name.is_empty() {
        name = fallback_name.to_string();
    }
    if description.is_empty() {
        description = first_non_empty_line(trimmed);
    }
    (name, truncate_description(&description))
}

pub fn discover_local_workspace_skills(cwd: &str) -> Vec<NativeSkill> {
    let root = Path::new(cwd);
    let mut out = Vec::new();
    collect_local_skill_dir(
        root.join(".agents/skills"),
        SkillSource::WorkspaceAgents,
        &mut out,
    );
    collect_local_skill_dir(
        root.join(".claude/skills"),
        SkillSource::WorkspaceClaude,
        &mut out,
    );
    out
}

pub fn discover_global_skills(config_dir: &Path) -> Vec<NativeSkill> {
    let mut out = Vec::new();
    collect_local_skill_dir(
        config_dir.join(GLOBAL_SKILLS_DIR_NAME),
        SkillSource::Global,
        &mut out,
    );
    out
}

pub async fn load_session_skills(
    cwd: &str,
    ssh: Option<&SshToolRuntime>,
    config_dir: Option<&Path>,
) -> Vec<NativeSkill> {
    let mut items = if let Some(ssh) = ssh {
        discover_ssh_workspace_skills(ssh).await
    } else {
        discover_local_workspace_skills(cwd)
    };
    if let Some(dir) = config_dir {
        items.extend(discover_global_skills(dir));
    }
    merge_skills(items)
}

pub fn merge_skills(mut items: Vec<NativeSkill>) -> Vec<NativeSkill> {
    items.sort_by(|left, right| {
        left.source.rank().cmp(&right.source.rank()).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let key = item.name.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(item);
        if out.len() >= MAX_SKILLS {
            break;
        }
    }
    out
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
        lines.push(format!(
            "- `{}`：{}（{}）",
            skill.name,
            skill.description,
            skill.source.label_zh()
        ));
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
    format!(
        "# {}（{}）\n目录: {}\n\n## SKILL.md\n{}\n\n## 目录文件\n{extra}",
        skill.name,
        skill.source.label_zh(),
        skill.dir,
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
            "find .agents/skills .claude/skills -mindepth 2 -maxdepth 2 -name SKILL.md 2>/dev/null | head -n 80",
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
        let source = if path.starts_with(".agents/") {
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
        let (name, description) = parse_skill_markdown(&body, &fallback);
        let extra = ssh_list_extra_files(ssh, &dir).await;
        out.push(NativeSkill {
            name,
            description,
            source,
            dir,
            skill_md_path: path.to_string(),
            body,
            extra_files: extra,
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

fn collect_local_skill_dir(dir: PathBuf, source: SkillSource, out: &mut Vec<NativeSkill>) {
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
        let (name, description) = parse_skill_markdown(&body, fallback);
        out.push(NativeSkill {
            name,
            description,
            source,
            dir: skill_dir.to_string_lossy().into_owned(),
            skill_md_path: skill_md.to_string_lossy().into_owned(),
            body,
            extra_files: list_local_extra_files(&skill_dir),
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

fn unquote(value: &str) -> String {
    let value = value.trim();
    if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn truncate_description(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= MAX_DESCRIPTION_CHARS {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(MAX_DESCRIPTION_CHARS).collect();
    format!("{prefix}…")
}

pub fn global_skills_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?
        .join(GLOBAL_SKILLS_DIR_NAME))
}

#[tauri::command]
pub async fn list_native_global_skills<R: Runtime>(
    app: AppHandle<R>,
) -> Result<NativeGlobalSkills, String> {
    let dir = global_skills_dir(&app)?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|error| format!("创建全局技能目录失败: {error}"))?;
    }
    Ok(NativeGlobalSkills {
        dir: dir.to_string_lossy().into_owned(),
        skills: discover_global_skills(
            dir.parent()
                .ok_or_else(|| "无法解析全局技能目录".to_string())?,
        ),
    })
}

#[tauri::command]
pub async fn open_native_skills_dir<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = global_skills_dir(&app)?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|error| format!("创建全局技能目录失败: {error}"))?;
    }
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| format!("打开技能目录失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn discover_and_merge_prefers_workspace() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("codex-ai-skills-{stamp}"));
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
        let config = std::env::temp_dir().join(format!("codex-ai-skills-cfg-{stamp}"));
        fs::create_dir_all(config.join(GLOBAL_SKILLS_DIR_NAME).join("demo")).expect("mkdir global");
        fs::write(
            config.join(GLOBAL_SKILLS_DIR_NAME).join("demo/SKILL.md"),
            "---\nname: demo\ndescription: from global\n---\nglobal\n",
        )
        .expect("write global");
        fs::create_dir_all(config.join(GLOBAL_SKILLS_DIR_NAME).join("other")).expect("mkdir other");
        fs::write(
            config.join(GLOBAL_SKILLS_DIR_NAME).join("other/SKILL.md"),
            "---\nname: other\ndescription: global other\n---\n",
        )
        .expect("write other");
        let merged = merge_skills(
            discover_local_workspace_skills(&root.to_string_lossy())
                .into_iter()
                .chain(discover_global_skills(&config))
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
        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn missing_skill_errors() {
        let err = find_skill(&[], "nope").unwrap_err();
        assert!(err.contains("未找到技能"));
    }
}
