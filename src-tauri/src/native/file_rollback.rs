//! 把受控 Write / Edit / ApplyPatch 记到消息边界上，并按预览凭据回滚这些文件。
//!
//! 不把整库检查点当成某条消息的改动。Bash、MCP、工作区外路径和没有快照的文件只说明限制。
//! 不撤销外部 API 或消息发送。回滚直接写文件，不改 Git HEAD 和用户 index。

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::history::{
    active_branch_id, create_referencing_branch, load_projection, BoundaryEdge, BranchReference,
};
use crate::native::model::types::{Message, Role};
use crate::native::tools::ssh::SshToolRuntime;

const SIDE_EFFECT_NOTE: &str = "不撤销外部 API、消息发送，以及 Bash / MCP 等无法归属的副作用";
const UNAVAILABLE: &str = "已关闭工具后自动检查点，文件回滚不可用";

static TARGET_LOCKS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn target_locked(root: &str) -> bool {
    TARGET_LOCKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(root)
}

pub(crate) fn acquire_target(root: &str) -> bool {
    TARGET_LOCKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(root.to_string())
}

pub(crate) fn release_target(root: &str) {
    TARGET_LOCKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(root);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFile {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub unsupported: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRollbackPreview {
    pub token: String,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub session_record_id: String,
    pub message_id: String,
    pub edge: String,
    pub mode: String,
    pub branch_id: String,
    pub revision: i64,
    pub execution_target: String,
    pub target_root: String,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub conflicts: Vec<String>,
    pub unsupported: Vec<String>,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedWrite {
    path: String,
    action: String,
    expected_sha: Option<String>,
    desired_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BackupFile {
    path: String,
    text: Option<String>,
    /// 这次回滚是否已经写过该路径。补偿时用它区分「我们写入的内容」和更新的第三方修改。
    #[serde(default)]
    applied: bool,
    #[serde(default)]
    applied_text: Option<String>,
}

/// 预览和应用绑定的真实执行目标。磁盘实现覆盖本地与隔离 worktree；SSH 走同一套相对路径。
pub enum LiveFiles {
    Disk {
        execution_target: String,
        root_key: String,
        root: PathBuf,
    },
    Ssh(Box<SshToolRuntime>),
}

impl LiveFiles {
    pub fn disk(execution_target: impl Into<String>, root: impl Into<String>) -> Self {
        let root_key = root.into();
        Self::Disk {
            execution_target: execution_target.into(),
            root: PathBuf::from(&root_key),
            root_key,
        }
    }

    pub fn ssh(runtime: SshToolRuntime) -> Self {
        Self::Ssh(Box::new(runtime))
    }

    pub fn execution_target(&self) -> &str {
        match self {
            Self::Disk {
                execution_target, ..
            } => execution_target,
            Self::Ssh(_) => "ssh",
        }
    }

    pub fn root_key(&self) -> &str {
        match self {
            Self::Disk { root_key, .. } => root_key,
            Self::Ssh(ssh) => &ssh.root,
        }
    }

    pub async fn read_relative(&self, relative: &str) -> Result<Option<String>, String> {
        match self {
            Self::Disk { root, .. } => read_optional(root, relative),
            Self::Ssh(ssh) => {
                reject_relative(relative)?;
                if !ssh.exists(relative).await? {
                    return Ok(None);
                }
                ssh.read(relative).await.map(Some)
            }
        }
    }

    pub async fn write_relative(&self, relative: &str, text: &str) -> Result<(), String> {
        match self {
            Self::Disk { root, .. } => {
                let path = safe_join(root, relative)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                std::fs::write(path, text).map_err(|error| error.to_string())
            }
            Self::Ssh(ssh) => {
                reject_relative(relative)?;
                ssh.write(relative, text).await.map(|_| ())
            }
        }
    }

    pub async fn remove_relative(&self, relative: &str) -> Result<(), String> {
        match self {
            Self::Disk { root, .. } => {
                let path = safe_join(root, relative)?;
                if path.exists() {
                    std::fs::remove_file(path).map_err(|error| error.to_string())?;
                }
                Ok(())
            }
            Self::Ssh(ssh) => {
                reject_relative(relative)?;
                ssh.delete(relative).await.map(|_| ())
            }
        }
    }
}

pub struct ApplyOptions<'a> {
    pub request_id: &'a str,
    pub interrupt_after: Option<usize>,
    pub before_compensate: Option<fn(&Path)>,
}

impl<'a> ApplyOptions<'a> {
    pub fn new(request_id: &'a str) -> Self {
        Self {
            request_id,
            interrupt_after: None,
            before_compensate: None,
        }
    }
}

pub struct RollbackCommit<'a> {
    pub session_record_id: &'a str,
    pub message_id: &'a str,
    pub edge: BoundaryEdge,
    pub mode: &'a str,
    pub expected_revision: i64,
    pub token: &'a str,
    pub request_id: &'a str,
    pub files_enabled: bool,
    pub workspace_id: Option<&'a str>,
    pub model: &'a str,
}

#[derive(Debug)]
pub struct RollbackCommitReceipt {
    pub session_record_id: String,
    pub branch_id: Option<String>,
    pub changed_conversation: bool,
}

struct TargetGuard(String);

impl Drop for TargetGuard {
    fn drop(&mut self) {
        release_target(&self.0);
    }
}

pub fn content_sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub async fn record_files(
    pool: &SqlitePool,
    session_record_id: &str,
    call_id: &str,
    tool_name: &str,
    execution_target: &str,
    target_root: &str,
    files: &[CapturedFile],
) -> Result<(), String> {
    if target_locked(target_root) {
        return Err("执行目标正在回滚文件，请稍后再写入".to_string());
    }
    let now = now_sqlite();
    for file in files {
        let (kind, before_sha, after_sha) = if let Some(reason) = &file.unsupported {
            ("unsupported", None, Some(reason.clone()))
        } else {
            match (&file.before, &file.after) {
                (None, None) => continue,
                (None, Some(after)) => ("added", None, Some(content_sha(after))),
                (Some(before), None) => ("deleted", Some(content_sha(before)), None),
                (Some(before), Some(after)) if before == after => continue,
                (Some(before), Some(after)) => (
                    "modified",
                    Some(content_sha(before)),
                    Some(content_sha(after)),
                ),
            }
        };
        sqlx::query(
            r#"
            INSERT INTO native_file_revisions (
                id, session_record_id, call_id, tool_name, path, execution_target, target_root,
                change_kind, before_sha, after_sha, before_text, after_text, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
        )
        .bind(new_id())
        .bind(session_record_id)
        .bind(call_id)
        .bind(tool_name)
        .bind(&file.path)
        .bind(execution_target)
        .bind(target_root)
        .bind(kind)
        .bind(&before_sha)
        .bind(&after_sha)
        .bind(file.before.as_deref())
        .bind(file.after.as_deref())
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|error| format!("保存文件回滚记录失败: {error}"))?;
    }
    Ok(())
}

pub async fn preview(
    pool: &SqlitePool,
    session_record_id: &str,
    message_id: &str,
    edge: BoundaryEdge,
    mode: &str,
    files_enabled: bool,
    files: &LiveFiles,
) -> Result<FileRollbackPreview, String> {
    let mode = normalize_mode(mode)?;
    let messages = load_projection(pool, session_record_id)
        .await?
        .unwrap_or_default();
    let (branch_id, revision) = active_revision(pool, session_record_id).await?;
    let mut preview = FileRollbackPreview {
        token: String::new(),
        available: files_enabled,
        unavailable_reason: (!files_enabled).then(|| UNAVAILABLE.to_string()),
        session_record_id: session_record_id.to_string(),
        message_id: message_id.to_string(),
        edge: edge_name(edge).to_string(),
        mode,
        branch_id,
        revision,
        execution_target: files.execution_target().to_string(),
        target_root: files.root_key().to_string(),
        added: Vec::new(),
        modified: Vec::new(),
        deleted: Vec::new(),
        conflicts: Vec::new(),
        unsupported: Vec::new(),
        side_effects: vec![SIDE_EFFECT_NOTE.to_string()],
    };
    let Some(after) = messages_after(&messages, message_id, edge) else {
        preview.available = false;
        preview.unavailable_reason = Some("边界消息不存在".to_string());
        preview.token = token_for(&preview);
        return Ok(preview);
    };
    for message in after {
        if message.role == Role::Tool && is_unscoped_tool(&message.name) {
            preview
                .unsupported
                .push(format!("不自动回滚 {}", message.name));
        }
        for call in &message.tool_calls {
            if is_unscoped_tool(&call.name) {
                preview
                    .unsupported
                    .push(format!("不自动回滚 {}", call.name));
            }
        }
    }
    if !files_enabled {
        preview.token = token_for(&preview);
        return Ok(preview);
    }
    let call_ids = call_ids_after(after);
    let revisions = load_revisions(pool, session_record_id).await?;
    let mut by_path: HashMap<String, Vec<RevisionRow>> = HashMap::new();
    for revision in revisions {
        if !call_ids.contains(&revision.call_id) {
            continue;
        }
        if revision.change_kind == "unsupported" {
            preview.unsupported.push(format!(
                "{}：{}",
                revision.path,
                revision.after_sha.unwrap_or_default()
            ));
            continue;
        }
        if revision.execution_target != files.execution_target()
            || revision.target_root != files.root_key()
        {
            preview.unsupported.push(format!(
                "{} 属于 {} {}，不能改到当前执行目标",
                revision.path, revision.execution_target, revision.target_root
            ));
            continue;
        }
        by_path
            .entry(revision.path.clone())
            .or_default()
            .push(revision);
    }
    for (path, rows) in by_path {
        let Some(first) = rows.first() else { continue };
        let Some(last) = rows.last() else { continue };
        let current = files.read_relative(&path).await.ok().flatten();
        let current_sha = current.as_deref().map(content_sha);
        if current_sha != last.after_sha {
            preview.conflicts.push(path);
            continue;
        }
        match first.change_kind.as_str() {
            "added" => preview.added.push(path),
            "deleted" => {
                if first.before_text.is_none() {
                    preview.unsupported.push(format!("{path} 没有删除前快照"));
                } else {
                    preview.deleted.push(path);
                }
            }
            "modified" => {
                if first.before_text.is_none() {
                    preview.unsupported.push(format!("{path} 没有修改前快照"));
                } else {
                    preview.modified.push(path);
                }
            }
            _ => preview.unsupported.push(path),
        }
    }
    preview.added.sort();
    preview.modified.sort();
    preview.deleted.sort();
    preview.conflicts.sort();
    preview.unsupported.sort();
    preview.unsupported.dedup();
    preview.token = token_for(&preview);
    Ok(preview)
}

pub async fn apply_preview(
    pool: &SqlitePool,
    preview: &FileRollbackPreview,
    files: &LiveFiles,
    options: ApplyOptions<'_>,
) -> Result<(), String> {
    if options.request_id.trim().is_empty() {
        return Err("请求标识不能为空".to_string());
    }
    if let Some(stored) = stored_rollback(pool, options.request_id).await? {
        return match stored.status.as_str() {
            "completed" | "failed" => Ok(()),
            "needs_recovery" => Err("文件回滚仍待恢复，不能重复应用".to_string()),
            _ => Err("文件回滚尚未结束".to_string()),
        };
    }
    if preview.mode == "conversation" {
        return Ok(());
    }
    if files.execution_target() != preview.execution_target
        || files.root_key() != preview.target_root
    {
        return Err("执行目标已变化，未写入任何路径".to_string());
    }
    if !preview.available {
        return Err(preview
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| UNAVAILABLE.to_string()));
    }
    if !preview.conflicts.is_empty() {
        return Err(format!(
            "文件已变化，未写入任何路径：{}",
            preview.conflicts.join("、")
        ));
    }
    let ops = planned_writes(pool, preview).await?;
    if !acquire_target(&preview.target_root) {
        return Err("执行目标正在回滚文件，请稍后再试".to_string());
    }
    let _guard = TargetGuard(preview.target_root.clone());
    let mut backup = backup_files(files, &ops).await?;
    let rollback_id = new_id();
    let now = now_sqlite();
    let backup_json = serde_json::to_string(&backup).unwrap_or_else(|_| "[]".to_string());
    sqlx::query(
        r#"
        INSERT INTO native_file_rollbacks (
            id, session_record_id, request_id, message_id, edge, mode, target_root,
            execution_target, preview_token, status, backup_json, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'applying', $10, $11, $11)
        "#,
    )
    .bind(&rollback_id)
    .bind(&preview.session_record_id)
    .bind(options.request_id)
    .bind(&preview.message_id)
    .bind(&preview.edge)
    .bind(&preview.mode)
    .bind(&preview.target_root)
    .bind(&preview.execution_target)
    .bind(&preview.token)
    .bind(&backup_json)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("保存回滚操作失败: {error}"))?;
    let outcome = write_ops(
        pool,
        &rollback_id,
        files,
        &ops,
        &mut backup,
        options.interrupt_after,
    )
    .await;
    match outcome {
        Ok(()) => {
            mark_rollback(pool, &rollback_id, "completed", None).await?;
            Ok(())
        }
        Err(error) => {
            if let Some(hook) = options.before_compensate {
                if let LiveFiles::Disk { root, .. } = files {
                    hook(root);
                }
            }
            compensate(pool, &rollback_id, files, &backup, &error).await
        }
    }
}

