//! 自定义斜杠命令：一个 Markdown 文件就是一条命令，正文是提示词模板。
//!
//! 来源（优先级从高到低，同名只保留优先者）：
//! 工作区 `.noxcode/commands/`、`.claude/commands/`、已启用插件的 `commands/`、全局
//! `$APPCONFIG/native-commands/`。文件名（去 `.md`）即命令名，子目录作为命名空间：
//! `commands/frontend/component.md` → `/frontend:component`。
//!
//! frontmatter（可选）：`description`、`argument-hint`、`allowed-tools`、`model`、`skills`。
//! 正文占位符：`$ARGUMENTS`（全部参数）、`$1..$9`（按空白切分、支持引号）；没有占位符而又
//! 传了参数时，参数追加到末尾。``!`cmd` `` 形式的内联命令在本地工作区执行后以输出替换
//! （30 秒超时，输出截断到 8k 字符）。
//!
//! 内置命令（`/compact /init /fork /mode /model /effort /goal /skill /memory /mcp /plugins
//! /new /help`）由前端注册表处理，不在这里出现。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::native::plugins::{load_enabled_plugins, plugin_command_dirs, NativePlugin};

pub const GLOBAL_COMMANDS_DIR_NAME: &str = "native-commands";
pub const WORKSPACE_COMMAND_DIRS: &[&str] = &[".noxcode/commands", ".claude/commands"];
pub const MAX_COMMANDS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 200;
const INLINE_BASH_TIMEOUT: Duration = Duration::from_secs(30);
const INLINE_BASH_MAX_CHARS: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandSource {
    WorkspaceNoxcode,
    WorkspaceClaude,
    Plugin,
    Global,
}

