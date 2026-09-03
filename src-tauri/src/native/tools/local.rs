use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use tokio::io::AsyncReadExt;

use crate::native::model::types::NativeImage;
use crate::process_spawn::tokio_command;

use super::cancel::CancelFlag;
use super::glob::glob_match;
use super::paths::resolve_under_workspace;
use super::shell_snapshot::{COMMAND_ENV, SNAPSHOT_ENV};

const READ_DEFAULT_LIMIT: usize = 2000;
pub const BASH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
pub const BASH_MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// 单次 Bash 在内存里保留的输出上限；完整输出由 artifact 层落盘，这里只防止
/// `cat` 一个巨型文件把进程内存吃光。
const BASH_OUTPUT_HARD_LIMIT: usize = 2 * 1024 * 1024;
const BASH_MODEL_CHARS: usize = 30_000;
/// Read 工具支持直接返回的图片类型。
const IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// 文件不存在时，同目录里 Levenshtein 距离不超过这个值的文件名会作为提示返回。
const SIMILAR_NAME_DISTANCE: usize = 3;

#[derive(Debug, Clone)]
pub struct CommandStatus {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}

/// 读取时记录的文件指纹，Write / Edit 前用它判断文件是否被别人改过。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileFingerprint {
    pub len: u64,
    pub mtime_ms: i64,
    pub hash: u64,
}

impl FileFingerprint {
    pub fn of_content(path: &Path, content: &[u8]) -> Self {
        let metadata = fs::metadata(path).ok();
        let mtime_ms = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0);
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Self {
            len: content.len() as u64,
            mtime_ms,
            hash: hasher.finish(),
        }
    }

    pub fn of_path(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        Some(Self::of_content(path, &bytes))
    }
}

#[derive(Debug, Clone)]
pub struct LocalWorkspace {
    pub root: PathBuf,
    /// 已导出的 shell 快照；有则 Bash 走 `source 快照 + eval`，否则 `bash -lc`。
    pub shell_snapshot: Option<PathBuf>,
    /// 可用的 ripgrep 可执行文件；无则回退到 Rust 自实现的正则遍历。
    pub rg_binary: Option<PathBuf>,
    pub bash_default_timeout: Duration,
    /// 工作区之外允许 Read 的目录（artifact 目录、记忆目录等）。
    pub extra_read_roots: Vec<PathBuf>,
    /// 工作区之外允许 Write / Edit 的目录（记忆目录）。
    pub extra_write_roots: Vec<PathBuf>,
}

