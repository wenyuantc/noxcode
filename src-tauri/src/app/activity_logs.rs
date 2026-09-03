use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Runtime};

use crate::app::shared::{new_id, now_sqlite, sqlite_pool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ActivityLog {
    pub id: String,
    pub kind: String,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub summary: String,
    pub payload_json: Option<String>,
    pub created_at: String,
}

pub(crate) async fn insert_activity_log(
    pool: &SqlitePool,
    kind: &str,
    workspace_id: Option<&str>,
    session_id: Option<&str>,
    summary: &str,
    payload: Value,
) -> Result<ActivityLog, String> {
    let record = ActivityLog {
        id: new_id(),
        kind: kind.to_string(),
        workspace_id: workspace_id.map(ToOwned::to_owned),
        session_id: session_id.map(ToOwned::to_owned),
        summary: summary.to_string(),
        payload_json: Some(
            serde_json::to_string(&payload)
                .map_err(|error| format!("序列化活动日志失败: {error}"))?,
        ),
        created_at: now_sqlite(),
    };
    sqlx::query(
        r#"
        INSERT INTO activity_logs (
            id, kind, workspace_id, session_id, summary, payload_json, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(&record.id)
    .bind(&record.kind)
    .bind(&record.workspace_id)
    .bind(&record.session_id)
    .bind(&record.summary)
    .bind(&record.payload_json)
    .bind(&record.created_at)
    .execute(pool)
    .await
    .map_err(|error| format!("写入活动日志失败: {error}"))?;
    Ok(record)
}

async fn list_activity_logs_with_pool(
    pool: &SqlitePool,
    workspace_id: Option<&str>,
    limit: i64,
) -> Result<Vec<ActivityLog>, String> {
    let limit = limit.clamp(1, 200);
    if let Some(workspace_id) = workspace_id {
        sqlx::query_as::<_, ActivityLog>(
            "SELECT * FROM activity_logs WHERE workspace_id = $1 ORDER BY created_at DESC, rowid DESC LIMIT $2",
        )
        .bind(workspace_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ActivityLog>(
            "SELECT * FROM activity_logs ORDER BY created_at DESC, rowid DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
    }
    .map_err(|error| format!("读取活动日志失败: {error}"))
}

#[tauri::command]
pub async fn list_activity_logs<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<ActivityLog>, String> {
    let pool = sqlite_pool(&app).await?;
    list_activity_logs_with_pool(&pool, workspace_id.as_deref(), limit.unwrap_or(50)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;

    #[tokio::test]
    async fn inserts_filters_and_limits_activity_logs() {
        let pool = setup_migrated_pool().await;
        insert_activity_log(
            &pool,
            "git_checkpoint_restore",
            Some("ws-1"),
            Some("session-1"),
            "回滚成功",
            serde_json::json!({"restored": 2}),
        )
        .await
        .expect("insert first");
        insert_activity_log(
            &pool,
            "git_checkpoints_cleared",
            Some("ws-2"),
            None,
            "已清除",
            serde_json::json!({"count": 3}),
        )
        .await
        .expect("insert second");

        let ws1 = list_activity_logs_with_pool(&pool, Some("ws-1"), 50)
            .await
            .expect("list ws-1");
        assert_eq!(ws1.len(), 1);
        assert_eq!(ws1[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(ws1[0].kind, "git_checkpoint_restore");

        let limited = list_activity_logs_with_pool(&pool, None, 1)
            .await
            .expect("list limited");
        assert_eq!(limited.len(), 1);
    }
}
