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

pub fn normalize_path_for_compare(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

pub fn default_worktree_branch_name(session_id: &str) -> String {
    let short: String = session_id
        .chars()
        .filter(|item| item.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let short = if short.is_empty() {
        "session".to_string()
    } else {
        short
    };
    format!("noxcode/wt-{short}")
}

pub fn looks_like_session_id(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 64
        && name.chars().any(|ch| ch.is_ascii_hexdigit())
        && name.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
}

pub fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn default_local_worktree_root(app_config: &Path) -> PathBuf {
    user_home_dir()
        .map(|home| home.join(".noxcode").join("worktrees"))
        .unwrap_or_else(|| app_config.join("worktrees"))
}

pub fn resolve_local_worktree_root(app_config: &Path, configured_root: &str) -> PathBuf {
    let trimmed = configured_root.trim();
    if trimmed.is_empty() {
        default_local_worktree_root(app_config)
    } else {
        PathBuf::from(trimmed)
    }
}

pub fn local_worktree_path_in_root(root: &Path, session_record_id: &str) -> PathBuf {
    root.join(session_record_id.trim())
}

pub fn local_worktree_path(app_config: &Path, session_record_id: &str) -> PathBuf {
    local_worktree_path_in_root(&default_local_worktree_root(app_config), session_record_id)
}

pub fn remote_worktree_path(session_record_id: &str) -> String {
    format!("$HOME/.noxcode/worktrees/{}", session_record_id.trim())
}

pub fn session_id_from_worktree_path(path: &str) -> Option<String> {
    Path::new(path.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| looks_like_session_id(name))
        .map(ToOwned::to_owned)
}

#[allow(dead_code)]
pub fn is_managed_worktree_path(path: &str, session_record_id: &str) -> bool {
    is_managed_worktree_path_with_root(path, session_record_id, None)
}

pub fn is_managed_worktree_path_with_root(
    path: &str,
    session_record_id: &str,
    configured_root: Option<&str>,
) -> bool {
    let path = normalize_path_for_compare(path);
    let id = session_record_id.trim();
    if id.is_empty() || path.is_empty() {
        return false;
    }
    if path.contains("worktrees")
        && (path.ends_with(id) || path.contains(&format!("worktrees/{id}")))
    {
        return true;
    }
    if let Some(root) = configured_root
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let expected = format!("{}/{id}", normalize_path_for_compare(root));
        if path == expected {
            return true;
        }
    }
    false
}

pub fn is_any_managed_worktree_path(path: &str, configured_root: Option<&str>) -> bool {
    let Some(id) = session_id_from_worktree_path(path) else {
        return false;
    };
    is_managed_worktree_path_with_root(path, &id, configured_root)
}

/// 从托管 worktree 内的绝对文件路径拆出 worktree 根目录和相对路径。
#[allow(dead_code)]
pub fn split_managed_worktree_file_path(path: &str) -> Option<(String, String)> {
    split_managed_worktree_file_path_with_root(path, None)
}

pub fn split_managed_worktree_file_path_with_root(
    path: &str,
    configured_root: Option<&str>,
) -> Option<(String, String)> {
    let normalized = path.trim().replace('\\', "/");
    let marker = "/worktrees/";
    if let Some(start) = normalized.find(marker) {
        let after = &normalized[start + marker.len()..];
        if let Some((id, rest)) = after.split_once('/') {
            if !id.is_empty() && !rest.is_empty() {
                let root = normalized[..start + marker.len() + id.len()].to_string();
                return Some((root, rest.to_string()));
            }
        }
    }
    let root = configured_root
        .map(str::trim)
        .filter(|item| !item.is_empty())?;
    let root_norm = normalize_path_for_compare(root);
    let rest = normalized.strip_prefix(&format!("{root_norm}/"))?;
    let (id, file) = rest.split_once('/')?;
    if id.is_empty() || file.is_empty() || !looks_like_session_id(id) {
        return None;
    }
    Some((format!("{root_norm}/{id}"), file.to_string()))
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

pub async fn add_with_branch(
    target: &GitTarget,
    path: &str,
    branch: &str,
) -> Result<String, GitError> {
    let path = path.trim();
    let branch = branch.trim();
    if path.is_empty() {
        return Err(GitError::Parse("worktree 路径不能为空".to_string()));
    }
    if branch.is_empty() {
        return add_detached(target, path).await;
    }
    git(
        target,
        &["check-ref-format", "--branch", branch],
        &IndexMode::ReadOnly,
    )
    .await?
    .require_success(&["check-ref-format", "--branch", branch])?;
    match target {
        GitTarget::Local(_) => {
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        GitTarget::Ssh { .. } => {}
    }
    let created = git(
        target,
        &["worktree", "add", "-b", branch, path],
        &IndexMode::ReadOnly,
    )
    .await?;
    if created.success() {
        return Ok(path.to_string());
    }
    let attached = git(
        target,
        &["worktree", "add", path, branch],
        &IndexMode::ReadOnly,
    )
    .await?;
    if attached.success() {
        return Ok(path.to_string());
    }
    add_detached(target, path).await
}

pub async fn add_session_worktree(
    target: &GitTarget,
    path: &str,
    session_record_id: &str,
) -> Result<String, GitError> {
    add_with_branch(
        target,
        path,
        &default_worktree_branch_name(session_record_id),
    )
    .await
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

pub async fn fetch_all_prune(target: &GitTarget) -> Result<(), GitError> {
    let output = git(target, &["fetch", "--all", "--prune"], &IndexMode::ReadOnly).await?;
    output.require_success(&["fetch", "--all", "--prune"])?;
    Ok(())
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
        assert!(is_managed_worktree_path_with_root(
            "/data/nox-wt/abc-1",
            "abc-1",
            Some("/data/nox-wt")
        ));
        assert!(!is_managed_worktree_path_with_root(
            "/data/nox-wt/abc-1",
            "abc-1",
            None
        ));
        assert!(!is_managed_worktree_path_with_root(
            "/data/other/abc-1",
            "abc-1",
            Some("/data/nox-wt")
        ));
    }

    #[test]
    fn split_worktree_file_keeps_relative_path() {
        let (root, rel) = split_managed_worktree_file_path(
            "/Users/me/Library/Application Support/app/worktrees/abc-1/oms/Test.java",
        )
        .expect("split");
        assert_eq!(
            root,
            "/Users/me/Library/Application Support/app/worktrees/abc-1"
        );
        assert_eq!(rel, "oms/Test.java");
        assert!(split_managed_worktree_file_path("/repo/README.md").is_none());
        let (custom_root, custom_rel) = split_managed_worktree_file_path_with_root(
            "/data/nox-wt/abc-1/oms/Test.java",
            Some("/data/nox-wt"),
        )
        .expect("custom root");
        assert_eq!(custom_root, "/data/nox-wt/abc-1");
        assert_eq!(custom_rel, "oms/Test.java");
    }

    #[test]
    fn session_id_from_custom_root() {
        assert_eq!(
            session_id_from_worktree_path("/data/nox-wt/abc-1"),
            Some("abc-1".to_string())
        );
        assert!(session_id_from_worktree_path("/data/nox-wt/not a session").is_none());
    }

    #[test]
    fn default_branch_uses_session_prefix() {
        assert_eq!(
            default_worktree_branch_name("abc-def-123"),
            "noxcode/wt-abcdef12"
        );
        assert_eq!(
            default_worktree_branch_name("sess-git"),
            "noxcode/wt-sessgit"
        );
    }

    #[test]
    fn default_root_uses_home_noxcode() {
        let app_config = Path::new("/cfg");
        if let Some(home) = user_home_dir() {
            assert_eq!(
                default_local_worktree_root(app_config),
                home.join(".noxcode").join("worktrees")
            );
        } else {
            assert_eq!(
                default_local_worktree_root(app_config),
                PathBuf::from("/cfg/worktrees")
            );
        }
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