impl SlashCommandSource {
    fn rank(self) -> u8 {
        match self {
            Self::WorkspaceNoxcode => 0,
            Self::WorkspaceClaude => 1,
            Self::Plugin => 2,
            Self::Global => 3,
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::WorkspaceNoxcode => "工作区 .noxcode",
            Self::WorkspaceClaude => "工作区 .claude",
            Self::Plugin => "插件",
            Self::Global => "全局",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSlashCommand {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub source: SlashCommandSource,
    #[serde(default)]
    pub plugin: Option<String>,
    pub path: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpandedSlashCommand {
    pub name: String,
    pub prompt: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub skills: Vec<String>,
}

// ---------------------------------------------------------------------------
// frontmatter
// ---------------------------------------------------------------------------

/// 解析简单 YAML frontmatter：`key: value` 与 `key:` + 缩进 `- item` 列表（列表以 `, ` 拼接）。
/// 返回（字段表，正文）。没有 frontmatter 时字段表为空、正文为原文。
pub fn parse_frontmatter(raw: &str) -> (BTreeMap<String, String>, String) {
    let trimmed = raw.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (BTreeMap::new(), trimmed.to_string());
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    let Some((front, body)) = rest.split_once("\n---") else {
        return (BTreeMap::new(), trimmed.to_string());
    };
    let mut fields = BTreeMap::new();
    let mut current_key: Option<String> = None;
    for line in front.lines() {
        let line = line.trim_end_matches('\r');
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) || stripped.starts_with('-') {
            if let Some(key) = current_key.as_ref() {
                if let Some(item) = stripped.strip_prefix('-') {
                    let item = unquote(item);
                    if item.is_empty() {
                        continue;
                    }
                    let entry: &mut String = fields.entry(key.clone()).or_default();
                    if !entry.is_empty() {
                        entry.push_str(", ");
                    }
                    entry.push_str(&item);
                    continue;
                }
            }
        }
        if let Some((key, value)) = stripped.split_once(':') {
            let key = key.trim().to_ascii_lowercase().replace('_', "-");
            if key.is_empty() || key.contains(' ') {
                continue;
            }
            fields.insert(key.clone(), unquote(value));
            current_key = Some(key);
        }
    }
    let body = body
        .trim_start_matches('-')
        .trim_start_matches(['\r', '\n'])
        .to_string();
    (fields, body)
}

pub fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// `[a, b]` / `a, b` / `a b` / 已由块列表拼成的 `a, b` → 去重后的列表。
/// 含逗号或括号（如 `Bash(git add:*)`）时只按逗号切分，避免拆散带空格的模式。
pub fn parse_list_field(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let raw_items: Vec<&str> = if inner.contains(',') || inner.contains('(') {
        inner.split(',').collect()
    } else {
        inner.split_whitespace().collect()
    };
    let mut out: Vec<String> = Vec::new();
    for item in raw_items {
        let item = unquote(item);
        if item.is_empty() || out.iter().any(|existing| existing == &item) {
            continue;
        }
        out.push(item);
    }
    out
}

fn truncate_chars(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(max).collect();
    format!("{prefix}…")
}

fn first_non_empty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

pub fn parse_command_markdown(
    raw: &str,
    name: &str,
    source: SlashCommandSource,
    plugin: Option<&str>,
    path: &Path,
) -> NativeSlashCommand {
    let (fields, body) = parse_frontmatter(raw);
    let description = fields
        .get("description")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| first_non_empty_line(&body));
    NativeSlashCommand {
        name: name.to_string(),
        description: truncate_chars(&description, MAX_DESCRIPTION_CHARS),
        argument_hint: fields
            .get("argument-hint")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        allowed_tools: fields
            .get("allowed-tools")
            .map(|value| parse_list_field(value))
            .unwrap_or_default(),
        model: fields
            .get("model")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        skills: fields
            .get("skills")
            .map(|value| parse_list_field(value))
            .unwrap_or_default(),
        source,
        plugin: plugin.map(str::to_string),
        path: path.to_string_lossy().into_owned(),
        body,
    }
}

// ---------------------------------------------------------------------------
// 发现
// ---------------------------------------------------------------------------

pub fn normalize_command_name(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('/').to_ascii_lowercase();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.'))
    {
        return None;
    }
    Some(trimmed)
}

fn command_name_for(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let stem = rel.with_extension("");
    let joined = stem
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");
    normalize_command_name(&joined)
}

fn walk_markdown(root: &Path, current: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 2 || out.len() >= MAX_COMMANDS {
        return;
    }
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            walk_markdown(root, &path, depth + 1, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
            if out.len() >= MAX_COMMANDS {
                return;
            }
        }
    }
}

pub fn collect_command_dir(
    dir: &Path,
    source: SlashCommandSource,
    plugin: Option<&str>,
    out: &mut Vec<NativeSlashCommand>,
) {
    let mut files = Vec::new();
    walk_markdown(dir, dir, 0, &mut files);
    for file in files {
        let Some(name) = command_name_for(dir, &file) else {
            continue;
        };
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        out.push(parse_command_markdown(&raw, &name, source, plugin, &file));
    }
}

pub fn merge_commands(mut items: Vec<NativeSlashCommand>) -> Vec<NativeSlashCommand> {
    items.sort_by(|left, right| {
        left.source
            .rank()
            .cmp(&right.source.rank())
            .then_with(|| left.name.cmp(&right.name))
    });
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if !seen.insert(item.name.clone()) {
            continue;
        }
        out.push(item);
        if out.len() >= MAX_COMMANDS {
            break;
        }
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out
}

pub fn discover_commands(
    workspace_root: Option<&Path>,
    config_dir: Option<&Path>,
    plugins: &[NativePlugin],
) -> Vec<NativeSlashCommand> {
    let mut out = Vec::new();
    if let Some(root) = workspace_root {
        collect_command_dir(
            &root.join(WORKSPACE_COMMAND_DIRS[0]),
            SlashCommandSource::WorkspaceNoxcode,
            None,
            &mut out,
        );
        collect_command_dir(
            &root.join(WORKSPACE_COMMAND_DIRS[1]),
            SlashCommandSource::WorkspaceClaude,
            None,
            &mut out,
        );
    }
    for (plugin, dir) in plugin_command_dirs(plugins) {
        collect_command_dir(&dir, SlashCommandSource::Plugin, Some(&plugin), &mut out);
    }
    if let Some(config) = config_dir {
        collect_command_dir(
            &config.join(GLOBAL_COMMANDS_DIR_NAME),
            SlashCommandSource::Global,
            None,
            &mut out,
        );
    }
    merge_commands(out)
}

// ---------------------------------------------------------------------------
// 展开
// ---------------------------------------------------------------------------

/// 按空白切分参数，支持单双引号包裹与反斜杠转义。
pub fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut has_token = false;
    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            has_token = true;
            continue;
        }
        match ch {
            '\\' if quote != Some('\'') => escaped = true,
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                    has_token = true;
                } else {
                    current.push(ch);
                }
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            ch => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if escaped {
        current.push('\\');
        has_token = true;
    }
    if has_token {
        out.push(current);
    }
    out
}

