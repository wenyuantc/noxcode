//! 把会话隔离 worktree 合并回主工作区。所有 git 走 [`super::runner`]。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::checkpoint::create_checkpoint;
use super::repo::in_progress_operation;
use super::runner::{
    assert_safe_rel_path, git, git_with, with_repo_lock, GitError, GitRunOptions, GitTarget,
    IndexMode,
};
use super::worktree::{is_managed_worktree_path_with_root, list_worktrees};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeWorktreeAction {
    MergeCurrent,
    CreateBranch,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveWorktreeAction {
    Ai,
    Abort,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeWorktreeStatus {
    Merged,
    Branched,
    Kept,
    Conflicted,
    Aborted,
    Resolved,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeWorktreeResult {
    pub status: MergeWorktreeStatus,
    pub branch: Option<String>,
    pub commit_oid: Option<String>,
    pub conflicts: Vec<String>,
    pub resolved: Vec<String>,
    pub failed: Vec<String>,
    pub message: String,
}

impl MergeWorktreeResult {
    fn kept() -> Self {
        Self {
            status: MergeWorktreeStatus::Kept,
            branch: None,
            commit_oid: None,
            conflicts: Vec::new(),
            resolved: Vec::new(),
            failed: Vec::new(),
            message: "已保留隔离工作树，未改动主工作区".to_string(),
        }
    }
}

pub fn merge_checkpoint_label(message: Option<&str>) -> &str {
    message
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("worktree_merge")
}

pub use super::worktree::default_worktree_branch_name;

pub fn has_conflict_markers(text: &str) -> bool {
    text.contains("<<<<<<<") || text.contains(">>>>>>>")
}

pub fn sanitize_conflict_resolution(raw: &str) -> Result<String, String> {
    let mut text = raw.trim().to_string();
    if text.starts_with("```") {
        let mut lines = text.lines();
        let _ = lines.next();
        let mut body: Vec<&str> = lines.collect();
        if body
            .last()
            .is_some_and(|line| line.trim().starts_with("```"))
        {
            body.pop();
        }
        text = body.join("\n");
    }
    let text = text.trim_end().to_string();
    if text.is_empty() {
        return Err("模型未返回可用的冲突解决结果".to_string());
    }
    if has_conflict_markers(&text) {
        return Err("模型输出仍含冲突标记".to_string());
    }
    Ok(if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    })
}

pub fn conflict_resolve_prompt(path: &str, content: &str) -> String {
    format!(
        "下面是 Git 合并冲突文件 `{path}` 的完整内容，含 <<<<<<< / ======= / >>>>>>> 标记。\n\
请输出解决冲突后的完整文件内容。\n\
只输出文件正文，不要解释，不要 Markdown 代码围栏，不要再留下冲突标记。\n\n\
{content}"
    )
}

pub async fn list_unmerged_paths(target: &GitTarget) -> Result<Vec<String>, GitError> {
    let output = git(
        target,
        &["diff", "--name-only", "--diff-filter=U", "-z"],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["diff", "--name-only", "--diff-filter=U"])?;
    Ok(output
        .stdout_lossy()
        .split('\0')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub async fn merge_in_progress(target: &GitTarget) -> Result<bool, GitError> {
    Ok(in_progress_operation(target).await?.as_deref() == Some("merge"))
}

pub async fn abort_merge(target: &GitTarget) -> Result<(), GitError> {
    with_repo_lock(target, || async {
        git(target, &["merge", "--abort"], &IndexMode::user())
            .await?
            .require_success(&["merge", "--abort"])?;
        Ok(())
    })
    .await
}

async fn current_branch_name(target: &GitTarget) -> Result<Option<String>, GitError> {
    let output = git(
        target,
        &["rev-parse", "--abbrev-ref", "HEAD"],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let name = output.stdout_lossy().trim().to_string();
    if name.is_empty() || name == "HEAD" {
        Ok(None)
    } else {
        Ok(Some(name))
    }
}

async fn reset_hard_to(target: &GitTarget, oid: &str) -> Result<(), GitError> {
    with_repo_lock(target, || async {
        git(target, &["reset", "--hard", oid], &IndexMode::user())
            .await?
            .require_success(&["reset", "--hard"])?;
        Ok(())
    })
    .await
}

async fn switch_discard_changes(target: &GitTarget, name: &str) -> Result<(), GitError> {
    with_repo_lock(target, || async {
        git(
            target,
            &["switch", "--discard-changes", name],
            &IndexMode::user(),
        )
        .await?
        .require_success(&["switch", "--discard-changes", name])?;
        Ok(())
    })
    .await
}

async fn branch_is_checked_out(target: &GitTarget, name: &str) -> Result<bool, GitError> {
    let expected = format!("refs/heads/{name}");
    let items = list_worktrees(target).await?;
    Ok(items
        .iter()
        .any(|item| item.branch == expected || item.branch == name))
}

async fn create_branch_at(target: &GitTarget, name: &str, oid: &str) -> Result<String, GitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GitError::Parse("分支名不能为空".to_string()));
    }
    git(
        target,
        &["check-ref-format", "--branch", name],
        &IndexMode::ReadOnly,
    )
    .await?
    .require_success(&["check-ref-format", "--branch", name])?;

    let exists = git(
        target,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    if exists.success() {
        if branch_is_checked_out(target, name).await? {
            return Err(GitError::Blocked(format!(
                "分支 {name} 已在工作区检出，无法改写。请换一个新分支名，或使用「合并回当前分支」。"
            )));
        }
        git(target, &["branch", "-f", name, oid], &IndexMode::ReadOnly)
            .await?
            .require_success(&["branch", "-f", name, oid])?;
    } else {
        git(target, &["branch", name, oid], &IndexMode::ReadOnly)
            .await?
            .require_success(&["branch", name, oid])?;
    }
    Ok(name.to_string())
}

async fn merge_named_branch(
    target: &GitTarget,
    branch: &str,
    commit_message: Option<&str>,
) -> Result<MergeWorktreeResult, GitError> {
    let label = merge_checkpoint_label(commit_message);
    let output = with_repo_lock(target, || async {
        git(
            target,
            &["merge", "--no-edit", "-m", label, branch],
            &IndexMode::user(),
        )
        .await
    })
    .await?;
    if output.success() {
        let oid = git(target, &["rev-parse", "HEAD"], &IndexMode::ReadOnly)
            .await?
            .require_success(&["rev-parse", "HEAD"])?
            .stdout_lossy()
            .trim()
            .to_string();
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Merged,
            branch: Some(branch.to_string()),
            commit_oid: Some(oid),
            conflicts: Vec::new(),
            resolved: Vec::new(),
            failed: Vec::new(),
            message: format!("已合并隔离工作树分支 {branch}"),
        });
    }
    if merge_in_progress(target).await? {
        let conflicts = list_unmerged_paths(target).await?;
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Conflicted,
            branch: Some(branch.to_string()),
            commit_oid: None,
            conflicts,
            resolved: Vec::new(),
            failed: Vec::new(),
            message: "合并存在冲突，请选择 AI 自动解决或手动解决".to_string(),
        });
    }
    Err(output.command_error(&["merge", "--no-edit", branch]))
}

