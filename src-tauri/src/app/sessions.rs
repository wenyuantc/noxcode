use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::app::shared::sqlite_pool;
use crate::db::models::{
    AgentSessionEvent, AgentSessionRecord, AgentSessionResumeInfo, NativeContextUsage,
};
use crate::git::{delete_checkpoints_for_session, resolve_git_target};
use crate::native::manager::NativeAgentManager;
use crate::native::transcript::has_transcript;

pub(crate) async fn list_agent_sessions_with(
    pool: &SqlitePool,
    workspace_id: Option<&str>,
    limit: Option<i64>,
    archived: Option<bool>,
    offset: Option<i64>,
) -> Result<Vec<AgentSessionRecord>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, AgentSessionRecord>(
        r#"
        SELECT * FROM agent_sessions
        WHERE ($1 IS NULL OR workspace_id = $1) AND archived = $3
        ORDER BY pinned DESC, started_at DESC, id ASC
        LIMIT $2 OFFSET $4
        "#,
    )
    .bind(workspace_id)
    .bind(limit)
    .bind(i32::from(archived.unwrap_or(false)))
    .bind(offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取会话列表失败: {error}"))?;
    Ok(rows)
}

pub(crate) async fn lock_agent_session_operation(
    manager: &Mutex<NativeAgentManager>,
    session_id: &str,
) -> OwnedMutexGuard<()> {
    let lock = manager.lock().await.session_operation_lock(session_id);
    lock.lock_owned().await
}

pub(crate) async fn require_unarchived_session_with(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<(), String> {
    let archived: i32 = sqlx::query_scalar("SELECT archived FROM agent_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取会话失败: {error}"))?
        .ok_or_else(|| format!("会话不存在: {session_id}"))?;
    if archived != 0 {
        return Err("会话已归档，请先取消归档任务".to_string());
    }
    Ok(())
}

async fn fetch_agent_session_with(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<AgentSessionRecord, String> {
    sqlx::query_as("SELECT * FROM agent_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取会话失败: {error}"))?
        .ok_or_else(|| format!("会话不存在: {session_id}"))
}

pub(crate) async fn rename_agent_session_with(
    pool: &SqlitePool,
    session_id: &str,
    title: &str,
) -> Result<AgentSessionRecord, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("任务名称不能为空".to_string());
    }
    sqlx::query("UPDATE agent_sessions SET title = $1 WHERE id = $2")
        .bind(title)
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| format!("重命名任务失败: {error}"))?;
    fetch_agent_session_with(pool, session_id).await
}

pub(crate) async fn set_agent_session_archived_with(
    pool: &SqlitePool,
    manager: &Mutex<NativeAgentManager>,
    session_id: &str,
    archived: bool,
) -> Result<AgentSessionRecord, String> {
    let _operation = lock_agent_session_operation(manager, session_id).await;
    let manager = manager.lock().await;
    if archived && manager.session_is_busy(session_id) {
        return Err("任务仍在工作或等待处理，请先停止当前回合或等待结束".to_string());
    }
    sqlx::query("UPDATE agent_sessions SET archived = $1 WHERE id = $2")
        .bind(i32::from(archived))
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| format!("更新任务归档状态失败: {error}"))?;
    fetch_agent_session_with(pool, session_id).await
}

async fn local_agent_session_directory_with(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<std::path::PathBuf, String> {
    let session = fetch_agent_session_with(pool, session_id).await?;
    if session.execution_target != crate::app::shared::EXECUTION_TARGET_LOCAL {
        return Err("远程任务目录无法在本机文件管理器中打开".to_string());
    }
    let path = match session.working_dir.filter(|path| !path.trim().is_empty()) {
        Some(path) => Some(path),
        None => sqlx::query_scalar::<_, Option<String>>(
            "SELECT repo_path FROM workspaces WHERE id = $1 AND workspace_type = 'local'",
        )
        .bind(session.workspace_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取工作区目录失败: {error}"))?
        .flatten(),
    }
    .filter(|path| !path.trim().is_empty())
    .ok_or_else(|| "任务没有可用的项目目录".to_string())?;
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!("项目目录不存在或不可访问: {}", path.display()));
    }
    Ok(path)
}

