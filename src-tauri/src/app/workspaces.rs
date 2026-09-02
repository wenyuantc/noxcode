use sqlx::SqlitePool;

use crate::db::models::Workspace;

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