impl LocalWorkspace {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            shell_snapshot: None,
            rg_binary: None,
            bash_default_timeout: BASH_DEFAULT_TIMEOUT,
            extra_read_roots: Vec::new(),
            extra_write_roots: Vec::new(),
        }
    }

    pub fn resolve(&self, input: &str) -> Result<PathBuf, String> {
        resolve_under_workspace(&self.root, input)
    }

    /// 只读解析：工作区优先，其次是 `extra_read_roots` 与 `extra_write_roots`。
    pub fn resolve_for_read(&self, input: &str) -> Result<PathBuf, String> {
        match resolve_under_workspace(&self.root, input) {
            Ok(path) => Ok(path),
            Err(error) => {
                for root in self.extra_read_roots.iter().chain(&self.extra_write_roots) {
                    if let Ok(path) = resolve_under_workspace(root, input) {
                        return Ok(path);
                    }
                }
                Err(error)
            }
        }
    }

    /// 可写解析：工作区优先，其次是 `extra_write_roots`（记忆目录）。
    pub fn resolve_for_write(&self, input: &str) -> Result<PathBuf, String> {
        match resolve_under_workspace(&self.root, input) {
            Ok(path) => Ok(path),
            Err(error) => {
                for root in &self.extra_write_roots {
                    if let Ok(path) = resolve_under_workspace(root, input) {
                        return Ok(path);
                    }
                }
                Err(error)
            }
        }
    }

    pub fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<String, String> {
        let resolved = self.resolve_for_read(path)?;
        let metadata = fs::metadata(&resolved).map_err(|_| missing_file_error(&resolved, path))?;
        if metadata.is_dir() {
            return Err(format!("路径是目录: {path}"));
        }
        let content =
            fs::read_to_string(&resolved).map_err(|error| format!("读取失败: {error}"))?;
        Ok(format_read(&content, offset, limit))
    }

    /// 读取图片文件并编码为 `NativeImage`。
    pub fn read_image(&self, path: &str) -> Result<NativeImage, String> {
        let resolved = self.resolve_for_read(path)?;
        let mime =
            image_mime_type(&resolved).ok_or_else(|| format!("不是支持的图片类型: {path}"))?;
        let metadata = fs::metadata(&resolved).map_err(|_| missing_file_error(&resolved, path))?;
        if metadata.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "图片超过 {} MB 上限: {path}",
                MAX_IMAGE_BYTES / (1024 * 1024)
            ));
        }
        let bytes = fs::read(&resolved).map_err(|error| format!("读取失败: {error}"))?;
        Ok(NativeImage {
            name: resolved
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string()),
            mime_type: mime.to_string(),
            data_base64: BASE64.encode(bytes),
        })
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<String, String> {
        let resolved = self.resolve_for_write(path)?;
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建目录失败: {error}"))?;
        }
        fs::write(&resolved, content).map_err(|error| format!("写入失败: {error}"))?;
        Ok(format!(
            "Wrote {} bytes to {}",
            content.len(),
            resolved.display()
        ))
    }

    pub fn delete_file(&self, path: &str) -> Result<String, String> {
        let resolved = self.resolve(path)?;
        let metadata = fs::metadata(&resolved).map_err(|_| missing_file_error(&resolved, path))?;
        if metadata.is_dir() {
            return Err(format!("路径是目录: {path}"));
        }
        fs::remove_file(&resolved).map_err(|error| format!("删除失败: {error}"))?;
        Ok(format!("Deleted {}", resolved.display()))
    }

    pub fn glob_files(&self, pattern: &str, search_path: Option<&str>) -> Result<String, String> {
        let root = match search_path {
            Some(path) => self.resolve(path)?,
            None => self.root.clone(),
        };
        if !root.exists() {
            return Ok("No files found".to_string());
        }
        let mut files = Vec::new();
        walk_files(&root, &mut files);
        let mut matches = Vec::new();
        for file in files {
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(file.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            if glob_match(pattern, &rel) {
                matches.push(file.display().to_string());
            }
            if matches.len() >= 100 {
                break;
            }
        }
        if matches.is_empty() {
            Ok("No files found".to_string())
        } else {
            Ok(matches.join("\n"))
        }
    }

    pub fn grep_files(
        &self,
        pattern: &str,
        path: Option<&str>,
        glob: Option<&str>,
        head_limit: Option<i64>,
    ) -> Result<String, String> {
        if pattern.trim().is_empty() {
            return Err("pattern 不能为空".to_string());
        }
        let root = match path {
            Some(value) => self.resolve(value)?,
            None => self.root.clone(),
        };
        let limit = head_limit.unwrap_or(250).clamp(1, 1000) as usize;
        if let Some(rg) = self.rg_binary.as_ref() {
            if let Ok(output) = grep_with_ripgrep(rg, pattern, &root, glob, limit) {
                return Ok(output);
            }
        }
        let regex = regex::RegexBuilder::new(pattern)
            .build()
            .map_err(|error| format!("pattern 不是合法正则: {error}"))?;
        let mut files = Vec::new();
        if root.is_file() {
            files.push(root);
        } else {
            walk_files(&root, &mut files);
        }
        let mut hits = Vec::new();
        for file in files {
            if let Some(glob_pattern) = glob {
                let rel = file
                    .strip_prefix(&self.root)
                    .unwrap_or(file.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if !glob_match(glob_pattern, &rel) {
                    continue;
                }
            }
            let Ok(content) = fs::read_to_string(&file) else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    hits.push(format!("{}:{}:{line}", file.display(), index + 1));
                    if hits.len() >= limit {
                        return Ok(hits.join("\n"));
                    }
                }
            }
        }
        if hits.is_empty() {
            Ok("No matches found".to_string())
        } else {
            Ok(hits.join("\n"))
        }
    }

    pub async fn bash(
        &self,
        command: &str,
        timeout_ms: Option<i64>,
        cancel: &CancelFlag,
        extra_env: &[(String, String)],
    ) -> Result<String, String> {
        let status = self
            .bash_with_status(command, timeout_ms, cancel, extra_env)
            .await?;
        if status.timed_out {
            return Err("Bash 超时".to_string());
        }
        if status.exit_code != 0 {
            return Err(if status.output.is_empty() {
                format!("command failed: {}", status.exit_code)
            } else {
                status.output
            });
        }
        if status.output.trim().is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(status.output)
        }
    }

    pub async fn bash_with_status(
        &self,
        command: &str,
        timeout_ms: Option<i64>,
        cancel: &CancelFlag,
        extra_env: &[(String, String)],
    ) -> Result<CommandStatus, String> {
        if command.trim().is_empty() {
            return Err("command 不能为空".to_string());
        }
        if cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        let timeout = Duration::from_millis(
            timeout_ms
                .unwrap_or(self.bash_default_timeout.as_millis() as i64)
                .clamp(1, BASH_MAX_TIMEOUT.as_millis() as i64) as u64,
        );
        let mut cmd = tokio_command("bash");
        match self.shell_snapshot.as_ref().filter(|path| path.is_file()) {
            Some(snapshot) => {
                cmd.arg("-c")
                    .arg(format!(
                        "source \"${SNAPSHOT_ENV}\" >/dev/null 2>&1 || true; eval \"${COMMAND_ENV}\""
                    ))
                    .env(SNAPSHOT_ENV, snapshot)
                    .env(COMMAND_ENV, command);
            }
            None => {
                cmd.arg("-lc").arg(command);
            }
        }
        cmd.current_dir(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|error| format!("启动 Bash 失败: {error}"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Bash stdout 不可用".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Bash stderr 不可用".to_string())?;
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let status = tokio::select! {
            result = child.wait() => {
                stdout.read_to_end(&mut stdout_buf).await.ok();
                stderr.read_to_end(&mut stderr_buf).await.ok();
                result.map_err(|error| format!("Bash 执行失败: {error}"))?
            }
            _ = tokio::time::sleep(timeout) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Ok(CommandStatus {
                    exit_code: -1,
                    output: "Bash 超时".to_string(),
                    timed_out: true,
                });
            }
            _ = wait_cancel(cancel) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err("已取消".to_string());
            }
        };
        let mut text = String::from_utf8_lossy(&stdout_buf).into_owned();
        if !stderr_buf.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&String::from_utf8_lossy(&stderr_buf));
        }
        let text = cap_bytes_tail(&text, BASH_OUTPUT_HARD_LIMIT);
        Ok(CommandStatus {
            exit_code: status.code().unwrap_or(-1),
            output: text,
            timed_out: false,
        })
    }
}

