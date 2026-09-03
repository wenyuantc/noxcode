pub mod preflight;
pub(crate) mod runner;

mod checkpoint;
mod commit;
mod diff;
mod repo;
mod stage;
mod status;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::sqlite_pool;
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::app::ssh::{resolve_connect_params, SshPool};
use crate::engine::context::resolve_workspace_execution_context_with_pool;

use self::checkpoint::{list_checkpoints, preview_restore, restore_checkpoint, sweep_orphan_refs};
use self::commit::{
    commit_changes, create_branch, list_branches, push_branch, GitBranch, GitCommitResult,
    GitPushResult,
};
use self::diff::{
    get_file_diff, get_numstat, GitFileDiff, GitFileDiffScope, GitNumstatEntry, GitNumstatScope,
};
use self::repo::{list_repo_files, GitRepoInfo};
use self::stage::{restore_paths, stage_paths, unstage_paths};
use self::status::{get_status, GitStatus};

pub(crate) use self::checkpoint::{
    clear_workspace_checkpoints, create_checkpoint, delete_checkpoints_for_session,
    GitCheckpointInfo, GitRestorePreview, GitRestoreResult,
};
pub(crate) use self::repo::load_repo_info;
pub(crate) use self::runner::{GitTarget, IndexMode};

pub(crate) async fn resolve_git_target<R: Runtime>(
    app: &AppHandle<R>,
    workspace_id: &str,
) -> Result<GitTarget, String> {
    let pool = sqlite_pool(app).await?;
    let ctx = resolve_workspace_execution_context_with_pool(&pool, workspace_id).await?;
    let is_ssh = ctx.is_ssh();
    let working_dir = ctx
        .working_dir
        .ok_or_else(|| "工作区缺少工作目录".to_string())?;
    if is_ssh {
        let ssh_config_id = ctx
            .ssh_config_id
            .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
        let record = fetch_ssh_config_record_by_id(&pool, &ssh_config_id).await?;
        let params = resolve_connect_params(app, &record, true)?;
        Ok(GitTarget::Ssh {
            pool: app.state::<SshPool>().inner().clone(),
            params,
            repo_path: working_dir,
        })
    } else {
        Ok(GitTarget::Local(PathBuf::from(working_dir)))
    }
}

#[tauri::command]
pub(crate) async fn get_git_repo_info<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<GitRepoInfo, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    if let Err(error) = sweep_orphan_refs(&pool, &target).await {
        eprintln!("[git] 清扫孤儿 checkpoint ref 失败: {error}");
    }
    load_repo_info(&target, &workspace_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn get_git_status<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    untracked_mode: Option<String>,
) -> Result<GitStatus, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    get_status(&target, untracked_mode.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn get_git_file_diff<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    path: String,
    scope: GitFileDiffScope,
    old_path: Option<String>,
) -> Result<GitFileDiff, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    get_file_diff(&target, &path, &scope, old_path.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn get_git_numstat<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    scope: GitNumstatScope,
) -> Result<Vec<GitNumstatEntry>, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    get_numstat(&target, &scope).await.map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn stage_git_paths<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    stage_paths(&target, &paths).await.map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn unstage_git_paths<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    unstage_paths(&target, &paths).await.map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn restore_git_paths<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    restore_paths(&target, &paths).await.map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn commit_git_changes<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    message: String,
    paths: Option<Vec<String>>,
) -> Result<GitCommitResult, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    commit_changes(&target, &message, paths.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn push_git_branch<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    remote: Option<String>,
    branch: Option<String>,
    set_upstream: bool,
) -> Result<GitPushResult, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    push_branch(&target, remote.as_deref(), branch.as_deref(), set_upstream)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn list_git_branches<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<Vec<GitBranch>, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    list_branches(&target).await.map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn create_git_branch<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    name: String,
    checkout: bool,
) -> Result<GitBranch, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    create_branch(&target, &name, checkout)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn create_git_checkpoint<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    session_id: String,
    label: Option<String>,
    kind: Option<String>,
) -> Result<GitCheckpointInfo, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    create_checkpoint(
        &pool,
        &target,
        &workspace_id,
        &session_id,
        label.as_deref(),
        kind.as_deref(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn list_git_checkpoints<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    session_id: String,
) -> Result<Vec<GitCheckpointInfo>, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    list_checkpoints(&pool, &target, &workspace_id, &session_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn preview_git_checkpoint_restore<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    checkpoint_id: String,
) -> Result<GitRestorePreview, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    preview_restore(&pool, &target, &checkpoint_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn restore_git_checkpoint<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    checkpoint_id: String,
    delete_new_paths: Option<Vec<String>>,
) -> Result<GitRestoreResult, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    restore_checkpoint(
        &pool,
        &target,
        &workspace_id,
        &checkpoint_id,
        delete_new_paths.as_deref().unwrap_or(&[]),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn list_git_files<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
    query: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<String>, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    list_repo_files(&target, query.as_deref(), limit.map(|value| value as usize))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(crate) async fn clear_git_checkpoints<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<u64, String> {
    let target = resolve_git_target(&app, &workspace_id).await?;
    let pool = sqlite_pool(&app).await?;
    clear_workspace_checkpoints(&pool, &target, &workspace_id)
        .await
        .map_err(Into::into)
}