pub async fn read_worktree_text(target: &GitTarget, path: &str) -> Result<String, GitError> {
    assert_safe_rel_path(path)?;
    let oid = git(
        target,
        &["hash-object", "-w", "--", path],
        &IndexMode::ReadOnly,
    )
    .await?
    .require_success(&["hash-object", "-w", "--", path])?
    .stdout_lossy()
    .trim()
    .to_string();
    if oid.is_empty() {
        return Err(GitError::Parse(format!("无法读取冲突文件: {path}")));
    }
    let output = git(target, &["cat-file", "-p", &oid], &IndexMode::ReadOnly).await?;
    output.require_success(&["cat-file", "-p"])?;
    let text = output.stdout_lossy();
    if text.contains('\0') {
        return Err(GitError::Parse(format!("无法处理二进制冲突文件: {path}")));
    }
    Ok(text)
}

pub async fn write_and_stage_text(
    target: &GitTarget,
    path: &str,
    content: &str,
) -> Result<(), GitError> {
    assert_safe_rel_path(path)?;
    let bytes = content.as_bytes().to_vec();
    let oid = git_with(
        target,
        &["hash-object", "-w", "--stdin"],
        &IndexMode::ReadOnly,
        GitRunOptions {
            timeout: None,
            stdin: Some(bytes),
            extra_env: Vec::new(),
        },
    )
    .await?
    .require_success(&["hash-object", "-w", "--stdin"])?
    .stdout_lossy()
    .trim()
    .to_string();
    if oid.is_empty() {
        return Err(GitError::Parse(format!("写入对象失败: {path}")));
    }
    let cacheinfo = format!("100644,{oid},{path}");
    with_repo_lock(target, || async {
        git(
            target,
            &["update-index", "--add", "--cacheinfo", &cacheinfo],
            &IndexMode::user(),
        )
        .await?
        .require_success(&["update-index", "--add", "--cacheinfo"])?;
        git(
            target,
            &["checkout-index", "-f", "--", path],
            &IndexMode::user(),
        )
        .await?
        .require_success(&["checkout-index", "-f", "--", path])?;
        Ok(())
    })
    .await
}