pub async fn commit_rollback(
    pool: &SqlitePool,
    commit: RollbackCommit<'_>,
    files: &LiveFiles,
) -> Result<RollbackCommitReceipt, String> {
    let mode = normalize_mode(commit.mode)?;
    if commit.request_id.trim().is_empty() {
        return Err("请求标识不能为空".to_string());
    }
    let stored = stored_rollback(pool, commit.request_id).await?;
    if let Some(stored) = &stored {
        match stored.status.as_str() {
            "completed" => {}
            "failed" => {
                return Err(stored
                    .error
                    .clone()
                    .unwrap_or_else(|| "文件回滚失败并已恢复备份".to_string()));
            }
            "needs_recovery" => return Err("文件回滚仍待恢复，不能重复应用".to_string()),
            _ => return Err("文件回滚尚未结束".to_string()),
        }
    }
    let history_done = history_request_done(pool, commit.request_id).await?;
    let files_done = stored
        .as_ref()
        .is_some_and(|item| item.status == "completed");
    let need_files = mode != "conversation" && !files_done;
    let need_conversation = mode != "files" && !history_done;
    if need_files || (need_conversation && !files_done) {
        let fresh = preview(
            pool,
            commit.session_record_id,
            commit.message_id,
            commit.edge,
            &mode,
            commit.files_enabled,
            files,
        )
        .await?;
        if fresh.revision != commit.expected_revision {
            return Err("分支修订号不匹配".to_string());
        }
        if fresh.token != commit.token {
            return Err("回滚预览已过期，未写入任何路径".to_string());
        }
        if fresh.execution_target != files.execution_target()
            || fresh.target_root != files.root_key()
        {
            return Err("执行目标已变化，未写入任何路径".to_string());
        }
        if need_files {
            apply_preview(pool, &fresh, files, ApplyOptions::new(commit.request_id)).await?;
        }
    } else if need_conversation && files_done {
        let stored = stored.expect("completed rollback");
        if stored.token != commit.token {
            return Err("回滚预览已过期，未写入任何路径".to_string());
        }
        let (_, revision) = active_revision(pool, commit.session_record_id).await?;
        if revision != commit.expected_revision {
            return Err("分支修订号不匹配".to_string());
        }
    }
    let mut branch_id = None;
    let mut changed_conversation = false;
    if mode != "files" {
        let source = active_branch_id(pool, commit.session_record_id)
            .await?
            .ok_or_else(|| "该会话没有活动历史分支".to_string())?;
        let receipt = create_referencing_branch(
            pool,
            BranchReference {
                new_session_id: commit.session_record_id,
                source_branch_id: &source,
                boundary_message_id: Some(commit.message_id),
                profile_id: None,
                workspace_id: commit.workspace_id,
                model: commit.model,
                turns: 0,
                edge: commit.edge,
                expected_revision: Some(commit.expected_revision),
                request_id: Some(commit.request_id),
                seal_source: true,
            },
        )
        .await?;
        branch_id = Some(receipt.branch_id);
        changed_conversation = receipt.changed;
    }
    Ok(RollbackCommitReceipt {
        session_record_id: commit.session_record_id.to_string(),
        branch_id,
        changed_conversation,
    })
}

