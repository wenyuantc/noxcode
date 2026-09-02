use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

use crate::app::shared::sqlite_pool;
use crate::db::models::{AgentSessionEvent, AgentSessionRecord, AgentSessionResumeInfo};
use crate::git::{delete_checkpoints_for_session, resolve_git_target};
use crate::native::manager::NativeAgentManager;
use crate::native::transcript::has_transcript;

pub(crate) async fn list_agent_sessions_with(
    pool: &SqlitePool,
    workspace_id: Option<&str>,
    profile_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<AgentSessionRecord>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, AgentSessionRecord>(
        r#"
        SELECT * FROM agent_sessions
        WHERE ($1 IS NULL OR workspace_id = $1)
          AND ($2 IS NULL OR profile_id = $2)
        ORDER BY started_at DESC
        LIMIT $3
        "#,
    )
    .bind(workspace_id)
    .bind(profile_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取会话列表失败: {error}"))?;
    Ok(rows)
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
            SELECT * FROM agent_session_events
            WHERE session_id = $1
            ORDER BY created_at ASC
            LIMIT $2
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
    let resumable = has_transcript(pool, session_id).await?;
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
        message: if resumable {
            "可以续聊".to_string()
        } else {
            "没有可恢复的上下文".to_string()
        },
    })
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
    profile_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AgentSessionRecord>, String> {
    let pool = sqlite_pool(&app).await?;
    list_agent_sessions_with(&pool, workspace_id.as_deref(), profile_id.as_deref(), limit).await
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
pub async fn delete_agent_session<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_id: String,
) -> Result<(), String> {
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
    delete_agent_session_row(&pool, &session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shared::{new_id, now_sqlite};
    use crate::db::test_support::setup_migrated_pool;

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
        let listed = list_agent_sessions_with(&pool, Some("ws-1"), None, Some(10))
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
        assert!(list_agent_sessions_with(&pool, Some("ws-1"), None, None)
            .await
            .expect("empty")
            .is_empty());
        let _ = new_id();
    }
}