pub async fn commit_merge(target: &GitTarget) -> Result<String, GitError> {
    with_repo_lock(target, || async {
        git(target, &["commit", "--no-edit"], &IndexMode::user())
            .await?
            .require_success(&["commit", "--no-edit"])?;
        let oid = git(target, &["rev-parse", "HEAD"], &IndexMode::ReadOnly)
            .await?
            .require_success(&["rev-parse", "HEAD"])?
            .stdout_lossy()
            .trim()
            .to_string();
        Ok(oid)
    })
    .await
}

pub async fn apply_resolved_files(
    target: &GitTarget,
    resolutions: &[(String, Result<String, String>)],
) -> Result<MergeWorktreeResult, GitError> {
    let mut resolved = Vec::new();
    let mut failed = Vec::new();
    for (path, outcome) in resolutions {
        match outcome {
            Ok(content) => match write_and_stage_text(target, path, content).await {
                Ok(()) => resolved.push(path.clone()),
                Err(error) => failed.push(format!("{path}: {error}")),
            },
            Err(error) => failed.push(format!("{path}: {error}")),
        }
    }
    if !failed.is_empty() {
        let conflicts = list_unmerged_paths(target).await?;
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Partial,
            branch: None,
            commit_oid: None,
            conflicts,
            resolved,
            failed,
            message: "部分冲突未能自动解决，合并仍停在中间态".to_string(),
        });
    }
    let remaining = list_unmerged_paths(target).await?;
    if !remaining.is_empty() {
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Partial,
            branch: None,
            commit_oid: None,
            conflicts: remaining,
            resolved,
            failed,
            message: "仍有未合并文件，合并停在中间态".to_string(),
        });
    }
    let oid = commit_merge(target).await?;
    Ok(MergeWorktreeResult {
        status: MergeWorktreeStatus::Resolved,
        branch: None,
        commit_oid: Some(oid),
        conflicts: Vec::new(),
        resolved,
        failed: Vec::new(),
        message: "已用自动解决完成合并".to_string(),
    })
}

pub async fn complete_merge_from_worktree(
    target: &GitTarget,
) -> Result<MergeWorktreeResult, GitError> {
    if !merge_in_progress(target).await? {
        return Err(GitError::Parse("当前没有进行中的合并".to_string()));
    }
    let conflicts = list_unmerged_paths(target).await?;
    if conflicts.is_empty() {
        let oid = commit_merge(target).await?;
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Resolved,
            branch: None,
            commit_oid: Some(oid),
            conflicts: Vec::new(),
            resolved: Vec::new(),
            failed: Vec::new(),
            message: "已用自动解决完成合并".to_string(),
        });
    }
    let mut resolutions = Vec::new();
    for path in conflicts {
        let outcome = match read_worktree_text(target, &path).await {
            Ok(content) => sanitize_conflict_resolution(&content),
            Err(error) => Err(error.to_string()),
        };
        resolutions.push((path, outcome));
    }
    apply_resolved_files(target, &resolutions).await
}

