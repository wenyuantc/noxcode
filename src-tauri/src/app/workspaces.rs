use std::fs;
use std::path::Path;
use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime, State};
use tokio::sync::Mutex;

use crate::app::shared::{
    new_id, normalize_optional_text, now_sqlite, sqlite_pool, WORKSPACE_TYPE_LOCAL,
    WORKSPACE_TYPE_SSH,
};
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::app::ssh::exec::ExecOptions;
use crate::app::ssh::SshPool;
use crate::db::models::{CreateWorkspace, UpdateWorkspace, Workspace, WorkspaceHealth};
use crate::git::{clear_workspace_checkpoints, load_repo_info, resolve_git_target};
use crate::native::manager::NativeAgentManager;

fn normalize_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("工作区名称不能为空".to_string());
    }
    Ok(name.to_string())
}

fn normalize_workspace_type(value: &str) -> Result<String, String> {
    match value.trim() {
        WORKSPACE_TYPE_LOCAL => Ok(WORKSPACE_TYPE_LOCAL.to_string()),
        WORKSPACE_TYPE_SSH => Ok(WORKSPACE_TYPE_SSH.to_string()),
        other => Err(format!("不支持的工作区类型: {other}")),
    }
}

type WorkspacePaths = (Option<String>, Option<String>, Option<String>);

fn validate_workspace_fields(
    workspace_type: &str,
    repo_path: Option<&str>,
    ssh_config_id: Option<&str>,
    remote_repo_path: Option<&str>,
) -> Result<WorkspacePaths, String> {
    match workspace_type {
        value if value == WORKSPACE_TYPE_LOCAL => {
            let repo_path = repo_path
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "本地工作区必须提供 repo_path".to_string())?;
            if !Path::new(repo_path).is_dir() {
                return Err(format!("本地仓库路径不存在或不是目录: {repo_path}"));
            }
            Ok((Some(repo_path.to_string()), None, None))
        }
        value if value == WORKSPACE_TYPE_SSH => {
            let ssh_config_id = ssh_config_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "SSH 工作区必须提供 ssh_config_id".to_string())?;
            let remote_repo_path = remote_repo_path
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "SSH 工作区必须提供 remote_repo_path".to_string())?;
            Ok((
                None,
                Some(ssh_config_id.to_string()),
                Some(remote_repo_path.to_string()),
            ))
        }
        other => Err(format!("不支持的工作区类型: {other}")),
    }
}

pub(crate) async fn fetch_workspace_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Workspace, String> {
    sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces WHERE id = $1 LIMIT 1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("Failed to load workspace: {error}"))?
        .ok_or_else(|| format!("工作区不存在: {id}"))
}

