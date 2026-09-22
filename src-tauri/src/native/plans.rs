//! Durable plan authorization and conflict-preserving atomic plan files.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::app::ssh::shell::shell_escape_single_quoted as quote;
use crate::native::tools::ssh::SshToolRuntime;

#[derive(Debug, Clone)]
pub struct PlanAuthorization(Arc<PlanAuthorizationInner>);

#[derive(Debug)]
struct PlanAuthorizationInner {
    state: Mutex<(bool, bool)>, // (current, implementation acknowledged)
    changed: tokio::sync::Notify,
}

impl PartialEq for PlanAuthorization {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for PlanAuthorization {}

impl PlanAuthorization {
    pub fn new() -> Self {
        Self(Arc::new(PlanAuthorizationInner {
            state: Mutex::new((true, false)),
            changed: tokio::sync::Notify::new(),
        }))
    }

    pub fn cancel(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0 = false;
        self.0.changed.notify_waiters();
    }

    /// Check and commit synchronously under the same lock used by invalidation.
    pub fn with_current<T>(&self, commit: impl FnOnce() -> T) -> Result<T, String> {
        let current = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !current.0 {
            return Err("计划授权已取消，请重新确认当前计划".into());
        }
        Ok(commit())
    }

    pub fn check(&self) -> Result<(), String> {
        self.with_current(|| ())
    }

    pub fn commit_implementation<T>(&self, commit: impl FnOnce() -> T) -> Result<T, String> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.0 {
            return Err("计划授权已取消，请重新确认当前计划".into());
        }
        let value = commit();
        state.1 = true;
        self.0.changed.notify_waiters();
        Ok(value)
    }

    pub async fn wait_for_implementation(&self) -> Result<(), String> {
        loop {
            let notified = self.0.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.1 {
                    return Ok(());
                }
                if !state.0 {
                    return Err("计划授权已取消，请重新确认当前计划".into());
                }
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingPlanSnapshot {
    pub request_id: String,
    pub plan: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanSaveStatus {
    Saving,
    Failed,
    Saved,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovedPlanSnapshot {
    pub authorization_id: String,
    pub request_id: String,
    pub body: String,
    pub feedback: String,
    pub cwd: String,
    pub path: String,
    /// False while a remote cwd still needs expansion/validation on the SSH host.
    #[serde(default = "resolved_cwd_default")]
    pub cwd_resolved: bool,
    pub content_hash: String,
    /// Last content actually written, retained when authorizing a new snapshot fails.
    pub saved_hash: Option<String>,
    /// The previous successful file remains distinct from an unresolved/new target.
    #[serde(default)]
    pub saved_path: Option<String>,
    pub status: PlanSaveStatus,
    pub ai_channel_id: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub error: Option<String>,
}

fn resolved_cwd_default() -> bool {
    true
}

impl ApprovedPlanSnapshot {
    pub fn last_saved_path(&self) -> Option<&str> {
        self.saved_hash.as_ref()?;
        self.saved_path
            .as_deref()
            .or_else(|| self.cwd_resolved.then_some(self.path.as_str()))
    }

    pub fn expected_hash(&self) -> Option<&str> {
        if self.last_saved_path() == Some(self.path.as_str()) {
            self.saved_hash.as_deref()
        } else {
            None
        }
    }
    pub fn content(&self) -> String {
        if self.feedback.trim().is_empty() {
            format!("{}\n", self.body.trim())
        } else {
            format!(
                "{}\n\n## 审批补充意见\n\n{}\n",
                self.body.trim(),
                self.feedback.trim()
            )
        }
    }

    pub fn retry_matches(&self, pending: &PendingPlanSnapshot) -> bool {
        self.request_id == pending.request_id
            && self.body == pending.plan
            && matches!(
                self.status,
                PlanSaveStatus::Saving | PlanSaveStatus::Failed | PlanSaveStatus::Saved
            )
    }
}

pub fn hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

pub fn relative_path(session_id: &str) -> Result<String, String> {
    if session_id.is_empty()
        || !session_id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_'))
    {
        return Err("计划会话标识含无效路径字符".into());
    }
    Ok(format!(".noxcode/plans/plan-{session_id}.md"))
}

pub async fn load_approved(
    pool: &SqlitePool,
    session: &str,
) -> Result<Option<ApprovedPlanSnapshot>, String> {
    let raw = sqlx::query_scalar::<_, Option<String>>(
        "SELECT approved_plan_json FROM agent_sessions WHERE id = $1",
    )
    .bind(session)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .flatten();
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| format!("读取已批准计划失败: {error}"))
    })
    .transpose()
}

pub async fn load_pending(pool: &SqlitePool, session: &str) -> Result<PendingPlanSnapshot, String> {
    load_pending_optional(pool, session)
        .await?
        .ok_or_else(|| "没有待批准的计划".to_string())
}

pub async fn load_pending_optional(
    pool: &SqlitePool,
    session: &str,
) -> Result<Option<PendingPlanSnapshot>, String> {
    let raw = sqlx::query_scalar::<_, Option<String>>(
        "SELECT pending_plan_json FROM agent_sessions WHERE id = $1",
    )
    .bind(session)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .flatten();
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|error| format!("读取待批准计划失败: {error}"))
    })
    .transpose()
}