/// 查找可用的 ripgrep：优先随应用打包的 `tools/rg`，其次 PATH。
pub fn locate_ripgrep(bundled_dir: Option<&Path>) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) { "rg.exe" } else { "rg" };
    if let Some(dir) = bundled_dir {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn grep_with_ripgrep(
    rg: &Path,
    pattern: &str,
    root: &Path,
    glob: Option<&str>,
    limit: usize,
) -> Result<String, String> {
    let mut cmd = std::process::Command::new(rg);
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count")
        .arg(limit.to_string())
        .arg("-e")
        .arg(pattern);
    if let Some(glob_pattern) = glob {
        cmd.arg("--glob").arg(glob_pattern);
    }
    cmd.arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .map_err(|error| format!("启动 ripgrep 失败: {error}"))?;
    match output.status.code() {
        Some(0) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = text.lines().take(limit).collect();
            Ok(lines.join("\n"))
        }
        Some(1) => Ok("No matches found".to_string()),
        _ => Err(String::from_utf8_lossy(&output.stderr).into_owned()),
    }
}

fn missing_file_error(resolved: &Path, requested: &str) -> String {
    match similar_filename_hint(resolved) {
        Some(hint) => format!("文件不存在: {requested}（{hint}）"),
        None => format!("文件不存在: {requested}"),
    }
}

/// 文件不存在时在同目录寻找拼写相近的文件名。
pub fn similar_filename_hint(resolved: &Path) -> Option<String> {
    let parent = resolved.parent()?;
    let wanted = resolved.file_name()?.to_string_lossy().to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for entry in fs::read_dir(parent).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let distance = levenshtein(&wanted, &name.to_lowercase());
        if distance == 0 || distance > SIMILAR_NAME_DISTANCE {
            continue;
        }
        if best.as_ref().is_none_or(|(current, _)| distance < *current) {
            best = Some((distance, name));
        }
    }
    best.map(|(_, name)| format!("你是否想找 {name}？"))
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