pub(crate) async fn list_workspaces_with(pool: &SqlitePool) -> Result<Vec<Workspace>, String> {
    sqlx::query_as::<_, Workspace>("SELECT * FROM workspaces ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await
        .map_err(|error| format!("读取工作区列表失败: {error}"))
}

pub(crate) async fn create_workspace_with(
    pool: &SqlitePool,
    payload: CreateWorkspace,
) -> Result<Workspace, String> {
    let name = normalize_name(&payload.name)?;
    let workspace_type = normalize_workspace_type(&payload.workspace_type)?;
    let (repo_path, ssh_config_id, remote_repo_path) = validate_workspace_fields(
        &workspace_type,
        payload.repo_path.as_deref(),
        payload.ssh_config_id.as_deref(),
        payload.remote_repo_path.as_deref(),
    )?;
    if let Some(ssh_config_id) = ssh_config_id.as_deref() {
        let _ = fetch_ssh_config_record_by_id(pool, ssh_config_id).await?;
    }
    let id = new_id();
    let now = now_sqlite();
    sqlx::query(
        "INSERT INTO workspaces (id, name, workspace_type, repo_path, ssh_config_id, remote_repo_path, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&workspace_type)
    .bind(&repo_path)
    .bind(&ssh_config_id)
    .bind(&remote_repo_path)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("创建工作区失败: {error}"))?;
    fetch_workspace_by_id(pool, &id).await
}

pub(crate) async fn update_workspace_with(
    pool: &SqlitePool,
    id: &str,
    updates: UpdateWorkspace,
) -> Result<Workspace, String> {
    let current = fetch_workspace_by_id(pool, id).await?;
    let name = match updates.name.as_deref() {
        Some(value) => normalize_name(value)?,
        None => current.name,
    };
    let workspace_type = match updates.workspace_type.as_deref() {
        Some(value) => normalize_workspace_type(value)?,
        None => current.workspace_type,
    };
    let repo_path = match updates.repo_path {
        Some(value) => normalize_optional_text(value.as_deref()),
        None => current.repo_path,
    };
    let ssh_config_id = match updates.ssh_config_id {
        Some(value) => normalize_optional_text(value.as_deref()),
        None => current.ssh_config_id,
    };
    let remote_repo_path = match updates.remote_repo_path {
        Some(value) => normalize_optional_text(value.as_deref()),
        None => current.remote_repo_path,
    };
    let (repo_path, ssh_config_id, remote_repo_path) = validate_workspace_fields(
        &workspace_type,
        repo_path.as_deref(),
        ssh_config_id.as_deref(),
        remote_repo_path.as_deref(),
    )?;
    if let Some(ssh_config_id) = ssh_config_id.as_deref() {
        let _ = fetch_ssh_config_record_by_id(pool, ssh_config_id).await?;
    }
    let now = now_sqlite();
    sqlx::query(
        "UPDATE workspaces SET name = $1, workspace_type = $2, repo_path = $3, ssh_config_id = $4, remote_repo_path = $5, updated_at = $6 WHERE id = $7",
    )
    .bind(&name)
    .bind(&workspace_type)
    .bind(&repo_path)
    .bind(&ssh_config_id)
    .bind(&remote_repo_path)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新工作区失败: {error}"))?;
    fetch_workspace_by_id(pool, id).await
}

pub(crate) async fn delete_workspace_row(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM workspaces WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除工作区失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err(format!("工作区不存在: {id}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_workspaces<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Workspace>, String> {
    let pool = sqlite_pool(&app).await?;
    list_workspaces_with(&pool).await
}

#[tauri::command]
pub async fn create_workspace<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateWorkspace,
) -> Result<Workspace, String> {
    let pool = sqlite_pool(&app).await?;
    create_workspace_with(&pool, payload).await
}

#[tauri::command]
pub async fn update_workspace<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    updates: UpdateWorkspace,
) -> Result<Workspace, String> {
    let pool = sqlite_pool(&app).await?;
    update_workspace_with(&pool, &id, updates).await
}

#[tauri::command]
pub async fn delete_workspace<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    id: String,
) -> Result<(), String> {
    if state.lock().await.has_workspace_processes(&id) {
        return Err("该工作区有运行中的会话，无法删除".to_string());
    }
    let pool = sqlite_pool(&app).await?;
    if let Ok(target) = resolve_git_target(&app, &id).await {
        if let Err(error) = clear_workspace_checkpoints(&pool, &target, &id).await {
            eprintln!("[git] 清理工作区 checkpoint 失败: {error}");
        }
    }
    delete_workspace_row(&pool, &id).await
}

const SCRATCH_DIR_NAME: &str = "scratch";
const SCRATCH_WORKSPACE_NAME: &str = "临时工作区";

pub(crate) async fn ensure_scratch_workspace_with(
    pool: &SqlitePool,
    scratch_dir: &Path,
) -> Result<Workspace, String> {
    fs::create_dir_all(scratch_dir).map_err(|error| format!("创建临时工作区目录失败: {error}"))?;
    let repo_path = scratch_dir
        .canonicalize()
        .map_err(|error| format!("解析临时工作区路径失败: {error}"))?
        .to_string_lossy()
        .into_owned();
    if let Some(existing) = sqlx::query_as::<_, Workspace>(
        "SELECT * FROM workspaces WHERE workspace_type = $1 AND repo_path = $2 LIMIT 1",
    )
    .bind(WORKSPACE_TYPE_LOCAL)
    .bind(&repo_path)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("查询临时工作区失败: {error}"))?
    {
        return Ok(existing);
    }
    create_workspace_with(
        pool,
        CreateWorkspace {
            name: SCRATCH_WORKSPACE_NAME.to_string(),
            workspace_type: WORKSPACE_TYPE_LOCAL.to_string(),
            repo_path: Some(repo_path),
            ssh_config_id: None,
            remote_repo_path: None,
        },
    )
    .await
}

#[tauri::command]
pub async fn ensure_scratch_workspace<R: Runtime>(app: AppHandle<R>) -> Result<Workspace, String> {
    let pool = sqlite_pool(&app).await?;
    let scratch_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?
        .join(SCRATCH_DIR_NAME);
    ensure_scratch_workspace_with(&pool, &scratch_dir).await
}