pub async fn has_unsettled(pool: &SqlitePool, session_record_id: &str) -> Result<bool, String> {
    let found = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1 FROM native_file_rollbacks
        WHERE session_record_id = $1 AND status IN ('applying', 'compensating', 'needs_recovery')
        LIMIT 1
        "#,
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取未完成回滚失败: {error}"))?;
    Ok(found.is_some())
}

pub async fn block_until_rollback_settled(
    pool: &SqlitePool,
    session_record_id: &str,
    files: &LiveFiles,
) -> Result<(), String> {
    let rows = sqlx::query(
        r#"
        SELECT id, target_root, execution_target, backup_json, status
        FROM native_file_rollbacks
        WHERE session_record_id = $1
          AND status IN ('applying', 'compensating', 'needs_recovery')
        ORDER BY created_at ASC
        "#,
    )
    .bind(session_record_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取未完成回滚失败: {error}"))?;
    for row in rows {
        let status: String = row.get("status");
        if status == "needs_recovery" {
            return Err("有未完成的文件回滚，备份已保留，请先处理后再继续".to_string());
        }
        let target_root: String = row.get("target_root");
        let execution_target: String = row.get("execution_target");
        if execution_target != files.execution_target() || target_root != files.root_key() {
            return Err(
                "有未完成的文件回滚，但当前执行目标与备份不一致，请先处理后再继续".to_string(),
            );
        }
        let id: String = row.get("id");
        let backup: Vec<BackupFile> =
            serde_json::from_str(row.get("backup_json")).unwrap_or_default();
        if let Err(error) = compensate(pool, &id, files, &backup, "进程中断后的回滚").await
        {
            if !error.contains("已恢复备份") {
                return Err(error);
            }
        }
    }
    Ok(())
}

async fn compensate(
    pool: &SqlitePool,
    rollback_id: &str,
    files: &LiveFiles,
    backup: &[BackupFile],
    error: &str,
) -> Result<(), String> {
    mark_rollback(pool, rollback_id, "compensating", Some(error)).await?;
    for file in backup {
        let current = files.read_relative(&file.path).await?;
        let matches_backup = current.as_deref() == file.text.as_deref();
        let matches_applied = file.applied && current.as_deref() == file.applied_text.as_deref();
        let newer = if file.applied {
            !matches_applied && !matches_backup
        } else {
            !matches_backup
        };
        if newer {
            mark_rollback(
                pool,
                rollback_id,
                "needs_recovery",
                Some("补偿时发现新的文件修改，已保留备份"),
            )
            .await?;
            return Err("补偿时发现新的文件修改，已保留备份，回滚未成功".to_string());
        }
        restore_backup(files, file).await?;
    }
    mark_rollback(pool, rollback_id, "failed", Some(error)).await?;
    Err(format!("文件回滚失败并已恢复备份：{error}"))
}

async fn restore_backup(files: &LiveFiles, file: &BackupFile) -> Result<(), String> {
    match &file.text {
        Some(text) => files.write_relative(&file.path, text).await,
        None => files.remove_relative(&file.path).await,
    }
}

async fn write_ops(
    pool: &SqlitePool,
    rollback_id: &str,
    files: &LiveFiles,
    ops: &[PlannedWrite],
    backup: &mut [BackupFile],
    interrupt_after: Option<usize>,
) -> Result<(), String> {
    for (index, op) in ops.iter().enumerate() {
        if interrupt_after.is_some_and(|limit| index >= limit) {
            return Err("回滚写入中断".to_string());
        }
        let current = files.read_relative(&op.path).await?;
        let current_sha = current.as_deref().map(content_sha);
        if current_sha != op.expected_sha {
            return Err(format!("{} 在写入前已变化", op.path));
        }
        if let Some(slot) = backup.iter_mut().find(|item| item.path == op.path) {
            slot.applied = true;
            slot.applied_text = if op.action == "delete" {
                None
            } else {
                op.desired_text.clone()
            };
        }
        persist_backup(pool, rollback_id, backup).await?;
        if op.action == "delete" {
            files.remove_relative(&op.path).await?;
        } else {
            let text = op.desired_text.clone().unwrap_or_default();
            files.write_relative(&op.path, &text).await?;
            let read_back = files.read_relative(&op.path).await?.unwrap_or_default();
            if content_sha(&read_back) != content_sha(&text) {
                return Err(format!("{} 写入后校验失败", op.path));
            }
        }
    }
    Ok(())
}

async fn backup_files(files: &LiveFiles, ops: &[PlannedWrite]) -> Result<Vec<BackupFile>, String> {
    let mut backup = Vec::with_capacity(ops.len());
    for op in ops {
        backup.push(BackupFile {
            path: op.path.clone(),
            text: files.read_relative(&op.path).await?,
            applied: false,
            applied_text: None,
        });
    }
    Ok(backup)
}

async fn persist_backup(pool: &SqlitePool, id: &str, backup: &[BackupFile]) -> Result<(), String> {
    let json =
        serde_json::to_string(backup).map_err(|error| format!("序列化回滚备份失败: {error}"))?;
    sqlx::query("UPDATE native_file_rollbacks SET backup_json = $1, updated_at = $2 WHERE id = $3")
        .bind(json)
        .bind(now_sqlite())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("保存回滚备份失败: {error}"))?;
    Ok(())
}