pub fn image_mime_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .find(|(candidate, _)| *candidate == ext)
        .map(|(_, mime)| *mime)
}

pub fn format_read(content: &str, offset: Option<i64>, limit: Option<i64>) -> String {
    let mut lines: Vec<&str> = content.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let start = offset.unwrap_or(1).max(1) as usize;
    let take = limit.unwrap_or(READ_DEFAULT_LIMIT as i64).max(1) as usize;
    let start_index = start.saturating_sub(1);
    let slice = lines.iter().skip(start_index).take(take);
    let mut out = String::new();
    for (idx, line) in slice.enumerate() {
        let number = start_index + idx + 1;
        out.push_str(&format!("{number:6}\t{line}\n"));
    }
    if out.is_empty() {
        "(empty file)".to_string()
    } else {
        out
    }
}

/// 一次 Edit 的结果：新内容、命中的匹配策略、替换次数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    pub content: String,
    pub strategy: &'static str,
    pub replacements: usize,
}

pub const EDIT_STRATEGY_EXACT: &str = "exact";

/// 兼容旧签名：只返回新内容。
pub fn apply_edit(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<String, String> {
    apply_edit_fuzzy(content, old, new, replace_all).map(|outcome| outcome.content)
}

/// 按策略链依次尝试匹配：exact → quote_normalized → line_number_prefix_stripped
/// → escape_normalized → unicode_escape_normalized → line_trimmed →
/// indentation_flexible → block_anchor。CRLF 文件会先归一化再还原。
pub fn apply_edit_fuzzy(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<EditOutcome, String> {
    if old == new {
        return Err("old_string and new_string must be different".to_string());
    }
    if old.is_empty() {
        return Err("old_string 不能为空".to_string());
    }
    let crlf = content.contains("\r\n") && !old.contains('\r');
    let working = if crlf {
        content.replace("\r\n", "\n")
    } else {
        content.to_string()
    };
    let new = if crlf {
        new.replace("\r\n", "\n")
    } else {
        new.to_string()
    };
    let outcome = apply_edit_strategies(&working, old, &new, replace_all)?;
    Ok(EditOutcome {
        content: if crlf {
            outcome.content.replace('\n', "\r\n")
        } else {
            outcome.content
        },
        strategy: outcome.strategy,
        replacements: outcome.replacements,
    })
}

fn apply_edit_strategies(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<EditOutcome, String> {
    // 1. exact
    if let Some(outcome) = replace_literal(content, old, new, replace_all, EDIT_STRATEGY_EXACT)? {
        return Ok(outcome);
    }
    // 2. quote_normalized：弯引号与直引号视为等价。
    if let Some(outcome) = replace_char_normalized(
        content,
        old,
        new,
        replace_all,
        normalize_quote_char,
        "quote_normalized",
    )? {
        return Ok(outcome);
    }
    // 3. line_number_prefix_stripped：模型把 Read 输出里的行号一起粘进来了。
    let stripped = strip_line_number_prefixes(old);
    if stripped != old {
        if let Some(outcome) = replace_literal(
            content,
            &stripped,
            new,
            replace_all,
            "line_number_prefix_stripped",
        )? {
            return Ok(outcome);
        }
    }
    // 4. escape_normalized：`\n` `\t` `\"` 等被二次转义。
    let unescaped = unescape_common_sequences(old);
    if unescaped != old {
        if let Some(outcome) =
            replace_literal(content, &unescaped, new, replace_all, "escape_normalized")?
        {
            return Ok(outcome);
        }
    }
    // 5. unicode_escape_normalized：`\u00e9` 形式的转义。
    let decoded = decode_unicode_escapes(old);
    if decoded != old {
        if let Some(outcome) = replace_literal(
            content,
            &decoded,
            new,
            replace_all,
            "unicode_escape_normalized",
        )? {
            return Ok(outcome);
        }
    }
    // 6. indentation_flexible：整块缩进平移（相对缩进一致），按文件缩进重排新内容。
    //    比 line_trimmed 严格，所以先试。
    if let Some(outcome) =
        replace_line_block(content, old, new, replace_all, LineMatch::Indentation)?
    {
        return Ok(outcome);
    }
    // 7. line_trimmed：逐行忽略首尾空白。
    if let Some(outcome) = replace_line_block(content, old, new, replace_all, LineMatch::Trimmed)? {
        return Ok(outcome);
    }
    // 8. block_anchor：首尾行锚定，中间行相似即可。
    if let Some(outcome) = replace_line_block(content, old, new, replace_all, LineMatch::Anchor)? {
        return Ok(outcome);
    }
    Err("old_string not found in file".to_string())
}

fn replace_literal(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    strategy: &'static str,
) -> Result<Option<EditOutcome>, String> {
    let count = content.matches(old).count();
    if count == 0 {
        return Ok(None);
    }
    if !replace_all && count > 1 {
        return Err(ambiguous_message(strategy, count));
    }
    Ok(Some(EditOutcome {
        content: if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        },
        strategy,
        replacements: if replace_all { count } else { 1 },
    }))
}

fn ambiguous_message(strategy: &str, count: usize) -> String {
    if strategy == EDIT_STRATEGY_EXACT {
        "old_string is not unique; use replace_all or provide more context".to_string()
    } else {
        format!(
            "old_string 经 {strategy} 归一化后命中 {count} 处，不唯一；请提供更多上下文或使用 replace_all"
        )
    }
}

fn normalize_quote_char(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        other => other,
    }
}

/// 在逐字符归一化后的文本里搜索，再把命中的字符区间映射回原文替换。
fn replace_char_normalized(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    normalize: fn(char) -> char,
    strategy: &'static str,
) -> Result<Option<EditOutcome>, String> {
    let content_chars: Vec<char> = content.chars().collect();
    let normalized_content: Vec<char> = content_chars.iter().map(|ch| normalize(*ch)).collect();
    let normalized_old: Vec<char> = old.chars().map(normalize).collect();
    if normalized_old.is_empty() || normalized_old.len() > normalized_content.len() {
        return Ok(None);
    }
    let mut starts = Vec::new();
    let mut index = 0;
    while index + normalized_old.len() <= normalized_content.len() {
        if normalized_content[index..index + normalized_old.len()] == normalized_old[..] {
            starts.push(index);
            index += normalized_old.len();
        } else {
            index += 1;
        }
    }
    if starts.is_empty() {
        return Ok(None);
    }
    if !replace_all && starts.len() > 1 {
        return Err(ambiguous_message(strategy, starts.len()));
    }
    let targets = if replace_all { starts } else { vec![starts[0]] };
    let mut out = String::with_capacity(content.len() + new.len());
    let mut cursor = 0;
    for start in &targets {
        out.extend(content_chars[cursor..*start].iter());
        out.push_str(new);
        cursor = start + normalized_old.len();
    }
    out.extend(content_chars[cursor..].iter());
    Ok(Some(EditOutcome {
        content: out,
        strategy,
        replacements: targets.len(),
    }))
}

fn strip_line_number_prefixes(old: &str) -> String {
    let lines: Vec<&str> = old.split('\n').collect();
    let stripped: Vec<String> = lines
        .iter()
        .map(|line| strip_one_line_number(line))
        .collect();
    // 只有所有非空行都带行号前缀时才认为是行号污染。
    let all_prefixed = lines
        .iter()
        .zip(stripped.iter())
        .filter(|(line, _)| !line.trim().is_empty())
        .all(|(line, out)| line != out);
    if all_prefixed {
        stripped.join("\n")
    } else {
        old.to_string()
    }
}

fn strip_one_line_number(line: &str) -> String {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits == 0 {
        return line.to_string();
    }
    let rest = &trimmed[digits..];
    let mut chars = rest.chars();
    match chars.next() {
        // Read 输出是 `{行号:6}\t{代码}`，制表符后的空白都是代码缩进，原样保留。
        Some('\t') => chars.as_str().to_string(),
        // `123| code` / `123: code` 形式允许分隔符后跟一个空格。
        Some('|') | Some(':') => {
            let after: &str = chars.as_str();
            after.strip_prefix(' ').unwrap_or(after).to_string()
        }
        _ => line.to_string(),
    }
}

fn unescape_common_sequences(old: &str) -> String {
    let mut out = String::with_capacity(old.len());
    let mut chars = old.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('r') => {
                chars.next();
                out.push('\r');
            }
            Some('"') => {
                chars.next();
                out.push('"');
            }
            Some('\'') => {
                chars.next();
                out.push('\'');
            }
            Some('`') => {
                chars.next();
                out.push('`');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some('$') => {
                chars.next();
                out.push('$');
            }
            _ => out.push('\\'),
        }
    }
    out
}