pub(crate) async fn get_agent_session_log_lines_with(
    pool: &SqlitePool,
    session_id: &str,
    after_event_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<AgentSessionEvent>, String> {
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let rows = if let Some(after) = after_event_id
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        sqlx::query_as::<_, AgentSessionEvent>(
            r#"
            SELECT * FROM agent_session_events
            WHERE session_id = $1
              AND created_at > COALESCE((SELECT created_at FROM agent_session_events WHERE id = $2), '')
            ORDER BY created_at ASC
            LIMIT $3
            "#,
        )
        .bind(session_id)
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, AgentSessionEvent>(
            r#"
            SELECT * FROM (
                SELECT * FROM agent_session_events
                WHERE session_id = $1
                ORDER BY created_at DESC
                LIMIT $2
            ) AS recent
            ORDER BY created_at ASC
            "#,
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    };
    rows.map_err(|error| format!("读取会话日志失败: {error}"))
}

pub(crate) async fn prepare_agent_session_resume_with(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<AgentSessionResumeInfo, String> {
    let session = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_id}"))?;
    let resumable = session.archived == 0 && has_transcript(pool, session_id).await?;
    let meta = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        "SELECT model, turns FROM native_session_transcripts WHERE session_record_id = $1 AND deleted_at IS NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    Ok(AgentSessionResumeInfo {
        session_id: session.id,
        resumable,
        model: meta.as_ref().and_then(|item| item.0.clone()),
        turns: meta.as_ref().and_then(|item| item.1),
        message: if session.archived != 0 {
            "会话已归档，请先取消归档任务".to_string()
        } else if resumable {
            "可以续聊".to_string()
        } else {
            "没有可恢复的上下文".to_string()
        },
    })
}

pub(crate) async fn set_agent_session_pinned_with(
    pool: &SqlitePool,
    session_id: &str,
    pinned: bool,
) -> Result<(), String> {
    require_unarchived_session_with(pool, session_id).await?;
    let result =
        sqlx::query("UPDATE agent_sessions SET pinned = $1 WHERE id = $2 AND archived = 0")
            .bind(if pinned { 1i32 } else { 0 })
            .bind(session_id)
            .execute(pool)
            .await
            .map_err(|error| format!("更新会话置顶失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err(format!("会话不存在: {session_id}"));
    }
    Ok(())
}

pub(crate) async fn persist_context_usage_with(
    pool: &SqlitePool,
    usage: &NativeContextUsage,
) -> Result<(), String> {
    let json =
        serde_json::to_string(usage).map_err(|error| format!("序列化上下文用量失败: {error}"))?;
    sqlx::query("UPDATE agent_sessions SET context_usage_json = $1 WHERE id = $2")
        .bind(&json)
        .bind(&usage.session_record_id)
        .execute(pool)
        .await
        .map_err(|error| format!("更新会话上下文用量失败: {error}"))?;
    Ok(())
}

pub(crate) async fn delete_agent_session_row(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<(), String> {
    let result = sqlx::query("DELETE FROM agent_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除会话失败: {error}"))?;
    if result.rows_affected() == 0 {
        return Err(format!("会话不存在: {session_id}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_agent_sessions<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    limit: Option<i64>,
    archived: Option<bool>,
    offset: Option<i64>,
) -> Result<Vec<AgentSessionRecord>, String> {
    let pool = sqlite_pool(&app).await?;
    list_agent_sessions_with(&pool, workspace_id.as_deref(), limit, archived, offset).await
}

#[tauri::command]
pub async fn rename_agent_session<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    title: String,
) -> Result<AgentSessionRecord, String> {
    rename_agent_session_with(&sqlite_pool(&app).await?, &session_id, &title).await
}

#[tauri::command]
pub async fn set_agent_session_archived<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_id: String,
    archived: bool,
) -> Result<AgentSessionRecord, String> {
    set_agent_session_archived_with(&sqlite_pool(&app).await?, &state, &session_id, archived).await
}

#[tauri::command]
pub async fn open_agent_session_directory<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> Result<(), String> {
    let path = local_agent_session_directory_with(&sqlite_pool(&app).await?, &session_id).await?;
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| format!("打开项目目录失败: {error}"))
}

#[tauri::command]
pub async fn get_agent_session_log_lines<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    after_event_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AgentSessionEvent>, String> {
    let pool = sqlite_pool(&app).await?;
    get_agent_session_log_lines_with(&pool, &session_id, after_event_id.as_deref(), limit).await
}

#[tauri::command]
pub async fn prepare_agent_session_resume<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> Result<AgentSessionResumeInfo, String> {
    let pool = sqlite_pool(&app).await?;
    prepare_agent_session_resume_with(&pool, &session_id).await
}

#[tauri::command]
pub async fn set_agent_session_pinned<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    pinned: bool,
) -> Result<(), String> {
    let pool = sqlite_pool(&app).await?;
    set_agent_session_pinned_with(&pool, &session_id, pinned).await
}