/// Returns true only when an ordinary planning turn explicitly supersedes pending approval.
pub async fn validate_resume(
    pool: &SqlitePool,
    session: &str,
    plan_mode: bool,
    approved_request: Option<&str>,
) -> Result<bool, String> {
    match (
        load_pending_optional(pool, session).await?,
        approved_request,
    ) {
        (Some(pending), Some(request)) => {
            let saved = load_approved(pool, session).await?;
            if pending.request_id != request
                || !saved.is_some_and(|plan| {
                    plan.request_id == request
                        && plan.body == pending.plan
                        && plan.status == PlanSaveStatus::Saved
                        && plan.cwd_resolved
                        && plan.expected_hash() == Some(plan.content_hash.as_str())
                })
            {
                return Err("计划尚未保存，无法开始实施".into());
            }
            Ok(false)
        }
        (Some(_), None) if !plan_mode => Err("当前计划仍待批准或保存，请先完成计划审批".into()),
        (Some(_), None) => Ok(true),
        (None, Some(_)) => Err("计划批准请求已过期".into()),
        (None, None) => Ok(false),
    }
}

/// Caller holds the session operation lock; pending identity is still checked in SQL.
pub async fn store_approved(
    pool: &SqlitePool,
    session: &str,
    plan: &ApprovedPlanSnapshot,
) -> Result<(), String> {
    let payload = serde_json::to_string(plan).map_err(|error| error.to_string())?;
    let result = sqlx::query("UPDATE agent_sessions SET approved_plan_json = $1 WHERE id = $2 AND json_extract(pending_plan_json, '$.request_id') = $3 AND json_extract(pending_plan_json, '$.plan') = $4")
        .bind(payload).bind(session).bind(&plan.request_id).bind(&plan.body).execute(pool).await.map_err(|error| format!("保存计划授权失败: {error}"))?;
    if result.rows_affected() != 1 {
        return Err("计划批准请求已过期".into());
    }
    Ok(())
}

pub async fn record_failure(
    pool: &SqlitePool,
    session: &str,
    snapshot: &mut ApprovedPlanSnapshot,
    authorization: &PlanAuthorization,
    error: &str,
) -> Result<(), String> {
    snapshot.status = if authorization.check().is_ok() {
        PlanSaveStatus::Failed
    } else {
        PlanSaveStatus::Cancelled
    };
    snapshot.error = Some(error.to_string());
    store_approved(pool, session, snapshot).await
}

