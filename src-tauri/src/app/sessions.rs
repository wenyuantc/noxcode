use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

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
) -> Result<Vec<AgentSessionRecord>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, AgentSessionRecord>(
        r#"
        SELECT * FROM agent_sessions
        WHERE ($1 IS NULL OR workspace_id = $1)
        ORDER BY pinned DESC, started_at DESC
        LIMIT $2
        "#,
    )
    .bind(workspace_id)
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

pub(crate) async fn set_agent_session_pinned_with(
    pool: &SqlitePool,
    session_id: &str,
    pinned: bool,
) -> Result<(), String> {
    let result = sqlx::query("UPDATE agent_sessions SET pinned = $1 WHERE id = $2")
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
) -> Result<Vec<AgentSessionRecord>, String> {
    let pool = sqlite_pool(&app).await?;
    list_agent_sessions_with(&pool, workspace_id.as_deref(), limit).await
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
        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10))
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
        assert!(list_agent_sessions_with(&pool, Some("ws-1"), None)
            .await
            .expect("empty")
            .is_empty());
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

        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10))
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
        let pinned_first = list_agent_sessions_with(&pool, Some("ws-1"), Some(10))
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
        let unpinned = list_agent_sessions_with(&pool, Some("ws-1"), Some(10))
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

        let listed = list_agent_sessions_with(&pool, Some("ws-1"), Some(10))
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
