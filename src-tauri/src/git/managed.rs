//! 托管隔离工作树的列表、删除与自动清理。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::Mutex;

use super::checkpoint::create_checkpoint;
use super::worktree::{
    default_local_worktree_root, is_any_managed_worktree_path, is_managed_worktree_path_with_root,
    looks_like_session_id, normalize_path_for_compare, remove_worktree,
    resolve_local_worktree_root, session_id_from_worktree_path,
};
use super::{record_git_activity, resolve_git_target, GitTarget};
use crate::db::models::AgentSessionRecord;
use crate::native::manager::NativeAgentManager;
use crate::native::settings::load_native_settings;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedWorktreeItem {
    pub session_id: String,
    pub title: String,
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    pub status: String,
    pub path: String,
    pub exists: bool,
    pub remote: bool,
    pub in_use: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedWorktreeList {
    pub root: String,
    pub default_root: String,
    pub items: Vec<ManagedWorktreeItem>,
}

pub fn configured_root_opt(root: &str) -> Option<&str> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub fn is_remote_placeholder_path(path: &str) -> bool {
    let trimmed = path.trim();
    trimmed.starts_with("$HOME/") || trimmed.starts_with("$HOME\\")
}

pub fn local_path_exists(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && !is_remote_placeholder_path(trimmed) && Path::new(trimmed).exists()
}

pub fn select_prune_candidates(
    items: &[ManagedWorktreeItem],
    keep_session_id: Option<&str>,
    limit: i32,
) -> Vec<String> {
    let keep = keep_session_id
        .map(str::trim)
        .filter(|item| !item.is_empty());
    let present: Vec<&ManagedWorktreeItem> = items
        .iter()
        .filter(|item| item.exists || item.remote)
        .collect();
    let limit = limit.max(0) as usize;
    if present.len() <= limit {
        return Vec::new();
    }
    let excess = present.len() - limit;
    let mut removable: Vec<&ManagedWorktreeItem> = present
        .into_iter()
        .filter(|item| !item.in_use && keep != Some(item.session_id.as_str()))
        .collect();
    removable.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    removable
        .into_iter()
        .take(excess)
        .map(|item| item.path.clone())
        .collect()
}

fn unique_scan_roots(primary: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for path in std::iter::once(primary.to_path_buf()).chain(extra.iter().cloned()) {
        let key = normalize_path_for_compare(&path.to_string_lossy());
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        roots.push(path);
    }
    roots
}

fn scan_local_root(root: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
            continue;
        };
        if !looks_like_session_id(name) {
            continue;
        }
        found.push((name.to_string(), path.to_string_lossy().into_owned()));
    }
    found
}

fn scan_local_roots(roots: &[PathBuf]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        for (id, path) in scan_local_root(root) {
            let key = normalize_path_for_compare(&path);
            if seen.insert(key) {
                found.push((id, path));
            }
        }
    }
    found
}

fn dir_stamp(path: &Path) -> String {
    path.metadata()
        .and_then(|meta| meta.modified().or_else(|_| meta.created()))
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Utc>::from(time)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

fn item_from_session(
    session: &AgentSessionRecord,
    path: &str,
    workspace_name: Option<String>,
    in_use: bool,
) -> ManagedWorktreeItem {
    let remote = is_remote_placeholder_path(path);
    ManagedWorktreeItem {
        session_id: session.id.clone(),
        title: session
            .title
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .unwrap_or("会话")
            .to_string(),
        workspace_id: session.workspace_id.clone(),
        workspace_name,
        status: session.status.clone(),
        path: path.to_string(),
        exists: remote || local_path_exists(path),
        remote,
        in_use,
        created_at: session.created_at.clone(),
    }
}

async fn load_workspace_names(pool: &SqlitePool) -> HashMap<String, String> {
    sqlx::query_as::<_, (String, String)>("SELECT id, name FROM workspaces")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect()
}

pub fn default_and_effective_roots<R: Runtime>(
    app: &AppHandle<R>,
    configured_root: &str,
) -> Result<(String, String), String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let default_root = default_local_worktree_root(&config_dir)
        .to_string_lossy()
        .into_owned();
    let root = resolve_local_worktree_root(&config_dir, configured_root)
        .to_string_lossy()
        .into_owned();
    Ok((root, default_root))
}