/// Persist consent before even starting an SSH lookup. This boundary also makes
/// initial connection failures and cancellation recoverable after app restart.
pub async fn resolve_authorized_ssh_target<T, F, Fut>(
    pool: &SqlitePool,
    session: &str,
    snapshot: &mut ApprovedPlanSnapshot,
    authorization: &PlanAuthorization,
    previous: Option<&ApprovedPlanSnapshot>,
    resolve: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(String, T), String>>,
{
    store_approved(pool, session, snapshot).await?;
    let outcome: Result<T, String> = async {
        authorization.check()?;
        let original_cwd = snapshot.cwd.clone();
        let original_path = snapshot.path.clone();
        let (cwd, prepared) = resolve().await?;
        authorization.check()?;
        if !cwd.starts_with('/') {
            return Err("远程计划工作目录不是有效绝对路径".into());
        }
        let path = crate::native::tools::paths::resolve_under_workspace_posix(
            &cwd,
            &relative_path(session)?,
        )?;
        if previous.is_some_and(|plan| {
            if plan.cwd_resolved {
                plan.cwd != cwd || plan.path != path
            } else {
                plan.cwd != original_cwd || plan.path != original_path
            }
        }) {
            authorization.cancel();
            return Err("计划工作目录已改变，请重新确认计划".into());
        }
        snapshot.cwd = cwd;
        snapshot.path = path;
        snapshot.cwd_resolved = true;
        store_approved(pool, session, snapshot).await?;
        Ok(prepared)
    }
    .await;
    if let Err(error) = &outcome {
        record_failure(pool, session, snapshot, authorization, error).await?;
    }
    outcome
}

/// Caller holds the session operation lock after cancelling the in-memory token.
/// Preserve successful history and the last saved hash; optionally discard the pending request.
pub async fn invalidate_persisted<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    session: &str,
    clear_pending: bool,
) -> Result<(), String> {
    sqlx::query("UPDATE agent_sessions SET approved_plan_json = CASE WHEN json_extract(approved_plan_json, '$.request_id') = json_extract(pending_plan_json, '$.request_id') THEN json_set(approved_plan_json, '$.status', 'cancelled') ELSE approved_plan_json END, pending_plan_json = CASE WHEN $2 THEN NULL ELSE pending_plan_json END WHERE id = $1")
        .bind(session).bind(clear_pending).execute(executor).await.map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn reference(pool: &SqlitePool, session: &str) -> Result<Option<String>, String> {
    Ok(load_approved(pool, session)
        .await?
        .filter(|plan| plan.status != PlanSaveStatus::Cancelled)
        .and_then(|plan| plan.last_saved_path().map(ToOwned::to_owned))
        .map(|path| {
            format!(
                "\n\n[已批准计划文件]\n{}\n后续工作请参考此文件中的计划。",
                path
            )
        }))
}

fn conflict() -> String {
    "计划文件已被修改，保存冲突。请保留你的修改并恢复上次保存内容后重试。".into()
}

fn check_existing(bytes: Option<&[u8]>, expected_hash: Option<&str>) -> Result<(), String> {
    match (bytes, expected_hash) {
        (None, None) => Ok(()),
        (Some(bytes), Some(expected)) if hash(bytes) == expected => Ok(()),
        _ => Err(conflict()),
    }
}

fn safe_local_target(cwd: &Path, session: &str) -> Result<PathBuf, String> {
    let relative = relative_path(session)?;
    let root = cwd.canonicalize().map_err(|error| error.to_string())?;
    let mut cursor = root.clone();
    let parts: Vec<_> = Path::new(&relative).components().collect();
    for (index, part) in parts.iter().enumerate() {
        cursor.push(part);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("计划路径包含符号链接".into())
            }
            Ok(metadata) if index + 1 < parts.len() && !metadata.is_dir() => {
                return Err("计划目录不是目录".into())
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index + 1 < parts.len() =>
            {
                std::fs::create_dir(&cursor).map_err(|error| error.to_string())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(cursor)
}

pub fn write_local(
    cwd: &Path,
    session: &str,
    content: &str,
    expected_hash: Option<&str>,
    authorization: &PlanAuthorization,
) -> Result<String, String> {
    authorization.check()?;
    let target = safe_local_target(cwd, session)?;
    let existing = match std::fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    // Recover a crash after rename but before the saved status reached SQLite.
    // Identical bytes need no replacement, so user data cannot be overwritten.
    if existing.as_deref() == Some(content.as_bytes()) {
        authorization.check()?;
        return Ok(target.to_string_lossy().into_owned());
    }
    check_existing(existing.as_deref(), expected_hash)?;
    let mut temporary = tempfile::NamedTempFile::new_in(target.parent().unwrap())
        .map_err(|error| error.to_string())?;
    temporary
        .write_all(content.as_bytes())
        .and_then(|()| temporary.flush())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    authorization.with_current(|| {
        // Recheck immediately before replacing; never unlink the old document.
        safe_local_target(cwd, session)?;
        let latest = match std::fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        check_existing(latest.as_deref(), expected_hash)?;
        temporary
            .persist(&target)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(target.to_string_lossy().into_owned())
    })?
}

fn ssh_guard(root: &str, target: &str) -> Result<String, String> {
    let target = crate::native::tools::paths::resolve_under_workspace_posix(root, target)?;
    let mut cursor = String::new();
    let mut checks = Vec::new();
    for part in target.split('/').filter(|part| !part.is_empty()) {
        cursor.push('/');
        cursor.push_str(part);
        checks.push(format!("[ ! -L {} ]", quote(&cursor)));
    }
    Ok(format!(
        "{} || {{ printf '%s\\n' '计划路径包含符号链接' >&2; exit 73; }}; ",
        checks.join(" && ")
    ))
}

pub fn ssh_resolve_cwd_command(root: &str) -> String {
    format!(
        "cd {} && pwd -P",
        crate::app::ssh::shell::remote_shell_path_expression(root)
    )
}

/// Expand the managed worktree's `$HOME` on the remote host, then anchor all paths
/// to the actual absolute cwd before containment checks or file operations.
pub async fn resolve_ssh_cwd(ssh: &SshToolRuntime) -> Result<String, String> {
    let output = crate::app::ssh::exec::execute_ssh_command(
        &ssh.app,
        &ssh.config,
        &ssh_resolve_cwd_command(&ssh.root),
        true,
    )
    .await?;
    if !output.success() {
        return Err(format!(
            "解析远程计划工作目录失败: {}",
            output.stderr_lossy()
        ));
    }
    let cwd = output
        .stdout_lossy()
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if !cwd.starts_with('/') || cwd.contains(['\r', '\n']) {
        return Err("远程计划工作目录不是有效绝对路径".into());
    }
    Ok(cwd)
}

pub fn ssh_write_command(
    root: &str,
    session: &str,
    expected_content: Option<&str>,
) -> Result<String, String> {
    if !root.starts_with('/') {
        return Err("请先解析远程计划工作目录为绝对路径".into());
    }
    let target =
        crate::native::tools::paths::resolve_under_workspace_posix(root, &relative_path(session)?)?;
    let parent = target.rsplit_once('/').unwrap().0;
    let guard = ssh_guard(root, &target)?;
    let compare = match expected_content {
        Some(content) => format!("printf '%s' {} > \"$plan_expected\"; [ -f {target} ] && cmp -s {target} \"$plan_expected\"", quote(content), target = quote(&target)),
        None => format!("[ ! -e {} ]", quote(&target)),
    };
    Ok(format!("set -e; umask 077; {guard}mkdir -p {parent}; plan_tmp=$(mktemp {template}); plan_expected=$(mktemp {template}); trap 'rm -f \"$plan_tmp\" \"$plan_expected\"' EXIT HUP INT TERM; cat > \"$plan_tmp\"; {guard}{{ {compare}; }} || {{ printf '%s\\n' '计划文件已被修改，保存冲突' >&2; exit 74; }}; mv -f \"$plan_tmp\" {target}", parent = quote(parent), template = quote(&format!("{parent}/.plan-XXXXXXXX")), target = quote(&target)))
}

pub async fn write_ssh(
    ssh: &SshToolRuntime,
    session: &str,
    content: &str,
    expected_hash: Option<&str>,
    authorization: &PlanAuthorization,
) -> Result<String, String> {
    use crate::app::ssh::exec::{execute_ssh_command, execute_ssh_command_with_input};
    let root = resolve_ssh_cwd(ssh).await?;
    let target = crate::native::tools::paths::resolve_under_workspace_posix(
        &root,
        &relative_path(session)?,
    )?;
    let command = format!(
        "{}if [ -e {path} ]; then cat {path}; else exit 44; fi",
        ssh_guard(&root, &target)?,
        path = quote(&target)
    );
    let existing = execute_ssh_command(&ssh.app, &ssh.config, &command, true).await?;
    let existing = match existing.exit_code {
        Some(0) => Some(existing.stdout_lossy()),
        Some(44) => None,
        _ => return Err(existing.stderr_lossy()),
    };
    if existing.as_deref() == Some(content) {
        authorization.check()?;
        return Ok(target);
    }
    check_existing(existing.as_deref().map(str::as_bytes), expected_hash)?;
    authorization.check()?;
    let command = ssh_write_command(&root, session, existing.as_deref())?;
    let output = execute_ssh_command_with_input(
        &ssh.app,
        &ssh.config,
        &command,
        content.as_bytes().to_vec(),
        true,
    )
    .await?;
    if !output.success() {
        return Err(output.stderr_lossy());
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unresolved_plan_snapshot() -> ApprovedPlanSnapshot {
        ApprovedPlanSnapshot {
            authorization_id: "authorization".into(),
            request_id: "request".into(),
            body: "approved body".into(),
            feedback: "keep these notes".into(),
            cwd: "$HOME/worktree".into(),
            path: "$HOME/worktree/.noxcode/plans/plan-session.md".into(),
            cwd_resolved: false,
            content_hash: String::new(),
            saved_hash: None,
            saved_path: None,
            status: PlanSaveStatus::Saving,
            ai_channel_id: "selected-channel".into(),
            model: "selected-model".into(),
            reasoning_effort: Some("high".into()),
            error: None,
        }
    }

    #[tokio::test]
    async fn initial_ssh_resolution_failure_persists_consent_and_retries_after_reload() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        let root = tempfile::tempdir().unwrap();
        let resolved = root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let previous_path = write_local(
            root.path(),
            "session",
            "old plan\n",
            None,
            &PlanAuthorization::new(),
        )
        .unwrap();
        let mut snapshot = unresolved_plan_snapshot();
        snapshot.saved_hash = Some(hash(b"old plan\n"));
        snapshot.saved_path = Some(previous_path.clone());
        snapshot.content_hash = hash(snapshot.content().as_bytes());
        let mut old = snapshot.clone();
        old.model = "historical-model".into();
        old.ai_channel_id = "historical-channel".into();
        old.path = previous_path.clone();
        old.cwd = resolved.clone();
        old.cwd_resolved = true;
        old.status = PlanSaveStatus::Saved;
        let pending = PendingPlanSnapshot {
            request_id: snapshot.request_id.clone(),
            plan: snapshot.body.clone(),
            created_at: "t".into(),
        };
        sqlx::query("INSERT INTO agent_sessions (id, pending_plan_json, approved_plan_json) VALUES ('session', $1, $2)")
            .bind(serde_json::to_string(&pending).unwrap()).bind(serde_json::to_string(&old).unwrap()).execute(&pool).await.unwrap();
        let error = resolve_authorized_ssh_target(
            &pool,
            "session",
            &mut snapshot,
            &PlanAuthorization::new(),
            None,
            || async {
                let stored = load_approved(&pool, "session").await.unwrap().unwrap();
                assert_eq!(stored.model, "selected-model");
                assert_eq!(stored.ai_channel_id, "selected-channel");
                assert_eq!(stored.feedback, "keep these notes");
                assert_eq!(stored.reasoning_effort.as_deref(), Some("high"));
                assert_eq!(stored.status, PlanSaveStatus::Saving);
                assert!(!stored.cwd_resolved);
                Err::<(String, ()), _>("SSH unavailable".to_string())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH unavailable");
        let mut retry = load_approved(&pool, "session").await.unwrap().unwrap();
        assert_eq!(retry.status, PlanSaveStatus::Failed);
        assert!(retry.retry_matches(&pending));
        assert_eq!(retry.saved_hash, Some(hash(b"old plan\n")));
        assert_eq!(retry.last_saved_path(), Some(previous_path.as_str()));
        assert!(reference(&pool, "session")
            .await
            .unwrap()
            .unwrap()
            .contains(&previous_path));
        let previous = retry.clone();
        retry.status = PlanSaveStatus::Saving;
        retry.error = None;
        resolve_authorized_ssh_target(
            &pool,
            "session",
            &mut retry,
            &PlanAuthorization::new(),
            Some(&previous),
            || async { Ok((resolved.clone(), ())) },
        )
        .await
        .unwrap();
        assert_eq!(retry.model, "selected-model");
        assert_eq!(retry.ai_channel_id, "selected-channel");
        assert_eq!(retry.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(retry.feedback, "keep these notes");
        assert!(retry.cwd_resolved);
        write_local(
            Path::new(&retry.cwd),
            "session",
            &retry.content(),
            retry.expected_hash(),
            &PlanAuthorization::new(),
        )
        .unwrap();
        retry.saved_hash = Some(retry.content_hash.clone());
        retry.saved_path = Some(retry.path.clone());
        retry.status = PlanSaveStatus::Saved;
        store_approved(&pool, "session", &retry).await.unwrap();
        assert!(!validate_resume(&pool, "session", false, Some("request"))
            .await
            .unwrap());
        assert_eq!(
            std::fs::read_to_string(previous_path).unwrap(),
            retry.content()
        );
    }

    #[tokio::test]
    async fn cancellation_during_ssh_resolution_is_durable_without_unlocking() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id, pending_plan_json) VALUES ('session', '{\"request_id\":\"request\",\"plan\":\"approved body\",\"created_at\":\"t\"}')").execute(&pool).await.unwrap();
        let authorization = PlanAuthorization::new();
        let resolving = authorization.clone();
        let task_pool = pool.clone();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut snapshot = unresolved_plan_snapshot();
            resolve_authorized_ssh_target(
                &task_pool,
                "session",
                &mut snapshot,
                &resolving,
                None,
                || async {
                    started.send(()).unwrap();
                    release_rx.await.unwrap();
                    Ok(("/remote/worktree".into(), ()))
                },
            )
            .await
        });
        started_rx.await.unwrap();
        let saving = load_approved(&pool, "session").await.unwrap().unwrap();
        assert_eq!(saving.status, PlanSaveStatus::Saving);
        assert_eq!(saving.model, "selected-model");
        authorization.cancel();
        release.send(()).unwrap();
        assert!(task.await.unwrap().is_err());
        let stored = load_approved(&pool, "session").await.unwrap().unwrap();
        assert_eq!(stored.status, PlanSaveStatus::Cancelled);
        assert_eq!(stored.model, "selected-model");
        assert!(!stored.cwd_resolved);
        assert_eq!(
            load_pending(&pool, "session").await.unwrap().request_id,
            "request"
        );
        assert!(authorization.commit_implementation(|| ()).is_err());
        assert!(validate_resume(&pool, "session", false, Some("request"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn acknowledgement_distinguishes_inflight_cancellation_from_completed_implementation() {
        let interrupted = PlanAuthorization::new();
        interrupted.cancel();
        assert!(interrupted.wait_for_implementation().await.is_err());
        let completed = PlanAuthorization::new();
        completed.commit_implementation(|| ()).unwrap();
        completed.cancel();
        completed.wait_for_implementation().await.unwrap();
    }

    #[tokio::test]
    async fn approval_round_trip_preserves_prior_hash_and_survives_continuation() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        let pending = PendingPlanSnapshot {
            request_id: "request".into(),
            plan: "approved body".into(),
            created_at: "t".into(),
        };
        sqlx::query("INSERT INTO agent_sessions (id, pending_plan_json) VALUES ('session', $1)")
            .bind(serde_json::to_string(&pending).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let mut plan = ApprovedPlanSnapshot {
            authorization_id: "auth".into(),
            request_id: pending.request_id.clone(),
            body: pending.plan.clone(),
            feedback: "add tests".into(),
            cwd: "/worktree".into(),
            cwd_resolved: true,
            saved_path: None,
            path: "/worktree/.noxcode/plans/plan-session.md".into(),
            content_hash: "new-content".into(),
            saved_hash: Some("old-content".into()),
            status: PlanSaveStatus::Saving,
            ai_channel_id: "channel".into(),
            model: "implementation-model".into(),
            reasoning_effort: Some("high".into()),
            error: None,
        };
        store_approved(&pool, "session", &plan).await.unwrap();
        plan.status = PlanSaveStatus::Failed;
        plan.error = Some("disk full".into());
        store_approved(&pool, "session", &plan).await.unwrap();
        assert_eq!(
            load_approved(&pool, "session").await.unwrap(),
            Some(plan.clone())
        );
        assert_eq!(load_pending(&pool, "session").await.unwrap(), pending);
        assert!(plan.retry_matches(&pending));
        assert!(validate_resume(&pool, "session", false, None)
            .await
            .is_err());
        assert!(validate_resume(&pool, "session", false, Some("request"))
            .await
            .is_err());
        assert!(validate_resume(&pool, "session", true, None).await.unwrap());
        assert!(!plan.retry_matches(&PendingPlanSnapshot {
            plan: "changed".into(),
            ..pending.clone()
        }));
        plan.status = PlanSaveStatus::Saved;
        plan.saved_hash = Some(plan.content_hash.clone());
        plan.error = None;
        store_approved(&pool, "session", &plan).await.unwrap();
        assert!(!validate_resume(&pool, "session", false, Some("request"))
            .await
            .unwrap());
        sqlx::query("UPDATE agent_sessions SET pending_plan_json = NULL, status = 'running' WHERE id = 'session'").execute(&pool).await.unwrap();
        invalidate_persisted(&pool, "session", true).await.unwrap();
        assert_eq!(load_approved(&pool, "session").await.unwrap(), Some(plan));
        assert!(reference(&pool, "session")
            .await
            .unwrap()
            .unwrap()
            .contains("/worktree/.noxcode/plans/plan-session.md"));
    }

    #[tokio::test]
    async fn cancellation_and_changed_request_reject_stale_durable_updates() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id, pending_plan_json) VALUES ('session', '{\"request_id\":\"new\",\"plan\":\"new plan\",\"created_at\":\"t\"}')").execute(&pool).await.unwrap();
        let mut plan = ApprovedPlanSnapshot {
            authorization_id: "auth".into(),
            request_id: "old".into(),
            body: "old plan".into(),
            feedback: String::new(),
            cwd: "/worktree".into(),
            cwd_resolved: true,
            saved_path: None,
            path: "/worktree/plan.md".into(),
            content_hash: "hash".into(),
            saved_hash: None,
            status: PlanSaveStatus::Saving,
            ai_channel_id: "channel".into(),
            model: "model".into(),
            reasoning_effort: None,
            error: None,
        };
        assert!(store_approved(&pool, "session", &plan).await.is_err());
        plan.request_id = "new".into();
        plan.body = "new plan".into();
        store_approved(&pool, "session", &plan).await.unwrap();
        invalidate_persisted(&pool, "session", true).await.unwrap();
        assert_eq!(
            load_approved(&pool, "session")
                .await
                .unwrap()
                .unwrap()
                .status,
            PlanSaveStatus::Cancelled
        );
        assert!(load_pending(&pool, "session").await.is_err());
        assert!(store_approved(&pool, "session", &plan).await.is_err());
    }

    #[test]
    fn local_worktree_atomic_replace_conflict_and_retry() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("worktree");
        std::fs::create_dir(&cwd).unwrap();
        let authorization = PlanAuthorization::new();
        let path = write_local(&cwd, "session", "first\n", None, &authorization).unwrap();
        let expected = hash(b"first\n");
        std::fs::write(&path, "user edit").unwrap();
        assert!(
            write_local(&cwd, "session", "second", Some(&expected), &authorization)
                .unwrap_err()
                .contains("冲突")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user edit");
        std::fs::write(&path, "first\n").unwrap();
        write_local(&cwd, "session", "second", Some(&expected), &authorization).unwrap();
        // A crash after rename still retries idempotently with the previous saved hash.
        write_local(&cwd, "session", "second", Some(&expected), &authorization).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "second");
        assert!(!dir.path().join(".noxcode").exists());
    }

    #[test]
    fn cancelled_authorization_never_writes_or_unlocks() {
        let dir = tempfile::tempdir().unwrap();
        let authorization = PlanAuthorization::new();
        authorization.cancel();
        let mut unlocked = false;
        assert!(authorization.with_current(|| unlocked = true).is_err());
        assert!(!unlocked);
        assert!(write_local(dir.path(), "session", "denied", None, &authorization).is_err());
        assert!(!dir.path().join(".noxcode").exists());
    }

    #[cfg(unix)]
    #[test]
    fn local_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".noxcode")).unwrap();
        assert!(write_local(
            dir.path(),
            "session",
            "body",
            None,
            &PlanAuthorization::new()
        )
        .is_err());
        assert!(!outside.path().join("plans").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ssh_plan_symbolic_worktree_root_is_resolved_before_writing() {
        use std::process::{Command, Stdio};
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("home"));
        let worktrees = home.join(".noxcode/worktrees");
        std::fs::create_dir_all(&worktrees).unwrap();
        let worktree = tempfile::Builder::new()
            .prefix("plan-test-")
            .tempdir_in(worktrees)
            .unwrap();
        let session = worktree.path().file_name().unwrap().to_str().unwrap();
        let symbolic = crate::git::worktree::remote_worktree_path(session);
        let unrelated = tempfile::tempdir().unwrap();
        for root in [
            symbolic.clone(),
            symbolic.replacen("$HOME", "~", 1),
            symbolic.replacen("$HOME", "${HOME}", 1),
        ] {
            let resolved = Command::new("sh")
                .arg("-c")
                .arg(ssh_resolve_cwd_command(&root))
                .current_dir(unrelated.path())
                .output()
                .unwrap();
            assert!(
                resolved.status.success(),
                "{}",
                String::from_utf8_lossy(&resolved.stderr)
            );
            let absolute = String::from_utf8(resolved.stdout).unwrap();
            let absolute = absolute.trim_end();
            assert_eq!(Path::new(absolute), worktree.path().canonicalize().unwrap());
            let mut writer = Command::new("sh")
                .arg("-c")
                .arg(ssh_write_command(absolute, "session", None).unwrap())
                .current_dir(unrelated.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writer
                .stdin
                .take()
                .unwrap()
                .write_all(b"approved plan")
                .unwrap();
            let output = writer.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let target = worktree.path().join(relative_path("session").unwrap());
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "approved plan");
            std::fs::remove_file(target).unwrap();
            assert!(!unrelated.path().join("$HOME").exists());
        }
        assert!(ssh_write_command(&symbolic, "session", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ssh_shell_atomic_replace_and_conflict() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        // macOS /var is a symlink; the remote guard intentionally rejects symlinks.
        let root = dir.path().canonicalize().unwrap();
        let root = root.to_str().unwrap();
        let run = |expected, body: &str| {
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(ssh_write_command(root, "session", expected).unwrap())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            if let Err(error) = child.stdin.take().unwrap().write_all(body.as_bytes()) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
            child.wait_with_output().unwrap()
        };
        assert!(run(None, "first\n").status.success());
        let path = Path::new(root).join(relative_path("session").unwrap());
        assert!(!run(None, "bad").status.success());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        assert!(run(Some("first\n"), "second 'quoted'\n").status.success());
        assert!(!run(Some("first\n"), "bad").status.success());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second 'quoted'\n");
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "outside content").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(outside.path(), &path).unwrap();
        assert!(!run(Some("outside content"), "bad").status.success());
        assert_eq!(
            std::fs::read_to_string(outside.path()).unwrap(),
            "outside content"
        );
    }
}
