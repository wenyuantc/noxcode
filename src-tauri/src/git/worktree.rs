//! 会话级 git worktree 隔离。所有 git 调用走 [`super::runner`]。

use std::path::{Path, PathBuf};

use super::runner::{git, GitError, GitTarget, IndexMode};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: String,
    pub head: String,
    pub branch: String,
}

pub fn local_worktree_path(app_config: &Path, session_record_id: &str) -> PathBuf {
    app_config.join("worktrees").join(session_record_id.trim())
}

pub fn remote_worktree_path(session_record_id: &str) -> String {
    format!("$HOME/.noxcode/worktrees/{}", session_record_id.trim())
}

pub fn is_managed_worktree_path(path: &str, session_record_id: &str) -> bool {
    let path = path.trim();
    let id = session_record_id.trim();
    !id.is_empty()
        && path.contains("worktrees")
        && (path.ends_with(id) || path.contains(&format!("worktrees/{id}")))
}

pub async fn add_detached(target: &GitTarget, path: &str) -> Result<String, GitError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(GitError::Parse("worktree 路径不能为空".to_string()));
    }
    match target {
        GitTarget::Local(_) => {
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        GitTarget::Ssh { .. } => {}
    }
    let output = git(
        target,
        &["worktree", "add", "--detach", path],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["worktree", "add", "--detach", path])?;
    Ok(path.to_string())
}

pub async fn remove_worktree(target: &GitTarget, path: &str) -> Result<(), GitError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(GitError::Parse("worktree 路径不能为空".to_string()));
    }
    let output = git(
        target,
        &["worktree", "remove", "--force", path],
        &IndexMode::ReadOnly,
    )
    .await?;
    if !output.success() {
        let _ = git(target, &["worktree", "prune"], &IndexMode::ReadOnly).await;
        return Err(output.command_error(&["worktree", "remove", "--force", path]));
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn list_worktrees(target: &GitTarget) -> Result<Vec<WorktreeInfo>, GitError> {
    let output = git(
        target,
        &["worktree", "list", "--porcelain"],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["worktree", "list", "--porcelain"])?;
    Ok(parse_porcelain(&output.stdout_lossy()))
}

#[allow(dead_code)]
pub fn parse_porcelain(text: &str) -> Vec<WorktreeInfo> {
    let mut items = Vec::new();
    let mut path = String::new();
    let mut head = String::new();
    let mut branch = String::new();
    for line in text.lines() {
        if line.is_empty() {
            if !path.is_empty() {
                items.push(WorktreeInfo {
                    path: std::mem::take(&mut path),
                    head: std::mem::take(&mut head),
                    branch: std::mem::take(&mut branch),
                });
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            path = value.to_string();
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            head = value.to_string();
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = value.to_string();
        } else if line == "detached" {
            branch = "(detached)".to_string();
        }
    }
    if !path.is_empty() {
        items.push(WorktreeInfo { path, head, branch });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_path_matches_session_id() {
        assert!(is_managed_worktree_path("/cfg/worktrees/abc-1", "abc-1"));
        assert!(is_managed_worktree_path(
            "/home/u/.noxcode/worktrees/abc-1",
            "abc-1"
        ));
        assert!(!is_managed_worktree_path("/repo", "abc-1"));
        assert!(!is_managed_worktree_path("/cfg/worktrees/other", "abc-1"));
    }

    #[test]
    fn porcelain_parser_reads_detached_worktree() {
        let text = "\
worktree /repo
HEAD abc
branch refs/heads/main

worktree /tmp/wt
HEAD def
detached
";
        let items = parse_porcelain(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "/repo");
        assert_eq!(items[0].branch, "refs/heads/main");
        assert_eq!(items[1].path, "/tmp/wt");
        assert_eq!(items[1].branch, "(detached)");
    }
}