fn decode_unicode_escapes(old: &str) -> String {
    let mut out = String::with_capacity(old.len());
    let chars: Vec<char> = old.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\'
            && index + 5 < chars.len()
            && chars[index + 1] == 'u'
            && chars[index + 2..index + 6]
                .iter()
                .all(|ch| ch.is_ascii_hexdigit())
        {
            let hex: String = chars[index + 2..index + 6].iter().collect();
            if let Some(decoded) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(decoded);
                index += 6;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineMatch {
    Trimmed,
    Indentation,
    Anchor,
}

impl LineMatch {
    fn strategy(self) -> &'static str {
        match self {
            Self::Trimmed => "line_trimmed",
            Self::Indentation => "indentation_flexible",
            Self::Anchor => "block_anchor",
        }
    }
}

fn block_matches(window: &[&str], old_lines: &[&str], mode: LineMatch) -> bool {
    match mode {
        LineMatch::Trimmed => window
            .iter()
            .zip(old_lines)
            .all(|(a, b)| a.trim() == b.trim()),
        LineMatch::Indentation => {
            let file_base = indent_width(window[0]);
            let old_base = indent_width(old_lines[0]);
            window.iter().zip(old_lines).all(|(a, b)| {
                if a.trim_start() != b.trim_start() {
                    return false;
                }
                // 空行不参与缩进比较。
                if a.trim().is_empty() {
                    return true;
                }
                indent_width(a) as i64 - file_base as i64
                    == indent_width(b) as i64 - old_base as i64
            })
        }
        LineMatch::Anchor => {
            if old_lines.len() < 3 {
                return false;
            }
            let first =
                window.first().map(|line| line.trim()) == old_lines.first().map(|line| line.trim());
            let last =
                window.last().map(|line| line.trim()) == old_lines.last().map(|line| line.trim());
            if !first || !last {
                return false;
            }
            let middle = &window[1..window.len() - 1];
            let old_middle = &old_lines[1..old_lines.len() - 1];
            if middle.is_empty() {
                return true;
            }
            let same = middle
                .iter()
                .zip(old_middle)
                .filter(|(a, b)| a.trim() == b.trim())
                .count();
            same * 2 >= middle.len()
        }
    }
}

fn leading_whitespace(line: &str) -> &str {
    let end = line.len() - line.trim_start().len();
    &line[..end]
}

fn indent_width(line: &str) -> usize {
    leading_whitespace(line)
        .chars()
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

/// 按文件里命中块的缩进重排新内容：去掉 old 首行缩进，再加上文件首行缩进。
fn reindent_new_lines(new: &str, old_first: &str, file_first: &str) -> Vec<String> {
    let old_indent = leading_whitespace(old_first);
    let file_indent = leading_whitespace(file_first);
    new.split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                return line.to_string();
            }
            match line.strip_prefix(old_indent) {
                Some(rest) => format!("{file_indent}{rest}"),
                None => line.to_string(),
            }
        })
        .collect()
}