fn live_ids_for_sessions(
    manager: &NativeAgentManager,
    session_ids: impl IntoIterator<Item = String>,
) -> HashSet<String> {
    session_ids
        .into_iter()
        .filter(|id| manager.get_session(id).is_some())
        .collect()
}

pub async fn list_managed_worktrees_for<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    state: &Arc<Mutex<NativeAgentManager>>,
) -> Result<ManagedWorktreeList, String> {
    let settings = load_native_settings(app)?;
    let configured_opt = configured_root_opt(&settings.worktree_root);
    let (root, default_root) = default_and_effective_roots(app, &settings.worktree_root)?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let scan_roots = unique_scan_roots(
        Path::new(&root),
        &[PathBuf::from(&default_root), config_dir.join("worktrees")],
    );
    let names = load_workspace_names(pool).await;
    let sessions = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE working_dir IS NOT NULL AND TRIM(working_dir) != ''",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?;

    let candidate_ids: Vec<String> = {
        let mut ids: HashSet<String> = sessions.iter().map(|item| item.id.clone()).collect();
        for (session_id, _) in scan_local_roots(&scan_roots) {
            ids.insert(session_id);
        }
        ids.into_iter().collect()
    };
    let live_ids = live_ids_for_sessions(&*state.lock().await, candidate_ids);

    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut seen_ids = HashSet::new();
    for session in &sessions {
        let Some(path) = session
            .working_dir
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        else {
            continue;
        };
        if !is_managed_worktree_path_with_root(path, &session.id, configured_opt) {
            continue;
        }
        let key = normalize_path_for_compare(path);
        seen_paths.insert(key);
        seen_ids.insert(session.id.clone());
        let workspace_name = session
            .workspace_id
            .as_ref()
            .and_then(|id| names.get(id).cloned());
        items.push(item_from_session(
            session,
            path,
            workspace_name,
            live_ids.contains(&session.id),
        ));
    }

    for (session_id, path) in scan_local_roots(&scan_roots) {
        let key = normalize_path_for_compare(&path);
        if seen_paths.contains(&key) {
            continue;
        }
        if !is_any_managed_worktree_path(&path, configured_opt) {
            continue;
        }
        seen_paths.insert(key);
        if seen_ids.contains(&session_id) {
            continue;
        }
        seen_ids.insert(session_id.clone());
        if let Some(session) = sqlx::query_as::<_, AgentSessionRecord>(
            "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
        )
        .bind(&session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        {
            let workspace_name = session
                .workspace_id
                .as_ref()
                .and_then(|id| names.get(id).cloned());
            items.push(item_from_session(
                &session,
                &path,
                workspace_name,
                live_ids.contains(&session_id),
            ));
        } else {
            let created_at = dir_stamp(Path::new(&path));
            items.push(ManagedWorktreeItem {
                session_id,
                title: "未关联会话".to_string(),
                workspace_id: None,
                workspace_name: None,
                status: String::new(),
                path,
                exists: true,
                remote: false,
                in_use: false,
                created_at,
            });
        }
    }

    items.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(ManagedWorktreeList {
        root,
        default_root,
        items,
    })
}

async fn clear_session_working_dir(pool: &SqlitePool, session_id: &str) -> Result<(), String> {
    sqlx::query("UPDATE agent_sessions SET working_dir = NULL WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| format!("清空会话工作树目录失败: {error}"))?;
    Ok(())
}

fn remove_local_directory(path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() || is_remote_placeholder_path(trimmed) {
        return;
    }
    let path = PathBuf::from(trimmed);
    if path.exists() {
        let _ = std::fs::remove_dir_all(&path);
    }
}

pub async fn remove_managed_worktree_path<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    state: &Arc<Mutex<NativeAgentManager>>,
    path: &str,
) -> Result<(), String> {
    let settings = load_native_settings(app)?;
    let configured_opt = configured_root_opt(&settings.worktree_root);
    let path = path.trim();
    if path.is_empty() {
        return Err("工作树路径不能为空".to_string());
    }
    if !is_any_managed_worktree_path(path, configured_opt) {
        return Err("只能删除 noxcode 托管的隔离工作树".to_string());
    }
    let session_id =
        session_id_from_worktree_path(path).ok_or_else(|| "无法从路径解析会话".to_string())?;
    if state.lock().await.get_session(&session_id).is_some() {
        return Err("会话仍在运行，无法删除工作树".to_string());
    }

    let session = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(&session_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?;

    if let Some(workspace_id) = session
        .as_ref()
        .and_then(|item| item.workspace_id.as_deref())
    {
        if let Ok(target) = resolve_git_target(app, workspace_id).await {
            if local_path_exists(path) || is_remote_placeholder_path(path) {
                if let Ok(worktree) = super::resolve_git_target_at(app, workspace_id, path).await {
                    if let Err(error) = create_checkpoint(
                        pool,
                        &worktree,
                        workspace_id,
                        &session_id,
                        Some("清理工作树前"),
                        Some("manual"),
                    )
                    .await
                    {
                        eprintln!("[git] 删除工作树前打点失败: {error}");
                    }
                }
            }
            if let Err(error) = remove_worktree(&target, path).await {
                eprintln!("[git] worktree remove 失败，尝试直接删除目录: {error}");
                remove_local_directory(path);
            }
        } else {
            remove_local_directory(path);
        }
        record_git_activity(
            pool,
            "git_worktree_removed",
            workspace_id,
            Some(&session_id),
            "已删除托管工作树",
            serde_json::json!({ "path": path }),
        )
        .await;
    } else if local_path_exists(path) {
        let target = GitTarget::Local(PathBuf::from(path));
        if let Err(error) = remove_worktree(&target, path).await {
            eprintln!("[git] 无工作区时 worktree remove 失败: {error}");
        }
        remove_local_directory(path);
    }

    if session.is_some() {
        clear_session_working_dir(pool, &session_id).await?;
    }
    Ok(())
}

pub async fn prune_old_managed_worktrees<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    state: &Arc<Mutex<NativeAgentManager>>,
    keep_session_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let settings = load_native_settings(app)?;
    if !settings.worktree_auto_prune {
        return Ok(Vec::new());
    }
    let listed = list_managed_worktrees_for(app, pool, state).await?;
    let targets = select_prune_candidates(
        &listed.items,
        keep_session_id,
        settings.worktree_auto_prune_limit,
    );
    let mut removed = Vec::new();
    for path in targets {
        match remove_managed_worktree_path(app, pool, state, &path).await {
            Ok(()) => removed.push(path),
            Err(error) => eprintln!("[git] 自动清理工作树失败 {path}: {error}"),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, created: &str, in_use: bool) -> ManagedWorktreeItem {
        ManagedWorktreeItem {
            session_id: id.to_string(),
            title: id.to_string(),
            workspace_id: None,
            workspace_name: None,
            status: "idle".to_string(),
            path: format!("/data/nox-wt/{id}"),
            exists: true,
            remote: false,
            in_use,
            created_at: created.to_string(),
        }
    }

    #[test]
    fn prune_keeps_newest_and_skips_in_use() {
        let items = vec![
            item("old", "2026-01-01", false),
            item("mid", "2026-02-01", false),
            item("new", "2026-03-01", false),
            item("live", "2026-01-15", true),
        ];
        let removed = select_prune_candidates(&items, Some("new"), 2);
        assert_eq!(
            removed,
            vec![
                "/data/nox-wt/old".to_string(),
                "/data/nox-wt/mid".to_string()
            ]
        );
    }

    #[test]
    fn prune_does_nothing_under_limit() {
        let items = vec![
            item("a", "2026-01-01", false),
            item("b", "2026-02-01", false),
        ];
        assert!(select_prune_candidates(&items, None, 2).is_empty());
    }

    #[test]
    fn unique_scan_roots_keeps_legacy_appconfig() {
        let primary = PathBuf::from("/home/u/.noxcode/worktrees");
        let roots = unique_scan_roots(
            &primary,
            &[
                PathBuf::from("/home/u/.noxcode/worktrees/"),
                PathBuf::from("/cfg/worktrees"),
            ],
        );
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/home/u/.noxcode/worktrees"),
                PathBuf::from("/cfg/worktrees"),
            ]
        );
    }
}