#[tauri::command]
pub async fn check_workspace_health<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: String,
) -> Result<WorkspaceHealth, String> {
    let pool = sqlite_pool(&app).await?;
    let workspace = fetch_workspace_by_id(&pool, &workspace_id).await?;
    if workspace.workspace_type == WORKSPACE_TYPE_SSH {
        let ssh_config_id = workspace
            .ssh_config_id
            .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
        let remote = workspace
            .remote_repo_path
            .ok_or_else(|| "SSH 工作区缺少 remote_repo_path".to_string())?;
        let record = fetch_ssh_config_record_by_id(&pool, &ssh_config_id).await?;
        let params = crate::app::ssh::resolve_connect_params(&app, &record, true)?;
        let ssh_pool = app.state::<SshPool>().inner().clone();
        let command = format!(
            "test -d {} && (git --version 2>/dev/null || echo 'git: not found')",
            crate::app::ssh::shell::shell_escape_single_quoted(&remote)
        );
        return match ssh_pool
            .exec(&params, &command, ExecOptions::default())
            .await
        {
            Ok(output) if output.success() => Ok(WorkspaceHealth {
                workspace_id,
                ok: true,
                message: "SSH 工作区目录可用".to_string(),
                git_version: Some(output.stdout_lossy().trim().to_string())
                    .filter(|item| !item.is_empty() && !item.contains("not found")),
                toplevel: Some(remote),
            }),
            Ok(output) => Ok(WorkspaceHealth {
                workspace_id,
                ok: false,
                message: format!(
                    "SSH 工作区检查失败: exit={:?} {}",
                    output.exit_code,
                    output.stderr_lossy()
                ),
                git_version: None,
                toplevel: None,
            }),
            Err(error) => Ok(WorkspaceHealth {
                workspace_id,
                ok: false,
                message: error.to_string(),
                git_version: None,
                toplevel: None,
            }),
        };
    }

    match resolve_git_target(&app, &workspace_id).await {
        Ok(target) => match load_repo_info(&target, &workspace_id).await {
            Ok(info) => Ok(WorkspaceHealth {
                workspace_id,
                ok: true,
                message: "本地 Git 仓库可用".to_string(),
                git_version: Some(info.git_version),
                toplevel: Some(info.toplevel),
            }),
            Err(error) => Ok(WorkspaceHealth {
                workspace_id,
                ok: false,
                message: error.to_string(),
                git_version: None,
                toplevel: None,
            }),
        },
        Err(error) => Ok(WorkspaceHealth {
            workspace_id,
            ok: false,
            message: error,
            git_version: None,
            toplevel: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;

    #[tokio::test]
    async fn create_list_update_and_delete_local_workspace() {
        let pool = setup_migrated_pool().await;
        let dir = tempfile::tempdir().expect("dir");
        let created = create_workspace_with(
            &pool,
            CreateWorkspace {
                name: "local".to_string(),
                workspace_type: "local".to_string(),
                repo_path: Some(dir.path().to_string_lossy().into_owned()),
                ssh_config_id: None,
                remote_repo_path: None,
            },
        )
        .await
        .expect("create");
        assert_eq!(created.workspace_type, "local");
        let listed = list_workspaces_with(&pool).await.expect("list");
        assert_eq!(listed.len(), 1);
        let updated = update_workspace_with(
            &pool,
            &created.id,
            UpdateWorkspace {
                name: Some("renamed".to_string()),
                workspace_type: None,
                repo_path: None,
                ssh_config_id: None,
                remote_repo_path: None,
            },
        )
        .await
        .expect("update");
        assert_eq!(updated.name, "renamed");
        delete_workspace_row(&pool, &created.id)
            .await
            .expect("delete");
        assert!(list_workspaces_with(&pool).await.expect("empty").is_empty());
    }

    #[tokio::test]
    async fn local_workspace_requires_existing_dir() {
        let pool = setup_migrated_pool().await;
        let err = create_workspace_with(
            &pool,
            CreateWorkspace {
                name: "bad".to_string(),
                workspace_type: "local".to_string(),
                repo_path: Some("/definitely/missing/noxcode-ws".to_string()),
                ssh_config_id: None,
                remote_repo_path: None,
            },
        )
        .await
        .expect_err("missing dir");
        assert!(err.contains("不是目录") || err.contains("不存在"));
    }

    #[tokio::test]
    async fn scratch_workspace_is_idempotent() {
        let pool = setup_migrated_pool().await;
        let dir = tempfile::tempdir().expect("dir");
        let first = ensure_scratch_workspace_with(&pool, dir.path())
            .await
            .expect("first");
        let second = ensure_scratch_workspace_with(&pool, dir.path())
            .await
            .expect("second");
        assert_eq!(first.id, second.id);
        assert_eq!(first.name, SCRATCH_WORKSPACE_NAME);
        assert_eq!(first.workspace_type, WORKSPACE_TYPE_LOCAL);
        assert_eq!(list_workspaces_with(&pool).await.expect("list").len(), 1);
    }
}
