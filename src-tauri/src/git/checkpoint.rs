use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::app::shared::{new_id, now_sqlite};
use crate::db::models::GitCheckpoint;

use super::diff::{name_status_against, NameStatusEntry};
use super::repo::{head_oid, in_progress_operation, is_detached_head, rev_parse_verify};
use super::runner::{
    git, git_with, remove_worktree_path, split_nul_strings, with_repo_lock, GitError, GitRunOptions,
    GitTarget, IndexMode, ScratchIndex,
};

const CHECKPOINT_AUTHOR_NAME: &str = "noxcode";
const CHECKPOINT_AUTHOR_EMAIL: &str = "noxcode@local";
const RESTORE_ARG_BUDGET: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCheckpointInfo {
    pub id: String,
    pub session_id: String,
    pub workspace_id: String,
    pub seq: i64,
    pub ref_name: String,
    pub commit_oid: String,
    pub parent_oid: Option<String>,
    pub label: Option<String>,
    pub kind: String,
    pub created_at: String,
    pub ref_valid: bool,
}

impl GitCheckpointInfo {
    fn from_row(row: GitCheckpoint, refs: &HashMap<String, String>) -> Self {
        let ref_valid = refs
            .get(&row.ref_name)
            .is_some_and(|oid| oid == &row.commit_oid);
        Self {
            id: row.id,
            session_id: row.session_id,
            workspace_id: row.workspace_id,
            seq: row.seq,
            ref_name: row.ref_name,
            commit_oid: row.commit_oid,
            parent_oid: row.parent_oid,
            label: row.label,
            kind: row.kind,
            created_at: row.created_at,
            ref_valid,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRestorePreview {
    pub checkpoint_id: String,
    pub blocked_reason: Option<String>,
    pub warnings: Vec<String>,
    pub will_overwrite: Vec<String>,
    pub will_recreate: Vec<String>,
    pub wont_be_touched: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRestoreResult {
    pub pre_restore_checkpoint: GitCheckpointInfo,
    pub restored: Vec<String>,
    pub deleted: Vec<String>,
    pub skipped_ignored: Vec<String>,
    pub failed: Vec<String>,
}

pub(crate) fn normalize_kind(kind: Option<&str>) -> Result<String, GitError> {
    match kind.unwrap_or("manual") {
        value @ ("session_start" | "after_tool_call" | "manual" | "auto_pre_restore") => {
            Ok(value.to_string())
        }
        other => Err(GitError::Parse(format!("无效的 checkpoint kind: {other}"))),
    }
}

pub(crate) fn checkpoint_ref_name(session_id: &str, seq: i64) -> String {
    format!("refs/noxcode/checkpoints/{session_id}/{seq}")
}

pub(crate) async fn create_checkpoint(
    pool: &SqlitePool,
    target: &GitTarget,
    workspace_id: &str,
    session_id: &str,
    label: Option<&str>,
    kind: Option<&str>,
) -> Result<GitCheckpointInfo, GitError> {
    let kind = normalize_kind(kind)?;
    ensure_session_workspace(pool, session_id, workspace_id).await?;
    with_repo_lock(target, || async {
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM git_checkpoints WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await
        .map_err(|error| GitError::Parse(format!("读取 checkpoint 序号失败: {error}")))?;

        let scratch = ScratchIndex::from_user_index_copy(target).await?;
        let created = create_objects(target, &scratch, label, seq).await;
        let _ = scratch.cleanup().await;
        let (commit_oid, parent_oid, used_label) = created?;

        let ref_name = checkpoint_ref_name(session_id, seq);
        git(
            target,
            &["update-ref", &ref_name, &commit_oid],
            &IndexMode::ReadOnly,
        )
        .await?
        .require_success(&["update-ref"])?;
        ensure_exclude_decoration(target).await?;

        let id = new_id();
        let created_at = now_sqlite();
        sqlx::query(
            r#"
            INSERT INTO git_checkpoints (
                id, session_id, workspace_id, seq, ref_name, commit_oid, parent_oid, label, kind, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(&id)
        .bind(session_id)
        .bind(workspace_id)
        .bind(seq)
        .bind(&ref_name)
        .bind(&commit_oid)
        .bind(&parent_oid)
        .bind(&used_label)
        .bind(&kind)
        .bind(&created_at)
        .execute(pool)
        .await
        .map_err(|error| GitError::Parse(format!("写入 git_checkpoints 失败: {error}")))?;

        Ok(GitCheckpointInfo {
            id,
            session_id: session_id.to_string(),
            workspace_id: workspace_id.to_string(),
            seq,
            ref_name,
            commit_oid,
            parent_oid,
            label: Some(used_label),
            kind,
            created_at,
            ref_valid: true,
        })
    })
    .await
}

async fn create_objects(
    target: &GitTarget,
    scratch: &ScratchIndex,
    label: Option<&str>,
    seq: i64,
) -> Result<(String, Option<String>, String), GitError> {
    let mode = IndexMode::Scratch(scratch.clone());
    git(target, &["add", "-A"], &mode)
        .await?
        .require_success(&["add", "-A"])?;
    let tree = git(target, &["write-tree"], &mode)
        .await?
        .require_success(&["write-tree"])?
        .stdout_lossy()
        .trim()
        .to_string();
    let parent_oid = head_oid(target).await?;
    let used_label = label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("checkpoint#{seq}"));
    let mut args = vec!["commit-tree".to_string(), tree];
    if let Some(parent) = parent_oid.as_deref() {
        args.push("-p".to_string());
        args.push(parent.to_string());
    }
    args.push("-m".to_string());
    args.push(used_label.clone());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let commit = git_with(
        target,
        &refs,
        &mode,
        GitRunOptions {
            timeout: None,
            stdin: None,
            extra_env: vec![
                ("GIT_AUTHOR_NAME".to_string(), CHECKPOINT_AUTHOR_NAME.to_string()),
                (
                    "GIT_AUTHOR_EMAIL".to_string(),
                    CHECKPOINT_AUTHOR_EMAIL.to_string(),
                ),
                (
                    "GIT_COMMITTER_NAME".to_string(),
                    CHECKPOINT_AUTHOR_NAME.to_string(),
                ),
                (
                    "GIT_COMMITTER_EMAIL".to_string(),
                    CHECKPOINT_AUTHOR_EMAIL.to_string(),
                ),
            ],
        },
    )
    .await?
    .require_success(&refs)?
    .stdout_lossy()
    .trim()
    .to_string();
    Ok((commit, parent_oid, used_label))
}

async fn ensure_exclude_decoration(target: &GitTarget) -> Result<(), GitError> {
    let output = git(
        target,
        &["config", "--get-all", "log.excludeDecoration"],
        &IndexMode::ReadOnly,
    )
    .await?;
    let has_rule = output
        .stdout_lossy()
        .lines()
        .any(|line| line.trim() == "refs/noxcode/");
    if has_rule {
        return Ok(());
    }
    git(
        target,
        &["config", "--add", "log.excludeDecoration", "refs/noxcode/"],
        &IndexMode::ReadOnly,
    )
    .await?
    .require_success(&["config", "--add"])?;
    Ok(())
}

async fn ensure_session_workspace(
    pool: &SqlitePool,
    session_id: &str,
    workspace_id: &str,
) -> Result<(), GitError> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT workspace_id FROM agent_sessions WHERE id = $1 LIMIT 1")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| GitError::Parse(format!("读取会话失败: {error}")))?;
    let Some((session_workspace,)) = row else {
        return Err(GitError::Parse(format!("会话不存在: {session_id}")));
    };
    if session_workspace.as_deref() != Some(workspace_id) {
        return Err(GitError::Parse(
            "session_id 与 workspace_id 不匹配".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn list_checkpoints(
    pool: &SqlitePool,
    target: &GitTarget,
    workspace_id: &str,
    session_id: &str,
) -> Result<Vec<GitCheckpointInfo>, GitError> {
    let rows = sqlx::query_as::<_, GitCheckpoint>(
        "SELECT * FROM git_checkpoints WHERE workspace_id = $1 AND session_id = $2 ORDER BY seq",
    )
    .bind(workspace_id)
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(|error| GitError::Parse(format!("列出 checkpoints 失败: {error}")))?;
    let refs = list_checkpoint_refs(target, Some(session_id)).await?;
    Ok(rows
        .into_iter()
        .map(|row| GitCheckpointInfo::from_row(row, &refs))
        .collect())
}

async fn list_checkpoint_refs(
    target: &GitTarget,
    session_id: Option<&str>,
) -> Result<HashMap<String, String>, GitError> {
    let pattern = match session_id {
        Some(session_id) => format!("refs/noxcode/checkpoints/{session_id}"),
        None => "refs/noxcode/checkpoints".to_string(),
    };
    let output = git(
        target,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            &pattern,
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    if !output.success() {
        return Ok(HashMap::new());
    }
    let mut map = HashMap::new();
    for line in output.stdout_lossy().lines() {
        let mut parts = line.split('\0');
        if let (Some(name), Some(oid)) = (parts.next(), parts.next()) {
            if !name.is_empty() {
                map.insert(name.to_string(), oid.to_string());
            }
        }
    }
    Ok(map)
}

async fn load_checkpoint(
    pool: &SqlitePool,
    checkpoint_id: &str,
) -> Result<GitCheckpoint, GitError> {
    sqlx::query_as::<_, GitCheckpoint>("SELECT * FROM git_checkpoints WHERE id = $1 LIMIT 1")
        .bind(checkpoint_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| GitError::Parse(format!("读取 checkpoint 失败: {error}")))?
        .ok_or_else(|| GitError::Parse(format!("检查点不存在: {checkpoint_id}")))
}

pub(crate) async fn preview_restore(
    pool: &SqlitePool,
    target: &GitTarget,
    checkpoint_id: &str,
) -> Result<GitRestorePreview, GitError> {
    let row = load_checkpoint(pool, checkpoint_id).await?;
    let (blocked_reason, warnings) = validate_restore(target, &row).await?;
    if let Some(reason) = blocked_reason.clone() {
        return Ok(GitRestorePreview {
            checkpoint_id: checkpoint_id.to_string(),
            blocked_reason: Some(reason),
            warnings,
            will_overwrite: Vec::new(),
            will_recreate: Vec::new(),
            wont_be_touched: Vec::new(),
        });
    }
    let impact = compute_impact(target, &row.commit_oid).await?;
    Ok(GitRestorePreview {
        checkpoint_id: checkpoint_id.to_string(),
        blocked_reason: None,
        warnings,
        will_overwrite: impact.will_overwrite,
        will_recreate: impact.will_recreate,
        wont_be_touched: impact.wont_be_touched,
    })
}

async fn validate_restore(
    target: &GitTarget,
    row: &GitCheckpoint,
) -> Result<(Option<String>, Vec<String>), GitError> {
    let mut warnings = Vec::new();
    let peel = format!("{}^{{commit}}", row.commit_oid);
    let object_ok = git(target, &["cat-file", "-e", &peel], &IndexMode::ReadOnly)
        .await?
        .success();
    if !object_ok {
        return Ok((
            Some("检查点已失效（可能被 git gc 清理或 ref 被手工删除）".to_string()),
            warnings,
        ));
    }
    match rev_parse_verify(target, &row.ref_name).await? {
        Some(oid) if oid == row.commit_oid => {}
        Some(_) => {
            return Ok((
                Some("检查点 ref 与数据库不一致".to_string()),
                warnings,
            ));
        }
        None => {
            return Ok((
                Some("检查点已失效（可能被 git gc 清理或 ref 被手工删除）".to_string()),
                warnings,
            ));
        }
    }
    if let Some(operation) = in_progress_operation(target).await? {
        return Ok((
            Some(format!("请先完成或中止当前 {operation}")),
            warnings,
        ));
    }
    if is_detached_head(target).await? {
        warnings.push("当前处于 detached HEAD".to_string());
    }
    Ok((None, warnings))
}

struct RestoreImpact {
    will_overwrite: Vec<String>,
    will_recreate: Vec<String>,
    wont_be_touched: Vec<String>,
}

async fn compute_impact(target: &GitTarget, oid: &str) -> Result<RestoreImpact, GitError> {
    let changes = name_status_against(target, oid).await?;
    let mut will_overwrite = Vec::new();
    let mut will_recreate = Vec::new();
    let mut wont_be_touched = Vec::new();
    for NameStatusEntry { status, path, .. } in changes {
        match status.as_str() {
            "D" => will_recreate.push(path),
            "A" => wont_be_touched.push(path),
            "M" | "T" => will_overwrite.push(path),
            _ => will_overwrite.push(path),
        }
    }

    let tree_paths = checkpoint_tree_paths(target, oid).await?;
    let untracked = git(
        target,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        &IndexMode::ReadOnly,
    )
    .await?;
    untracked.require_success(&["ls-files"])?;
    for path in split_nul_strings(&untracked.stdout) {
        if !tree_paths.contains(&path) && !wont_be_touched.contains(&path) {
            wont_be_touched.push(path);
        }
    }

    Ok(RestoreImpact {
        will_overwrite,
        will_recreate,
        wont_be_touched,
    })
}

async fn checkpoint_tree_paths(target: &GitTarget, oid: &str) -> Result<HashSet<String>, GitError> {
    let output = git(
        target,
        &["ls-tree", "-r", "--name-only", "-z", oid],
        &IndexMode::ReadOnly,
    )
    .await?;
    output.require_success(&["ls-tree"])?;
    Ok(split_nul_strings(&output.stdout).into_iter().collect())
}

pub(crate) async fn restore_checkpoint(
    pool: &SqlitePool,
    target: &GitTarget,
    workspace_id: &str,
    checkpoint_id: &str,
    delete_new_paths: &[String],
) -> Result<GitRestoreResult, GitError> {
    let preview = preview_restore(pool, target, checkpoint_id).await?;
    if let Some(reason) = preview.blocked_reason {
        return Err(GitError::Blocked(reason));
    }
    let allowed: HashSet<&str> = preview
        .wont_be_touched
        .iter()
        .map(String::as_str)
        .collect();
    let ignored_untracked = ignored_untracked_paths(target).await?;
    for path in delete_new_paths {
        super::runner::assert_safe_rel_path(path)?;
        if !allowed.contains(path.as_str()) && !ignored_untracked.contains(path) {
            return Err(GitError::Parse(format!(
                "只能删除检查点之后新建的文件: {path}"
            )));
        }
    }

    let row = load_checkpoint(pool, checkpoint_id).await?;
    let pre = create_checkpoint(
        pool,
        target,
        workspace_id,
        &row.session_id,
        Some(&format!("回滚前自动快照（目标：#{}）", row.seq)),
        Some("auto_pre_restore"),
    )
    .await?;

    let restore_paths: Vec<String> = preview
        .will_overwrite
        .iter()
        .chain(preview.will_recreate.iter())
        .cloned()
        .collect();
    let mut failed = Vec::new();
    let mut restored = Vec::new();
    let source = format!("--source={}", row.commit_oid);
    for batch in batch_paths(&restore_paths, RESTORE_ARG_BUDGET) {
        let mut args = vec!["restore", source.as_str(), "--worktree", "--"];
        let refs: Vec<&str> = batch.iter().map(String::as_str).collect();
        args.extend(refs.iter().copied());
        let output = git(target, &args, &IndexMode::ReadOnly).await?;
        if !output.success() {
            failed.extend(batch);
            continue;
        }
        let leftover = remaining_diff(target, &row.commit_oid, &batch).await?;
        for path in batch {
            if leftover.contains(&path) {
                failed.push(path);
            } else {
                restored.push(path);
            }
        }
    }

    let (deleted, skipped_ignored) =
        delete_new_files(target, delete_new_paths).await?;

    eprintln!(
        "[git] restore checkpoint={} pre={} overwrite={} recreate={} delete={} failed={}",
        row.id,
        pre.id,
        preview.will_overwrite.len(),
        preview.will_recreate.len(),
        deleted.len(),
        failed.len()
    );

    Ok(GitRestoreResult {
        pre_restore_checkpoint: pre,
        restored,
        deleted,
        skipped_ignored,
        failed,
    })
}

async fn remaining_diff(
    target: &GitTarget,
    oid: &str,
    paths: &[String],
) -> Result<HashSet<String>, GitError> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut args = vec![
        "diff".to_string(),
        "--name-only".to_string(),
        "--no-renames".to_string(),
        "-z".to_string(),
        oid.to_string(),
        "--".to_string(),
    ];
    args.extend(paths.iter().cloned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git(target, &refs, &IndexMode::ReadOnly).await?;
    if !output.success() {
        return Ok(paths.iter().cloned().collect());
    }
    Ok(split_nul_strings(&output.stdout).into_iter().collect())
}

async fn delete_new_files(
    target: &GitTarget,
    paths: &[String],
) -> Result<(Vec<String>, Vec<String>), GitError> {
    if paths.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let ignored = {
        let mut set = ignored_paths(target, paths).await?;
        set.extend(ignored_untracked_paths(target).await?);
        set
    };
    let mut deleted = Vec::new();
    let mut skipped_ignored = Vec::new();
    for path in paths {
        if ignored.contains(path) {
            skipped_ignored.push(path.clone());
            continue;
        }
        remove_worktree_path(target, path).await?;
        deleted.push(path.clone());
    }
    Ok((deleted, skipped_ignored))
}

async fn ignored_untracked_paths(target: &GitTarget) -> Result<HashSet<String>, GitError> {
    let output = git(
        target,
        &[
            "ls-files",
            "--others",
            "-i",
            "--exclude-standard",
            "-z",
        ],
        &IndexMode::ReadOnly,
    )
    .await?;
    if !output.success() {
        return Ok(HashSet::new());
    }
    Ok(split_nul_strings(&output.stdout).into_iter().collect())
}

async fn ignored_paths(target: &GitTarget, paths: &[String]) -> Result<HashSet<String>, GitError> {
    let mut args = vec!["check-ignore", "--no-index", "-z", "--"];
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    args.extend(refs.iter().copied());
    let output = git(target, &args, &IndexMode::ReadOnly).await?;
    Ok(split_nul_strings(&output.stdout).into_iter().collect())
}

fn batch_paths(paths: &[String], budget: usize) -> Vec<Vec<String>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut size = 0;
    for path in paths {
        let cost = path.len() + 1;
        if !current.is_empty() && size + cost > budget {
            batches.push(std::mem::take(&mut current));
            size = 0;
        }
        size += cost;
        current.push(path.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

#[allow(dead_code)]
pub(crate) async fn delete_checkpoints_for_session(
    pool: &SqlitePool,
    target: &GitTarget,
    session_id: &str,
) -> Result<(), GitError> {
    let rows = sqlx::query_as::<_, GitCheckpoint>(
        "SELECT * FROM git_checkpoints WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(|error| GitError::Parse(format!("读取 session checkpoints 失败: {error}")))?;
    for row in &rows {
        let _ = git(
            target,
            &["update-ref", "-d", &row.ref_name],
            &IndexMode::ReadOnly,
        )
        .await;
    }
    sqlx::query("DELETE FROM git_checkpoints WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| GitError::Parse(format!("删除 session checkpoints 失败: {error}")))?;
    Ok(())
}

pub(crate) async fn clear_workspace_checkpoints(
    pool: &SqlitePool,
    target: &GitTarget,
    workspace_id: &str,
) -> Result<u64, GitError> {
    let refs = list_checkpoint_refs(target, None).await?;
    for ref_name in refs.keys() {
        let _ = git(
            target,
            &["update-ref", "-d", ref_name],
            &IndexMode::ReadOnly,
        )
        .await;
    }
    let result = sqlx::query("DELETE FROM git_checkpoints WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(pool)
        .await
        .map_err(|error| GitError::Parse(format!("清除 workspace checkpoints 失败: {error}")))?;
    Ok(result.rows_affected())
}

pub(crate) async fn sweep_orphan_refs(
    pool: &SqlitePool,
    target: &GitTarget,
) -> Result<Vec<String>, GitError> {
    let refs = list_checkpoint_refs(target, None).await?;
    let rows: Vec<(String,)> = sqlx::query_as("SELECT ref_name FROM git_checkpoints")
        .fetch_all(pool)
        .await
        .map_err(|error| GitError::Parse(format!("读取 checkpoint refs 失败: {error}")))?;
    let known: HashSet<String> = rows.into_iter().map(|(name,)| name).collect();
    let mut deleted = Vec::new();
    for ref_name in refs.keys() {
        if !known.contains(ref_name) {
            let output = git(
                target,
                &["update-ref", "-d", ref_name],
                &IndexMode::ReadOnly,
            )
            .await?;
            if output.success() {
                deleted.push(ref_name.clone());
            }
        }
    }
    Ok(deleted)
}