pub fn has_argument_placeholders(body: &str) -> bool {
    if body.contains("$ARGUMENTS") {
        return true;
    }
    let bytes = body.as_bytes();
    bytes.windows(2).any(|window| {
        window[0] == b'$' && window[1].is_ascii_digit() && window[1] != b'0'
    })
}

pub fn expand_arguments(body: &str, args: &str) -> String {
    let args = args.trim();
    let positional = split_args(args);
    let mut out = body.to_string();
    let had_placeholders = has_argument_placeholders(body);
    out = out.replace("$ARGUMENTS", args);
    for index in (1..=9usize).rev() {
        let token = format!("${index}");
        if !out.contains(&token) {
            continue;
        }
        let value = positional.get(index - 1).map(String::as_str).unwrap_or("");
        out = out.replace(&token, value);
    }
    if !had_placeholders && !args.is_empty() {
        out = format!("{}\n\n{args}", out.trim_end());
    }
    out
}

/// 找出所有 ``!`cmd` `` 片段：返回 (起始偏移, 结束偏移, 命令)。
pub fn inline_bash_spans(body: &str) -> Vec<(usize, usize, String)> {
    let mut spans = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = body[search_from..].find("!`") {
        let start = search_from + rel;
        let cmd_start = start + 2;
        let Some(rel_end) = body[cmd_start..].find('`') else {
            break;
        };
        let end = cmd_start + rel_end;
        let raw_command = &body[cmd_start..end];
        let command = raw_command.trim().to_string();
        if !command.is_empty() && !raw_command.contains('\n') {
            spans.push((start, end + 1, command));
        }
        search_from = end + 1;
    }
    spans
}

async fn run_inline_bash(command: &str, cwd: &Path) -> String {
    let mut child = tokio::process::Command::new("bash");
    child
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = match child.spawn() {
        Ok(process) => tokio::time::timeout(INLINE_BASH_TIMEOUT, process.wait_with_output()).await,
        Err(error) => return format!("(执行失败: {error})"),
    };
    let text = match output {
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push('\n');
                }
                text.push_str(stderr.trim_end());
            }
            if !output.status.success() {
                text.push_str(&format!(
                    "\n(exit {})",
                    output.status.code().unwrap_or(-1)
                ));
            }
            text
        }
        Ok(Err(error)) => format!("(执行失败: {error})"),
        Err(_) => "(执行超时 30s)".to_string(),
    };
    let trimmed = text.trim();
    if trimmed.chars().count() > INLINE_BASH_MAX_CHARS {
        let prefix: String = trimmed.chars().take(INLINE_BASH_MAX_CHARS).collect();
        format!("{prefix}\n…(已截断)")
    } else {
        trimmed.to_string()
    }
}

/// 执行正文里的内联命令并替换为输出；`cwd` 为 `None`（SSH 工作区或未知）时原样保留。
pub async fn expand_inline_bash(body: &str, cwd: Option<&Path>) -> String {
    let Some(cwd) = cwd else {
        return body.to_string();
    };
    let spans = inline_bash_spans(body);
    if spans.is_empty() {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len());
    let mut cursor = 0;
    for (start, end, command) in spans {
        out.push_str(&body[cursor..start]);
        out.push_str(&run_inline_bash(&command, cwd).await);
        cursor = end;
    }
    out.push_str(&body[cursor..]);
    out
}