#[tauri::command]
pub async fn delete_agent_session<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_id: String,
) -> Result<(), String> {
    let _operation = lock_agent_session_operation(&state, &session_id).await;
    if state.lock().await.get_session(&session_id).is_some() {
        return Err("会话仍在运行，无法删除".to_string());
    }
    let pool = sqlite_pool(&app).await?;
    let session = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(&session_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_id}"))?;
    if let Some(workspace_id) = session.workspace_id.as_deref() {
        if let Ok(target) = resolve_git_target(&app, workspace_id).await {
            if let Err(error) = delete_checkpoints_for_session(&pool, &target, &session_id).await {
                eprintln!("[git] 清理会话 checkpoint 失败: {error}");
            }
        }
    }
    if let Ok(config_dir) = app.path().app_config_dir() {
        if let Err(error) =
            crate::native::artifacts::delete_session_artifacts(&pool, &config_dir, &session_id)
                .await
        {
            eprintln!("[native] 清理会话 artifact 失败: {error}");
        }
    }
    delete_agent_session_row(&pool, &session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shared::{new_id, now_sqlite};
    use crate::db::test_support::setup_migrated_pool;

    async fn seed_session(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT OR IGNORE INTO workspaces (id, name, workspace_type, repo_path) VALUES ('ws-1', 'workspace', 'local', '/project')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO agent_sessions (id, workspace_id, title, pinned, status) VALUES ($1, 'ws-1', 'original', 1, 'exited')")
            .bind(id).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn renames_full_unicode_title_and_rejects_empty_or_missing() {
        let pool = setup_migrated_pool().await;
        seed_session(&pool, "rename").await;
        let title = "任务标题".repeat(30);
        let renamed = rename_agent_session_with(&pool, "rename", &format!("  {title}  "))
            .await
            .unwrap();
        assert_eq!(renamed.title.as_deref(), Some(title.as_str()));
        assert_eq!(renamed.pinned, 1);
        assert!(rename_agent_session_with(&pool, "rename", " \n ")
            .await
            .is_err());
        assert_eq!(
            fetch_agent_session_with(&pool, "rename")
                .await
                .unwrap()
                .title,
            Some(title)
        );
        assert!(rename_agent_session_with(&pool, "missing", "title")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn archive_retains_history_metadata_and_pin_and_can_be_restored() {
        let pool = setup_migrated_pool().await;
        let manager = Mutex::new(NativeAgentManager::new());
        seed_session(&pool, "archive").await;
        sqlx::query("INSERT INTO agent_session_events (id, session_id, event_type, message) VALUES ('event', 'archive', 'input', 'history')")
            .execute(&pool).await.unwrap();
        sqlx::query("UPDATE agent_sessions SET input_tokens = 123, context_usage_json = '{}' WHERE id = 'archive'")
            .execute(&pool).await.unwrap();
        let archived = set_agent_session_archived_with(&pool, &manager, "archive", true)
            .await
            .unwrap();
        assert_eq!(archived.archived, 1);
        assert_eq!(archived.pinned, 1);
        assert_eq!(archived.input_tokens, Some(123));
        assert_eq!(archived.context_usage_json.as_deref(), Some("{}"));
        assert!(list_agent_sessions_with(&pool, None, None, None, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            get_agent_session_log_lines_with(&pool, "archive", None, None)
                .await
                .unwrap()[0]
                .message
                .as_deref(),
            Some("history")
        );
        assert!(require_unarchived_session_with(&pool, "archive")
            .await
            .is_err());
        assert!(set_agent_session_pinned_with(&pool, "archive", false)
            .await
            .is_err());
        let resume = prepare_agent_session_resume_with(&pool, "archive")
            .await
            .unwrap();
        assert!(!resume.resumable);
        assert!(resume.message.contains("已归档"));
        let restored = set_agent_session_archived_with(&pool, &manager, "archive", false)
            .await
            .unwrap();
        assert_eq!(restored.archived, 0);
        assert_eq!(restored.pinned, 1);
        require_unarchived_session_with(&pool, "archive")
            .await
            .unwrap();
        assert_eq!(
            list_agent_sessions_with(&pool, None, None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            set_agent_session_archived_with(&pool, &manager, "missing", true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn archive_filter_precedes_limit_and_pagination_is_stable() {
        let pool = setup_migrated_pool().await;
        for index in 0..52 {
            seed_session(&pool, &format!("session-{index:02}")).await;
        }
        sqlx::query("UPDATE agent_sessions SET archived = 1 WHERE id != 'session-51'")
            .execute(&pool)
            .await
            .unwrap();
        let active = list_agent_sessions_with(&pool, Some("ws-1"), Some(1), None, None)
            .await
            .unwrap();
        assert_eq!(active[0].id, "session-51");
        let first = list_agent_sessions_with(&pool, None, Some(50), Some(true), Some(0))
            .await
            .unwrap();
        let next = list_agent_sessions_with(&pool, None, Some(50), Some(true), Some(50))
            .await
            .unwrap();
        assert_eq!(first.len(), 50);
        assert_eq!(next.len(), 1);
        assert!(!first.iter().any(|session| session.id == next[0].id));
        assert!(
            list_agent_sessions_with(&pool, Some("other"), None, Some(true), None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn archive_waits_for_inflight_session_operation_and_guard_rejects_later_input() {
        let pool = setup_migrated_pool().await;
        seed_session(&pool, "serialized").await;
        let manager = Arc::new(Mutex::new(NativeAgentManager::new()));
        let operation = lock_agent_session_operation(&manager, "serialized").await;
        let archive = {
            let pool = pool.clone();
            let manager = manager.clone();
            tokio::spawn(async move {
                set_agent_session_archived_with(&pool, &manager, "serialized", true).await
            })
        };
        tokio::task::yield_now().await;
        assert!(!archive.is_finished());
        drop(operation);
        archive.await.unwrap().unwrap();
        let _next = lock_agent_session_operation(&manager, "serialized").await;
        assert!(require_unarchived_session_with(&pool, "serialized")
            .await
            .unwrap_err()
            .contains("已归档"));
    }

    #[tokio::test]
    async fn resolves_session_directory_before_workspace_and_rejects_remote_or_missing() {
        let pool = setup_migrated_pool().await;
        seed_session(&pool, "directory").await;
        let workspace = tempfile::tempdir().unwrap();
        let session = tempfile::tempdir().unwrap();
        sqlx::query("UPDATE workspaces SET repo_path = $1 WHERE id = 'ws-1'")
            .bind(workspace.path().to_str().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            local_agent_session_directory_with(&pool, "directory")
                .await
                .unwrap(),
            workspace.path()
        );
        sqlx::query("UPDATE agent_sessions SET working_dir = $1 WHERE id = 'directory'")
            .bind(session.path().to_str().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            local_agent_session_directory_with(&pool, "directory")
                .await
                .unwrap(),
            session.path()
        );
        sqlx::query("UPDATE agent_sessions SET execution_target = 'ssh' WHERE id = 'directory'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(local_agent_session_directory_with(&pool, "directory")
            .await
            .unwrap_err()
            .contains("远程"));
        sqlx::query("UPDATE agent_sessions SET execution_target = 'local', working_dir = $1 WHERE id = 'directory'")
            .bind(session.path().join("missing").to_str().unwrap()).execute(&pool).await.unwrap();
        assert!(local_agent_session_directory_with(&pool, "directory")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn lists_and_deletes_session() {
        let pool = setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        let now = now_sqlite();
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, status, started_at, created_at) VALUES ('sess-1', 'ws-1', 'exited', $1, $1)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .expect("session");
        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10), None, None)
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        let resume = prepare_agent_session_resume_with(&pool, "sess-1")
            .await
            .expect("resume");
        assert!(!resume.resumable);
        delete_agent_session_row(&pool, "sess-1")
            .await
            .expect("delete");
        assert!(
            list_agent_sessions_with(&pool, Some("ws-1"), None, None, None)
                .await
                .expect("empty")
                .is_empty()
        );
        let _ = new_id();
    }

    #[tokio::test]
    async fn pins_session_and_lists_pinned_first() {
        let pool = setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        let older = "2026-01-01 00:00:00";
        let newer = "2026-02-01 00:00:00";
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, status, started_at, created_at) VALUES ('sess-old', 'ws-1', 'exited', $1, $1)",
        )
        .bind(older)
        .execute(&pool)
        .await
        .expect("old session");
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, status, started_at, created_at) VALUES ('sess-new', 'ws-1', 'exited', $1, $1)",
        )
        .bind(newer)
        .execute(&pool)
        .await
        .expect("new session");

        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10), None, None)
            .await
            .expect("list");
        assert_eq!(
            listed
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-new", "sess-old"]
        );
        assert_eq!(listed[0].pinned, 0);

        set_agent_session_pinned_with(&pool, "sess-old", true)
            .await
            .expect("pin");
        let pinned_first = list_agent_sessions_with(&pool, Some("ws-1"), Some(10), None, None)
            .await
            .expect("list pinned");
        assert_eq!(
            pinned_first
                .iter()
                .map(|session| (session.id.as_str(), session.pinned))
                .collect::<Vec<_>>(),
            vec![("sess-old", 1), ("sess-new", 0)]
        );

        set_agent_session_pinned_with(&pool, "sess-old", false)
            .await
            .expect("unpin");
        let unpinned = list_agent_sessions_with(&pool, Some("ws-1"), Some(10), None, None)
            .await
            .expect("list unpinned");
        assert_eq!(
            unpinned
                .iter()
                .map(|session| (session.id.as_str(), session.pinned))
                .collect::<Vec<_>>(),
            vec![("sess-new", 0), ("sess-old", 0)]
        );

        let missing = set_agent_session_pinned_with(&pool, "missing", true).await;
        assert!(missing
            .expect_err("missing session")
            .contains("会话不存在: missing"));
    }

    #[tokio::test]
    async fn persists_context_usage_json_for_list() {
        let pool = setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, status, started_at, created_at) VALUES ('sess-1', 'ws-1', 'exited', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .expect("session");

        persist_context_usage_with(
            &pool,
            &NativeContextUsage {
                session_record_id: "sess-1".to_string(),
                used_tokens: 28000,
                limit_tokens: 500000,
                generation: 1,
                compactions: 0,
                mcp_tokens: 10,
                system_tool_tokens: 20,
                skill_tokens: 30,
                system_prompt_tokens: 40,
                other_tokens: 50,
                message_tokens: 27850,
                prompt_tokens: 27000,
                cached_tokens: 22410,
            },
        )
        .await
        .expect("persist");

        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10), None, None)
            .await
            .expect("list");
        let json = listed[0]
            .context_usage_json
            .as_deref()
            .expect("context_usage_json");
        let stored: NativeContextUsage = serde_json::from_str(json).expect("parse");
        assert_eq!(stored.session_record_id, "sess-1");
        assert_eq!(stored.used_tokens, 28000);
        assert_eq!(stored.limit_tokens, 500000);
        assert_eq!(stored.cached_tokens, 22410);
    }

    #[tokio::test]
    async fn log_lines_return_latest_window_in_asc_order() {
        let pool = setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, status, started_at, created_at) VALUES ('sess-1', 'ws-1', 'exited', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .expect("session");
        for (id, created_at, message) in [
            ("evt-1", "2026-01-01 00:00:01", "oldest"),
            ("evt-2", "2026-01-01 00:00:02", "middle"),
            ("evt-3", "2026-01-01 00:00:03", "newest"),
        ] {
            sqlx::query(
                "INSERT INTO agent_session_events (id, session_id, event_type, message, created_at) VALUES ($1, 'sess-1', 'stdout', $2, $3)",
            )
            .bind(id)
            .bind(message)
            .bind(created_at)
            .execute(&pool)
            .await
            .expect("event");
        }

        let rows = get_agent_session_log_lines_with(&pool, "sess-1", None, Some(2))
            .await
            .expect("latest window");
        assert_eq!(
            rows.iter()
                .map(|event| (event.id.as_str(), event.message.as_deref()))
                .collect::<Vec<_>>(),
            vec![("evt-2", Some("middle")), ("evt-3", Some("newest"))]
        );
    }
}
