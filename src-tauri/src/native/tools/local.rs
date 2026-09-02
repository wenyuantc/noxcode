use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use crate::process_spawn::tokio_command;

use super::cancel::CancelFlag;
use super::glob::glob_match;
use super::paths::resolve_under_workspace;

const READ_DEFAULT_LIMIT: usize = 2000;
const BASH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const BASH_MAX_TIMEOUT: Duration = Duration::from_secs(600);
const BASH_MODEL_CHARS: usize = 30_000;

#[derive(Debug, Clone)]
pub struct CommandStatus {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct LocalWorkspace {
    pub root: PathBuf,
}

impl LocalWorkspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn resolve(&self, input: &str) -> Result<PathBuf, String> {
        resolve_under_workspace(&self.root, input)
    }

    pub fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<String, String> {
        let resolved = self.resolve(path)?;
        let metadata = fs::metadata(&resolved).map_err(|_| format!("文件不存在: {path}"))?;
        if metadata.is_dir() {
            return Err(format!("路径是目录: {path}"));
        }
        let content =
            fs::read_to_string(&resolved).map_err(|error| format!("读取失败: {error}"))?;
        Ok(format_read(&content, offset, limit))
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<String, String> {
        let resolved = self.resolve(path)?;
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
        let metadata = fs::metadata(&resolved).map_err(|_| format!("文件不存在: {path}"))?;
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
                if line.contains(pattern) {
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
    ) -> Result<String, String> {
        let status = self
            .bash_with_status(command, timeout_ms, cancel, &[])
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
        extra_env: &[(&str, String)],
    ) -> Result<CommandStatus, String> {
        if command.trim().is_empty() {
            return Err("command 不能为空".to_string());
        }
        if cancel.is_cancelled() {
            return Err("已取消".to_string());
        }
        let timeout = Duration::from_millis(
            timeout_ms
                .unwrap_or(BASH_DEFAULT_TIMEOUT.as_millis() as i64)
                .clamp(1, BASH_MAX_TIMEOUT.as_millis() as i64) as u64,
        );
        let mut cmd = tokio_command("bash");
        cmd.arg("-lc")
            .arg(command)
            .current_dir(&self.root)
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
        let text = cap_model_output(&text, BASH_MODEL_CHARS);
        Ok(CommandStatus {
            exit_code: status.code().unwrap_or(-1),
            output: text,
            timed_out: false,
        })
    }
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

pub fn apply_edit(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<String, String> {
    if old == new {
        return Err("old_string and new_string must be different".to_string());
    }
    let count = content.matches(old).count();
    if count == 0 {
        return Err("old_string not found in file".to_string());
    }
    if !replace_all && count > 1 {
        return Err(
            "old_string is not unique; use replace_all or provide more context".to_string(),
        );
    }
    Ok(if replace_all {
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    })
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
        let error = ws.read_file("../secret.txt", None, None).unwrap_err();
        assert!(error.contains("超出工作区"));
        let _ = fs::remove_dir_all(&ws.root);
    }

    #[tokio::test]
    async fn bash_echoes_in_workspace() {
        let ws = temp_workspace();
        let cancel = CancelFlag::new();
        let out = ws
            .bash("echo native-bash", None, &cancel)
            .await
            .expect("bash");
        assert!(out.contains("native-bash"));
        let _ = fs::remove_dir_all(&ws.root);
    }
}