pub fn find_command<'a>(
    commands: &'a [NativeSlashCommand],
    name: &str,
) -> Result<&'a NativeSlashCommand, String> {
    let needle = normalize_command_name(name).ok_or_else(|| format!("命令名无效：{name}"))?;
    commands
        .iter()
        .find(|item| item.name == needle)
        .ok_or_else(|| format!("未找到斜杠命令：/{needle}"))
}

pub async fn expand_command(
    command: &NativeSlashCommand,
    args: &str,
    cwd: Option<&Path>,
) -> ExpandedSlashCommand {
    let with_args = expand_arguments(&command.body, args);
    let prompt = expand_inline_bash(&with_args, cwd).await;
    ExpandedSlashCommand {
        name: command.name.clone(),
        prompt: prompt.trim().to_string(),
        allowed_tools: command.allowed_tools.clone(),
        model: command.model.clone(),
        skills: command.skills.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

fn config_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

async fn load_for_workspace<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: Option<&str>,
) -> (Vec<NativeSlashCommand>, Option<PathBuf>) {
    let config = config_dir(app);
    let workspace_root = crate::native::permission_rules::local_workspace_root(app, workspace_id)
        .await
        .ok()
        .flatten();
    let plugins = load_enabled_plugins(config.as_deref(), workspace_root.as_deref());
    (
        discover_commands(workspace_root.as_deref(), config.as_deref(), &plugins),
        workspace_root,
    )
}

#[tauri::command]
pub async fn list_native_slash_commands<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<Vec<NativeSlashCommand>, String> {
    if let Some(config) = config_dir(&app) {
        let dir = config.join(GLOBAL_COMMANDS_DIR_NAME);
        if !dir.exists() {
            let _ = fs::create_dir_all(&dir);
        }
    }
    let (commands, _) = load_for_workspace(&app, workspace_id.as_deref()).await;
    Ok(commands)
}

#[tauri::command]
pub async fn expand_native_slash_command<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    name: String,
    args: Option<String>,
) -> Result<ExpandedSlashCommand, String> {
    let (commands, workspace_root) = load_for_workspace(&app, workspace_id.as_deref()).await;
    let command = find_command(&commands, &name)?;
    Ok(expand_command(
        command,
        args.as_deref().unwrap_or(""),
        workspace_root.as_deref(),
    )
    .await)
}