async fn planned_writes(
    pool: &SqlitePool,
    preview: &FileRollbackPreview,
) -> Result<Vec<PlannedWrite>, String> {
    let messages = load_projection(pool, &preview.session_record_id)
        .await?
        .unwrap_or_default();
    let edge = BoundaryEdge::parse(Some(&preview.edge))?;
    let after = messages_after(&messages, &preview.message_id, edge).unwrap_or_default();
    let call_ids = call_ids_after(after);
    let revisions = load_revisions(pool, &preview.session_record_id).await?;
    let mut by_path: HashMap<String, Vec<RevisionRow>> = HashMap::new();
    for revision in revisions {
        if call_ids.contains(&revision.call_id)
            && revision.target_root == preview.target_root
            && revision.execution_target == preview.execution_target
            && revision.change_kind != "unsupported"
        {
            by_path
                .entry(revision.path.clone())
                .or_default()
                .push(revision);
        }
    }
    let mut ops = Vec::new();
    for (path, rows) in by_path {
        let Some(first) = rows.first() else { continue };
        let Some(last) = rows.last() else { continue };
        if preview.conflicts.iter().any(|item| item == &path) {
            continue;
        }
        match first.change_kind.as_str() {
            "added" => ops.push(PlannedWrite {
                path,
                action: "delete".to_string(),
                expected_sha: last.after_sha.clone(),
                desired_text: None,
            }),
            "deleted" | "modified" => ops.push(PlannedWrite {
                path,
                action: "restore".to_string(),
                expected_sha: last.after_sha.clone(),
                desired_text: first.before_text.clone(),
            }),
            _ => {}
        }
    }
    ops.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(ops)
}