fn replace_line_block(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    mode: LineMatch,
) -> Result<Option<EditOutcome>, String> {
    let old_lines: Vec<&str> = old.split('\n').collect();
    if old_lines.iter().all(|line| line.trim().is_empty()) {
        return Ok(None);
    }
    let content_lines: Vec<&str> = content.split('\n').collect();
    if old_lines.len() > content_lines.len() {
        return Ok(None);
    }
    let mut starts = Vec::new();
    let mut index = 0;
    while index + old_lines.len() <= content_lines.len() {
        if block_matches(
            &content_lines[index..index + old_lines.len()],
            &old_lines,
            mode,
        ) {
            starts.push(index);
            index += old_lines.len();
        } else {
            index += 1;
        }
    }
    if starts.is_empty() {
        return Ok(None);
    }
    if starts.len() > 1 && !replace_all {
        return Err(ambiguous_message(mode.strategy(), starts.len()));
    }
    // 行级策略即便 replace_all 也只替换第一处：多处近似匹配同时替换风险太高。
    let start = starts[0];
    // 三种行级策略都按文件里命中块的首行缩进重排新内容，避免把模型侧的缩进带进文件。
    let replacement = reindent_new_lines(new, old_lines[0], content_lines[start]);
    let mut out: Vec<String> = content_lines[..start]
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    out.extend(replacement);
    out.extend(
        content_lines[start + old_lines.len()..]
            .iter()
            .map(|line| (*line).to_string()),
    );
    Ok(Some(EditOutcome {
        content: out.join("\n"),
        strategy: mode.strategy(),
        replacements: 1,
    }))
}