#[tauri::command]
pub async fn open_native_commands_dir<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let dir = config_dir(&app)
        .ok_or_else(|| "无法读取应用配置目录".to_string())?
        .join(GLOBAL_COMMANDS_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|error| format!("创建命令目录失败: {error}"))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| format!("打开命令目录失败: {error}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "noxcode-commands-{}",
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
    fn frontmatter_supports_inline_and_block_lists() {
        let raw = "---\ndescription: \"Review a PR\"\nargument-hint: [pr-number]\nallowed_tools:\n  - Bash(git *)\n  - Read\nmodel: gpt-5\nskills: [review, 'lint']\n---\n\n# Body\nUse $1\n";
        let command = parse_command_markdown(
            raw,
            "review",
            SlashCommandSource::WorkspaceClaude,
            None,
            Path::new("/x/review.md"),
        );
        assert_eq!(command.description, "Review a PR");
        assert_eq!(command.argument_hint.as_deref(), Some("[pr-number]"));
        assert_eq!(command.allowed_tools, vec!["Bash(git *)", "Read"]);
        assert_eq!(parse_list_field("Read Grep Glob"), vec!["Read", "Grep", "Glob"]);
        assert_eq!(command.model.as_deref(), Some("gpt-5"));
        assert_eq!(command.skills, vec!["review", "lint"]);
        assert_eq!(command.body.trim(), "# Body\nUse $1");

        let plain = parse_command_markdown(
            "# Just a title\n\nDo the thing",
            "thing",
            SlashCommandSource::Global,
            None,
            Path::new("/x/thing.md"),
        );
        assert_eq!(plain.description, "Just a title");
        assert!(plain.allowed_tools.is_empty());
        assert_eq!(plain.body, "# Just a title\n\nDo the thing");
    }

    #[test]
    fn arguments_expand_positionally_and_append_when_missing() {
        assert_eq!(
            expand_arguments("Fix $1 in $2; all: $ARGUMENTS", "bug \"src/a b.rs\" extra"),
            "Fix bug in src/a b.rs; all: bug \"src/a b.rs\" extra"
        );
        assert_eq!(expand_arguments("Only $3", "a b"), "Only ");
        assert_eq!(expand_arguments("No placeholders", "x y"), "No placeholders\n\nx y");
        assert_eq!(expand_arguments("No placeholders", "  "), "No placeholders");
        assert_eq!(split_args(r#"a 'b c' d\ e "f\"g""#), vec!["a", "b c", "d e", "f\"g"]);
        assert!(has_argument_placeholders("$1"));
        assert!(!has_argument_placeholders("$0 costs $"));
    }

    #[test]
    fn inline_bash_spans_are_detected() {
        let spans = inline_bash_spans("Status:\n!`git status --short`\nand !`echo hi` end");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].2, "git status --short");
        assert_eq!(spans[1].2, "echo hi");
        assert!(inline_bash_spans("no !`\nmultiline` here").is_empty());
    }

    #[tokio::test]
    async fn inline_bash_runs_in_cwd_when_local() {
        let root = temp_root();
        write(&root.join("marker.txt"), "x");
        let expanded = expand_inline_bash("Files: !`ls`", Some(&root)).await;
        assert_eq!(expanded, "Files: marker.txt");
        let untouched = expand_inline_bash("Files: !`ls`", None).await;
        assert_eq!(untouched, "Files: !`ls`");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_namespaces_and_prioritizes_sources() {
        let workspace = temp_root();
        let config = temp_root();
        write(
            &workspace.join(".noxcode/commands/deploy.md"),
            "---\ndescription: ws deploy\n---\nDeploy $ARGUMENTS",
        );
        write(
            &workspace.join(".claude/commands/deploy.md"),
            "---\ndescription: claude deploy\n---\nx",
        );
        write(
            &workspace.join(".claude/commands/frontend/component.md"),
            "Make component $1",
        );
        write(&workspace.join(".claude/commands/.hidden/skip.md"), "skip");
        write(&workspace.join(".claude/commands/Bad Name.md"), "skip");
        write(
            &config.join(GLOBAL_COMMANDS_DIR_NAME).join("deploy.md"),
            "global deploy",
        );
        write(
            &config.join(GLOBAL_COMMANDS_DIR_NAME).join("release.md"),
            "release",
        );
        let commands = discover_commands(Some(&workspace), Some(&config), &[]);
        let names: Vec<&str> = commands.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["deploy", "frontend:component", "release"]);
        let deploy = find_command(&commands, "/Deploy").expect("deploy");
        assert_eq!(deploy.source, SlashCommandSource::WorkspaceNoxcode);
        assert_eq!(deploy.description, "ws deploy");
        assert!(find_command(&commands, "missing").is_err());
        assert!(find_command(&commands, "bad name").is_err());
        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&config);
    }

    #[test]
    fn plugin_commands_are_included_with_plugin_name() {
        let config = temp_root();
        let plugin_root = crate::native::plugins::plugins_dir(&config).join("tools");
        write(
            &plugin_root.join("noxcode-plugin.json"),
            r#"{ "name": "tools" }"#,
        );
        write(&plugin_root.join("commands/lint.md"), "Run lint");
        let plugins = crate::native::plugins::load_enabled_plugins(Some(&config), None);
        let commands = discover_commands(None, Some(&config), &plugins);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "lint");
        assert_eq!(commands[0].source, SlashCommandSource::Plugin);
        assert_eq!(commands[0].plugin.as_deref(), Some("tools"));
        let _ = fs::remove_dir_all(&config);
    }
}