fn messages_after<'a>(
    messages: &'a [Message],
    message_id: &str,
    edge: BoundaryEdge,
) -> Option<&'a [Message]> {
    let position = messages
        .iter()
        .position(|message| message.history_id == message_id)?;
    let end = match edge {
        BoundaryEdge::After => position + 1,
        BoundaryEdge::Before => position,
    };
    Some(&messages[end..])
}

fn call_ids_after(messages: &[Message]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for message in messages {
        if message.role == Role::Tool && !message.tool_call_id.is_empty() {
            ids.insert(message.tool_call_id.clone());
        }
        for call in &message.tool_calls {
            if !call.id.is_empty() {
                ids.insert(call.id.clone());
            }
        }
    }
    ids
}

fn is_unscoped_tool(name: &str) -> bool {
    matches!(name, "Bash") || name.starts_with("mcp") || name.starts_with("MCP")
}

struct RevisionRow {
    call_id: String,
    path: String,
    execution_target: String,
    target_root: String,
    change_kind: String,
    before_sha: Option<String>,
    after_sha: Option<String>,
    before_text: Option<String>,
}

async fn load_revisions(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Vec<RevisionRow>, String> {
    let rows = sqlx::query(
        r#"
        SELECT call_id, path, execution_target, target_root, change_kind,
               before_sha, after_sha, before_text
        FROM native_file_revisions
        WHERE session_record_id = $1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(session_record_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取文件回滚记录失败: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| RevisionRow {
            call_id: row.get("call_id"),
            path: row.get("path"),
            execution_target: row.get("execution_target"),
            target_root: row.get("target_root"),
            change_kind: row.get("change_kind"),
            before_sha: row.get("before_sha"),
            after_sha: row.get("after_sha"),
            before_text: row.get("before_text"),
        })
        .collect())
}

async fn active_revision(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<(String, i64), String> {
    let row = sqlx::query(
        r#"
        SELECT id, revision FROM native_history_branches
        WHERE session_record_id = $1 AND active = 1 AND deleted_at IS NULL
        "#,
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取历史分支失败: {error}"))?
    .ok_or_else(|| "该会话没有活动历史分支".to_string())?;
    Ok((row.get("id"), row.get("revision")))
}

struct StoredRollback {
    status: String,
    token: String,
    error: Option<String>,
}

async fn stored_rollback(
    pool: &SqlitePool,
    request_id: &str,
) -> Result<Option<StoredRollback>, String> {
    let row = sqlx::query(
        "SELECT status, preview_token, error FROM native_file_rollbacks WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取回滚请求失败: {error}"))?;
    Ok(row.map(|row| StoredRollback {
        status: row.get("status"),
        token: row.get("preview_token"),
        error: row.get("error"),
    }))
}

async fn history_request_done(pool: &SqlitePool, request_id: &str) -> Result<bool, String> {
    let found = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM native_history_requests WHERE request_id = $1 LIMIT 1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取边界请求失败: {error}"))?;
    Ok(found.is_some())
}

async fn mark_rollback(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE native_file_rollbacks SET status = $1, error = $2, updated_at = $3 WHERE id = $4",
    )
    .bind(status)
    .bind(error)
    .bind(now_sqlite())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新回滚状态失败: {error}"))?;
    Ok(())
}

fn normalize_mode(mode: &str) -> Result<String, String> {
    match mode.trim() {
        "conversation" | "files" | "both" => Ok(mode.trim().to_string()),
        _ => Err("回滚方式只能是 conversation、files 或 both".to_string()),
    }
}

fn edge_name(edge: BoundaryEdge) -> &'static str {
    match edge {
        BoundaryEdge::Before => "before",
        BoundaryEdge::After => "after",
    }
}

fn token_for(preview: &FileRollbackPreview) -> String {
    let payload = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}",
        preview.session_record_id,
        preview.message_id,
        preview.edge,
        preview.mode,
        preview.revision,
        preview.execution_target,
        preview.target_root,
        preview.available,
        preview.added,
        preview.modified,
        preview.deleted,
        preview.conflicts,
        preview.unsupported
    );
    content_sha(&payload)
}

pub fn read_optional(root: &Path, relative: &str) -> Result<Option<String>, String> {
    let path = safe_join(root, relative)?;
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("读取 {relative} 失败: {error}"))
}