pub fn cap_model_output(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(max_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("Output truncated (showing last {max_chars} chars):\n{tail}")
}

/// 只保留末尾 `max_bytes`（按字符边界切）。
fn cap_bytes_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("[输出过长，已丢弃前 {} 字节]\n{}", start, &text[start..])
}

pub fn bash_model_chars() -> usize {
    BASH_MODEL_CHARS
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

async fn wait_cancel(flag: &CancelFlag) {
    loop {
        if flag.is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> LocalWorkspace {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("codex-ai-native-ws-{stamp}"));
        fs::create_dir_all(root.join("src")).expect("mkdir");
        fs::write(root.join("src/hello.txt"), "hello world\nsecond line\n").expect("write");
        LocalWorkspace::new(root)
    }

    #[test]
    fn read_write_edit_glob_grep_and_escape() {
        let ws = temp_workspace();
        let read = ws.read_file("src/hello.txt", None, None).expect("read");
        assert!(read.contains("hello world"));
        let edited = apply_edit("hello world", "hello", "goodbye", false).expect("edit");
        assert_eq!(edited, "goodbye world");
        ws.write_file("src/hello.txt", &edited).expect("write");
        let glob = ws.glob_files("**/*.txt", None).expect("glob");
        assert!(glob.contains("hello.txt"));
        let grep = ws
            .grep_files("goodbye", None, Some("**/*.txt"), None)
            .expect("grep");
        assert!(grep.contains("goodbye"));
        let regex_grep = ws
            .grep_files("good.ye w", None, None, None)
            .expect("regex grep");
        assert!(regex_grep.contains("goodbye world"));
        let error = ws.read_file("../secret.txt", None, None).unwrap_err();
        assert!(error.contains("超出工作区"));
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[test]
    fn missing_file_suggests_similar_name() {
        let ws = temp_workspace();
        let error = ws.read_file("src/helo.txt", None, None).unwrap_err();
        assert!(error.contains("文件不存在"));
        assert!(error.contains("hello.txt"), "{error}");
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[test]
    fn fingerprint_changes_when_content_changes() {
        let ws = temp_workspace();
        let path = ws.root.join("src/hello.txt");
        let first = FileFingerprint::of_path(&path).expect("fp");
        fs::write(&path, "changed").expect("write");
        let second = FileFingerprint::of_path(&path).expect("fp");
        assert_ne!(first, second);
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[test]
    fn read_image_returns_base64_payload() {
        let ws = temp_workspace();
        let png = [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
        fs::write(ws.root.join("src/pic.png"), png).expect("write");
        let image = ws.read_image("src/pic.png").expect("image");
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.name, "pic.png");
        assert!(!image.data_base64.is_empty());
        assert!(image_mime_type(Path::new("a.JPG")).is_some());
        assert!(image_mime_type(Path::new("a.rs")).is_none());
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[test]
    fn edit_strategy_quote_normalized() {
        let content = "say “hello” to ‘you’\n";
        let outcome =
            apply_edit_fuzzy(content, "say \"hello\" to 'you'", "bye", false).expect("edit");
        assert_eq!(outcome.strategy, "quote_normalized");
        assert_eq!(outcome.content, "bye\n");
    }

    #[test]
    fn edit_strategy_line_number_prefix_stripped() {
        let content = "fn main() {\n    println!(\"hi\");\n}\n";
        let old = "     1\tfn main() {\n     2\t    println!(\"hi\");\n     3\t}";
        let outcome = apply_edit_fuzzy(content, old, "fn main() {}", false).expect("edit");
        assert_eq!(outcome.strategy, "line_number_prefix_stripped");
        assert_eq!(outcome.content, "fn main() {}\n");
    }

    #[test]
    fn edit_strategy_escape_and_unicode() {
        let content = "a\tb\nc\n";
        let outcome = apply_edit_fuzzy(content, "a\\tb", "ab", false).expect("edit");
        assert_eq!(outcome.strategy, "escape_normalized");
        assert_eq!(outcome.content, "ab\nc\n");
        let content = "café\n";
        let outcome = apply_edit_fuzzy(content, "caf\\u00e9", "coffee", false).expect("edit");
        assert_eq!(outcome.strategy, "unicode_escape_normalized");
        assert_eq!(outcome.content, "coffee\n");
    }

    #[test]
    fn edit_strategy_line_trimmed_and_indentation() {
        let content = "if x {\n        do_it();  \n    }\n";
        let outcome = apply_edit_fuzzy(content, "do_it();\n}", "done();\n}", false).expect("edit");
        assert_eq!(outcome.strategy, "line_trimmed");
        assert!(outcome.content.contains("done();"));

        let content = "    fn a() {\n        one();\n    }\n";
        let outcome = apply_edit_fuzzy(
            content,
            "fn a() {\n    one();\n}",
            "fn a() {\n    two();\n}",
            false,
        )
        .expect("edit");
        assert_eq!(outcome.strategy, "indentation_flexible");
        assert_eq!(outcome.content, "    fn a() {\n        two();\n    }\n");
    }

    #[test]
    fn edit_strategy_block_anchor_tolerates_middle_drift() {
        let content = "start\nmid one\nmid two\nmid three\nend\n";
        let old = "start\nmid one\nmid TWO\nmid three\nend";
        let outcome = apply_edit_fuzzy(content, old, "replaced", false).expect("edit");
        assert_eq!(outcome.strategy, "block_anchor");
        assert_eq!(outcome.content, "replaced\n");
    }

    #[test]
    fn edit_preserves_crlf_and_reports_ambiguity() {
        let content = "a\r\nb\r\na\r\n";
        let error = apply_edit_fuzzy(content, "a", "c", false).unwrap_err();
        assert!(error.contains("not unique"));
        let outcome = apply_edit_fuzzy(content, "a", "c", true).expect("edit");
        assert_eq!(outcome.content, "c\r\nb\r\nc\r\n");
        assert_eq!(outcome.replacements, 2);
        let missing = apply_edit_fuzzy("x", "zzz", "y", false).unwrap_err();
        assert!(missing.contains("not found"));
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[tokio::test]
    async fn bash_echoes_in_workspace() {
        let ws = temp_workspace();
        let cancel = CancelFlag::new();
        let out = ws
            .bash("echo native-bash", None, &cancel, &[])
            .await
            .expect("bash");
        assert!(out.contains("native-bash"));
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[tokio::test]
    async fn bash_receives_extra_environment() {
        let ws = temp_workspace();
        let cancel = CancelFlag::new();
        let out = ws
            .bash(
                "printf '%s' \"$NOXCODE_PROXY_TEST\"",
                None,
                &cancel,
                &[("NOXCODE_PROXY_TEST".to_string(), "injected".to_string())],
            )
            .await
            .expect("bash");
        assert_eq!(out, "injected");
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[tokio::test]
    async fn bash_uses_shell_snapshot_when_present() {
        let mut ws = temp_workspace();
        let snapshot = ws.root.join("snapshot.sh");
        fs::write(&snapshot, "export NOXCODE_SNAPSHOT_MARK=from-snapshot\n").expect("write");
        ws.shell_snapshot = Some(snapshot);
        let cancel = CancelFlag::new();
        let out = ws
            .bash("printf '%s' \"$NOXCODE_SNAPSHOT_MARK\"", None, &cancel, &[])
            .await
            .expect("bash");
        assert_eq!(out, "from-snapshot");
        // 带引号与管道的命令经 eval 也保持语义。
        let quoted = ws
            .bash("echo \"a b\" | tr ' ' '-'", None, &cancel, &[])
            .await
            .expect("bash");
        assert_eq!(quoted.trim(), "a-b");
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[test]
    fn cap_bytes_tail_keeps_char_boundaries() {
        let text = "汉字".repeat(10);
        let capped = cap_bytes_tail(&text, 7);
        // 7 字节只放得下 2 个三字节汉字，且必须落在字符边界上。
        assert!(capped.starts_with("[输出过长"));
        let kept = capped.rsplit('\n').next().expect("tail");
        assert_eq!(kept, "汉字");
    }
}