async fn require_managed_worktree(
    pool: &SqlitePool,
    workspace_id: &str,
    session_id: &str,
    configured_root: Option<&str>,
) -> Result<String, GitError> {
    let working_dir: Option<String> =
        sqlx::query_scalar("SELECT working_dir FROM agent_sessions WHERE id = $1 LIMIT 1")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| GitError::Parse(format!("读取会话失败: {error}")))?
            .flatten();
    let working_dir = working_dir
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .ok_or_else(|| GitError::Parse("会话没有隔离工作树目录".to_string()))?;
    let session_workspace: Option<String> =
        sqlx::query_scalar("SELECT workspace_id FROM agent_sessions WHERE id = $1 LIMIT 1")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| GitError::Parse(format!("读取会话失败: {error}")))?
            .flatten();
    if session_workspace.as_deref() != Some(workspace_id) {
        return Err(GitError::Parse(
            "session_id 与 workspace_id 不匹配".to_string(),
        ));
    }
    if !is_managed_worktree_path_with_root(&working_dir, session_id, configured_root) {
        return Err(GitError::Parse("当前会话未使用托管隔离工作树".to_string()));
    }
    Ok(working_dir)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_merge_session_worktree(
    pool: &SqlitePool,
    main: &GitTarget,
    worktree: &GitTarget,
    workspace_id: &str,
    session_id: &str,
    action: MergeWorktreeAction,
    branch_name: Option<&str>,
    commit_message: Option<&str>,
    configured_root: Option<&str>,
) -> Result<MergeWorktreeResult, GitError> {
    if action == MergeWorktreeAction::Keep {
        return Ok(MergeWorktreeResult::kept());
    }
    if let Some(operation) = in_progress_operation(main).await? {
        return Err(GitError::Blocked(format!(
            "主工作区正在进行 {operation}，请先处理后再合并"
        )));
    }
    let _working_dir =
        require_managed_worktree(pool, workspace_id, session_id, configured_root).await?;
    let label = merge_checkpoint_label(commit_message);
    let checkpoint = create_checkpoint(
        pool,
        worktree,
        workspace_id,
        session_id,
        Some(label),
        Some("manual"),
    )
    .await?;
    if action == MergeWorktreeAction::CreateBranch {
        let branch = branch_name
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_worktree_branch_name(session_id));
        if current_branch_name(worktree).await?.as_deref() == Some(branch.as_str()) {
            reset_hard_to(worktree, &checkpoint.commit_oid).await?;
            return Ok(MergeWorktreeResult {
                status: MergeWorktreeStatus::Branched,
                branch: Some(branch.clone()),
                commit_oid: Some(checkpoint.commit_oid),
                conflicts: Vec::new(),
                resolved: Vec::new(),
                failed: Vec::new(),
                message: format!("已更新分支 {branch}，隔离工作树已提交并清空，主工作区未改动"),
            });
        }
        let branch = create_branch_at(main, &branch, &checkpoint.commit_oid).await?;
        switch_discard_changes(worktree, &branch).await?;
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Branched,
            branch: Some(branch.clone()),
            commit_oid: Some(checkpoint.commit_oid),
            conflicts: Vec::new(),
            resolved: Vec::new(),
            failed: Vec::new(),
            message: format!("已创建分支 {branch}，隔离工作树已切换到该分支，主工作区未改动"),
        });
    }
    // 隔离树已检出会话分支时，在该工作树内 reset 到打点提交（同步 index），再合并回主工作区。
    // 旧的 detached 工作树仍建内部指针，绝不 force-update 已检出分支。
    if let Some(worktree_branch) = current_branch_name(worktree).await? {
        reset_hard_to(worktree, &checkpoint.commit_oid).await?;
        return merge_named_branch(main, &worktree_branch, Some(label)).await;
    }
    let pointer = default_worktree_branch_name(session_id);
    let pointer = create_branch_at(main, &pointer, &checkpoint.commit_oid).await?;
    reset_hard_to(worktree, &checkpoint.commit_oid).await?;
    merge_named_branch(main, &pointer, Some(label)).await
}

pub async fn run_abort_merge(target: &GitTarget) -> Result<MergeWorktreeResult, GitError> {
    if !merge_in_progress(target).await? {
        return Ok(MergeWorktreeResult {
            status: MergeWorktreeStatus::Aborted,
            branch: None,
            commit_oid: None,
            conflicts: Vec::new(),
            resolved: Vec::new(),
            failed: Vec::new(),
            message: "当前没有进行中的合并".to_string(),
        });
    }
    abort_merge(target).await?;
    Ok(MergeWorktreeResult {
        status: MergeWorktreeStatus::Aborted,
        branch: None,
        commit_oid: None,
        conflicts: Vec::new(),
        resolved: Vec::new(),
        failed: Vec::new(),
        message: "已中止合并，主工作区回到合并前".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_branch_uses_session_prefix() {
        assert_eq!(
            default_worktree_branch_name("abc-def-123"),
            "noxcode/wt-abcdef12"
        );
    }

    #[test]
    fn merge_label_uses_message_or_fallback() {
        assert_eq!(
            merge_checkpoint_label(Some(" feat: add status ")),
            "feat: add status"
        );
        assert_eq!(merge_checkpoint_label(Some("   ")), "worktree_merge");
        assert_eq!(merge_checkpoint_label(None), "worktree_merge");
    }

    #[test]
    fn conflict_markers_and_sanitize() {
        assert!(has_conflict_markers(
            "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> x\n"
        ));
        assert!(!has_conflict_markers("plain file\n"));
        let cleaned = sanitize_conflict_resolution("```\nhello\n```\n").expect("clean");
        assert_eq!(cleaned, "hello\n");
        assert!(sanitize_conflict_resolution("<<<<<<< HEAD\n").is_err());
    }
}