fn reject_relative(relative: &str) -> Result<(), String> {
    safe_join(Path::new("."), relative).map(|_| ())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() || relative.contains('\0') {
        return Err(format!("拒绝绝对路径 {relative}"));
    }
    let mut out = root.to_path_buf();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(format!("路径越界 {relative}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::history::{active_branch_id, commit_model_context, HistoryWrite};
    use crate::native::model::types::{Message, ToolCall};

    fn write<'a>(session: &'a str, messages: &'a mut [Message]) -> HistoryWrite<'a> {
        HistoryWrite {
            session_record_id: session,
            profile_id: None,
            workspace_id: None,
            model: "m",
            turns: 1,
            messages,
            turn_id: None,
            attempt_id: None,
            expected_revision: None,
            request_id: None,
            links: &[],
            legacy_baseline: false,
        }
    }

    async fn seed(pool: &SqlitePool, session: &str) -> (String, Vec<Message>) {
        sqlx::query("INSERT INTO agent_sessions (id) VALUES ($1)")
            .bind(session)
            .execute(pool)
            .await
            .unwrap();
        let mut messages = vec![Message::user("one"), Message::user("two")];
        commit_model_context(pool, write(session, &mut messages))
            .await
            .unwrap();
        let root = std::env::temp_dir().join(format!("nox-rollback-{session}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (root.to_string_lossy().into_owned(), messages)
    }

    async fn link_calls(
        pool: &SqlitePool,
        session: &str,
        messages: &mut Vec<Message>,
        ids: &[(&str, &str)],
    ) {
        let mut assistant = Message::assistant_text("tools");
        assistant.tool_calls = ids
            .iter()
            .map(|(id, name)| ToolCall {
                id: (*id).to_string(),
                name: (*name).to_string(),
                arguments: "{}".to_string(),
            })
            .collect();
        messages.push(assistant);
        for (id, name) in ids {
            let mut result = Message::tool_result(*id, "ok");
            result.name = (*name).to_string();
            messages.push(result);
        }
        commit_model_context(pool, write(session, messages))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn local_ssh_and_worktree_cover_add_edit_delete_without_touching_other_roots() {
        let pool = setup_migrated_pool().await;
        let (local, mut messages) = seed(&pool, "local").await;
        let ssh = std::env::temp_dir().join("nox-rollback-ssh");
        let work = std::env::temp_dir().join("nox-rollback-work");
        let _ = std::fs::remove_dir_all(&ssh);
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        record_files(
            &pool,
            "local",
            "call-add",
            "Write",
            "local",
            &local,
            &[CapturedFile {
                path: "added.txt".into(),
                before: None,
                after: Some("new".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&local).join("added.txt"), "new").unwrap();
        record_files(
            &pool,
            "local",
            "call-edit",
            "Edit",
            "ssh",
            ssh.to_str().unwrap(),
            &[CapturedFile {
                path: "note.txt".into(),
                before: Some("old".into()),
                after: Some("edited".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        std::fs::write(ssh.join("note.txt"), "edited").unwrap();
        record_files(
            &pool,
            "local",
            "call-del",
            "ApplyPatch",
            "local",
            work.to_str().unwrap(),
            &[CapturedFile {
                path: "gone.txt".into(),
                before: Some("keep".into()),
                after: None,
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        link_calls(
            &pool,
            "local",
            &mut messages,
            &[
                ("call-add", "Write"),
                ("call-edit", "Edit"),
                ("call-del", "ApplyPatch"),
                ("call-bash", "Bash"),
            ],
        )
        .await;
        let boundary = messages[0].history_id.clone();
        let local_files = LiveFiles::disk("local", &local);
        let preview = preview(
            &pool,
            "local",
            &boundary,
            BoundaryEdge::After,
            "files",
            true,
            &local_files,
        )
        .await
        .unwrap();
        assert_eq!(preview.added, vec!["added.txt".to_string()]);
        assert!(preview
            .unsupported
            .iter()
            .any(|item| item.contains("note.txt") && item.contains("ssh")));
        assert!(preview
            .unsupported
            .iter()
            .any(|item| item.contains("gone.txt")));
        assert!(preview
            .unsupported
            .iter()
            .any(|item| item.contains("不自动回滚 Bash")));
        apply_preview(
            &pool,
            &preview,
            &local_files,
            ApplyOptions::new("req-local"),
        )
        .await
        .unwrap();
        assert!(!Path::new(&local).join("added.txt").exists());
        assert_eq!(
            std::fs::read_to_string(ssh.join("note.txt")).unwrap(),
            "edited"
        );
        assert!(!work.join("gone.txt").exists());
        let ssh_files = LiveFiles::disk("ssh", ssh.to_string_lossy());
        let ssh_preview = super::preview(
            &pool,
            "local",
            &boundary,
            BoundaryEdge::After,
            "files",
            true,
            &ssh_files,
        )
        .await
        .unwrap();
        assert_eq!(ssh_preview.modified, vec!["note.txt".to_string()]);
        apply_preview(
            &pool,
            &ssh_preview,
            &ssh_files,
            ApplyOptions::new("req-ssh"),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(ssh.join("note.txt")).unwrap(),
            "old"
        );
        assert!(!Path::new(&local).join("added.txt").exists());
        let work_files = LiveFiles::disk("local", work.to_string_lossy());
        let work_preview = super::preview(
            &pool,
            "local",
            &boundary,
            BoundaryEdge::After,
            "files",
            true,
            &work_files,
        )
        .await
        .unwrap();
        assert_eq!(work_preview.deleted, vec!["gone.txt".to_string()]);
        apply_preview(
            &pool,
            &work_preview,
            &work_files,
            ApplyOptions::new("req-work"),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(work.join("gone.txt")).unwrap(),
            "keep"
        );
        assert_eq!(
            std::fs::read_to_string(ssh.join("note.txt")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&ssh);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn conflict_writes_nothing_and_restart_keeps_backup() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "conflict").await;
        record_files(
            &pool,
            "conflict",
            "c1",
            "Write",
            "local",
            &root,
            &[
                CapturedFile {
                    path: "a.txt".into(),
                    before: Some("a".into()),
                    after: Some("A".into()),
                    unsupported: None,
                },
                CapturedFile {
                    path: "b.txt".into(),
                    before: Some("b".into()),
                    after: Some("B".into()),
                    unsupported: None,
                },
            ],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "A").unwrap();
        std::fs::write(Path::new(&root).join("b.txt"), "changed-by-user").unwrap();
        link_calls(&pool, "conflict", &mut messages, &[("c1", "Write")]).await;
        let files = LiveFiles::disk("local", &root);
        let preview = preview(
            &pool,
            "conflict",
            &messages[0].history_id,
            BoundaryEdge::After,
            "files",
            true,
            &files,
        )
        .await
        .unwrap();
        assert_eq!(preview.conflicts, vec!["b.txt".to_string()]);
        let branch = active_branch_id(&pool, "conflict").await.unwrap().unwrap();
        let error = commit_rollback(
            &pool,
            commit_of(
                "conflict",
                &messages[0].history_id,
                &preview,
                "req-conflict",
            ),
            &files,
        )
        .await
        .unwrap_err();
        assert!(error.contains("未写入"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "A"
        );
        assert_eq!(
            active_branch_id(&pool, "conflict").await.unwrap().unwrap(),
            branch
        );
        let disabled = super::preview(
            &pool,
            "conflict",
            &messages[0].history_id,
            BoundaryEdge::After,
            "files",
            false,
            &files,
        )
        .await
        .unwrap();
        assert!(!disabled.available);
        assert!(disabled.unavailable_reason.unwrap().contains("自动检查点"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn interrupted_apply_restores_backup_unless_a_new_edit_appears() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "interrupt").await;
        record_files(
            &pool,
            "interrupt",
            "c1",
            "Write",
            "local",
            &root,
            &[CapturedFile {
                path: "a.txt".into(),
                before: Some("before".into()),
                after: Some("after".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        link_calls(&pool, "interrupt", &mut messages, &[("c1", "Write")]).await;
        let files = LiveFiles::disk("local", &root);
        let preview = preview(
            &pool,
            "interrupt",
            &messages[0].history_id,
            BoundaryEdge::Before,
            "files",
            true,
            &files,
        )
        .await
        .unwrap();
        let error = apply_preview(
            &pool,
            &preview,
            &files,
            ApplyOptions {
                request_id: "req-int",
                interrupt_after: Some(0),
                before_compensate: None,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("已恢复备份"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "after"
        );
        block_until_rollback_settled(&pool, "interrupt", &files)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn git_head_and_index_stay_unchanged() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "githead").await;
        let target = crate::git::runner::GitTarget::Local(PathBuf::from(&root));
        crate::git::runner::fixture_git(&target, &["init"])
            .await
            .unwrap();
        crate::git::runner::fixture_git(&target, &["config", "user.name", "tester"])
            .await
            .unwrap();
        crate::git::runner::fixture_git(&target, &["config", "user.email", "t@local"])
            .await
            .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        crate::git::runner::fixture_git(&target, &["add", "a.txt"])
            .await
            .unwrap();
        crate::git::runner::fixture_git(&target, &["commit", "-m", "init"])
            .await
            .unwrap();
        let head = crate::git::runner::fixture_git(&target, &["rev-parse", "HEAD"])
            .await
            .unwrap()
            .stdout_lossy();
        record_files(
            &pool,
            "githead",
            "c1",
            "Write",
            "local",
            &root,
            &[CapturedFile {
                path: "a.txt".into(),
                before: Some("before".into()),
                after: Some("after".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        link_calls(&pool, "githead", &mut messages, &[("c1", "Write")]).await;
        let files = LiveFiles::disk("local", &root);
        let preview = preview(
            &pool,
            "githead",
            &messages[0].history_id,
            BoundaryEdge::Before,
            "files",
            true,
            &files,
        )
        .await
        .unwrap();
        apply_preview(&pool, &preview, &files, ApplyOptions::new("req-git"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "before"
        );
        let head_after = crate::git::runner::fixture_git(&target, &["rev-parse", "HEAD"])
            .await
            .unwrap()
            .stdout_lossy();
        assert_eq!(head, head_after);
        let staged = crate::git::runner::fixture_git(&target, &["diff", "--cached", "--name-only"])
            .await
            .unwrap()
            .stdout_lossy();
        assert!(staged.trim().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn commit_of<'a>(
        session: &'a str,
        message_id: &'a str,
        preview: &'a FileRollbackPreview,
        request_id: &'a str,
    ) -> RollbackCommit<'a> {
        RollbackCommit {
            session_record_id: session,
            message_id,
            edge: BoundaryEdge::parse(Some(&preview.edge)).expect("edge"),
            mode: &preview.mode,
            expected_revision: preview.revision,
            token: &preview.token,
            request_id,
            files_enabled: true,
            workspace_id: None,
            model: "m",
        }
    }

    async fn active_branch(pool: &SqlitePool, session: &str) -> String {
        active_branch_id(pool, session).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn both_rewinds_only_after_files_and_a_stale_token_writes_nothing() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "both").await;
        record_files(
            &pool,
            "both",
            "c1",
            "Write",
            "local",
            &root,
            &[CapturedFile {
                path: "a.txt".into(),
                before: Some("before".into()),
                after: Some("after".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        link_calls(&pool, "both", &mut messages, &[("c1", "Write")]).await;
        let files = LiveFiles::disk("local", &root);
        let message_id = messages[0].history_id.clone();
        let preview = preview(
            &pool,
            "both",
            &message_id,
            BoundaryEdge::After,
            "both",
            true,
            &files,
        )
        .await
        .unwrap();
        let branch = active_branch(&pool, "both").await;
        let mut stale = commit_of("both", &message_id, &preview, "req-stale");
        stale.token = "nope";
        let error = commit_rollback(&pool, stale, &files).await.unwrap_err();
        assert!(error.contains("未写入"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "after"
        );
        assert_eq!(active_branch(&pool, "both").await, branch);
        let mut wrong = commit_of("both", &message_id, &preview, "req-rev");
        wrong.expected_revision += 1;
        let error = commit_rollback(&pool, wrong, &files).await.unwrap_err();
        assert!(error.contains("分支修订号不匹配"));
        assert_eq!(active_branch(&pool, "both").await, branch);
        let receipt = commit_rollback(
            &pool,
            commit_of("both", &message_id, &preview, "req-both"),
            &files,
        )
        .await
        .unwrap();
        assert!(receipt.changed_conversation);
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "before"
        );
        let sealed: i64 =
            sqlx::query_scalar("SELECT sealed FROM native_history_branches WHERE id = $1")
                .bind(&branch)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sealed, 1);
        assert_ne!(active_branch(&pool, "both").await, branch);
        let again = commit_rollback(
            &pool,
            commit_of("both", &message_id, &preview, "req-both"),
            &files,
        )
        .await
        .unwrap();
        assert!(!again.changed_conversation);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn files_leave_the_branch_and_conversation_leaves_the_file() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "modes").await;
        record_files(
            &pool,
            "modes",
            "c1",
            "Edit",
            "local",
            &root,
            &[CapturedFile {
                path: "a.txt".into(),
                before: Some("before".into()),
                after: Some("after".into()),
                unsupported: None,
            }],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        link_calls(&pool, "modes", &mut messages, &[("c1", "Edit")]).await;
        let files = LiveFiles::disk("local", &root);
        let message_id = messages[0].history_id.clone();
        let files_preview = preview(
            &pool,
            "modes",
            &message_id,
            BoundaryEdge::After,
            "files",
            true,
            &files,
        )
        .await
        .unwrap();
        let branch = active_branch(&pool, "modes").await;
        commit_rollback(
            &pool,
            commit_of("modes", &message_id, &files_preview, "req-files"),
            &files,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "before"
        );
        assert_eq!(active_branch(&pool, "modes").await, branch);
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        let talk = preview(
            &pool,
            "modes",
            &message_id,
            BoundaryEdge::After,
            "conversation",
            true,
            &files,
        )
        .await
        .unwrap();
        let receipt = commit_rollback(
            &pool,
            commit_of("modes", &message_id, &talk, "req-talk"),
            &files,
        )
        .await
        .unwrap();
        assert!(receipt.changed_conversation);
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "after"
        );
        assert_ne!(active_branch(&pool, "modes").await, branch);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn overwrite_with_third(root: &Path) {
        std::fs::write(root.join("a.txt"), "third").expect("tamper");
    }

    #[tokio::test]
    async fn newer_edit_during_compensation_stays_recoverable() {
        let pool = setup_migrated_pool().await;
        let (root, mut messages) = seed(&pool, "recover").await;
        record_files(
            &pool,
            "recover",
            "c1",
            "Write",
            "local",
            &root,
            &[
                CapturedFile {
                    path: "a.txt".into(),
                    before: Some("before".into()),
                    after: Some("after".into()),
                    unsupported: None,
                },
                CapturedFile {
                    path: "b.txt".into(),
                    before: Some("b".into()),
                    after: Some("B".into()),
                    unsupported: None,
                },
            ],
        )
        .await
        .unwrap();
        std::fs::write(Path::new(&root).join("a.txt"), "after").unwrap();
        std::fs::write(Path::new(&root).join("b.txt"), "B").unwrap();
        link_calls(&pool, "recover", &mut messages, &[("c1", "Write")]).await;
        let files = LiveFiles::disk("local", &root);
        let preview = preview(
            &pool,
            "recover",
            &messages[0].history_id,
            BoundaryEdge::Before,
            "files",
            true,
            &files,
        )
        .await
        .unwrap();
        let error = apply_preview(
            &pool,
            &preview,
            &files,
            ApplyOptions {
                request_id: "req-recover",
                interrupt_after: Some(1),
                before_compensate: Some(overwrite_with_third),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("回滚未成功"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "third"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("b.txt")).unwrap(),
            "B"
        );
        let blocked = block_until_rollback_settled(&pool, "recover", &files)
            .await
            .unwrap_err();
        assert!(blocked.contains("请先处理"));
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "third"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("b.txt")).unwrap(),
            "B"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn restart_restores_an_applying_rollback() {
        let pool = setup_migrated_pool().await;
        let (root, _messages) = seed(&pool, "restart").await;
        std::fs::write(Path::new(&root).join("a.txt"), "before").unwrap();
        let backup = r#"[{"path":"a.txt","text":"after","applied":true,"applied_text":"before"}]"#;
        sqlx::query(
            r#"
            INSERT INTO native_file_rollbacks (
                id, session_record_id, request_id, message_id, edge, mode, target_root,
                execution_target, preview_token, status, backup_json, created_at, updated_at
            ) VALUES ('rb1', 'restart', 'req-restart', 'm1', 'after', 'files', $1, 'local', 't', 'applying', $2, '2026-01-01 00:00:00', '2026-01-01 00:00:00')
            "#,
        )
        .bind(&root)
        .bind(backup)
        .execute(&pool)
        .await
        .unwrap();
        let files = LiveFiles::disk("local", &root);
        block_until_rollback_settled(&pool, "restart", &files)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&root).join("a.txt")).unwrap(),
            "after"
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM native_file_rollbacks WHERE id = 'rb1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
