use sqlx::Row;

use crate::app::shared::new_id;
use crate::native::media_error::{MediaError, ATTACHMENT_DELETING, IMPORT_INVALID, NOT_FOUND};
use crate::native::model::types::AttachmentUse;

pub(crate) async fn replace_owner_uses(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_type: &str,
    owner_id: &str,
    uses: &[AttachmentUse],
    now: &str,
) -> Result<(), MediaError> {
    if !matches!(owner_type, "message" | "input" | "tool_run" | "event") {
        return Err(MediaError::new(IMPORT_INVALID, "引用所有者无效"));
    }
    for use_value in uses {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM native_attachments WHERE id = $1")
                .bind(use_value.attachment_id())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
        match status.as_deref() {
            Some("ready") => {}
            Some("deleting") => {
                return Err(MediaError::new(ATTACHMENT_DELETING, "附件正在回收"));
            }
            _ => return Err(MediaError::new(NOT_FOUND, "附件不可引用")),
        }
    }
    let previous: Vec<String> = sqlx::query_scalar(
        "SELECT attachment_id FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2",
    )
    .bind(owner_type)
    .bind(owner_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
    sqlx::query("DELETE FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2")
        .bind(owner_type)
        .bind(owner_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
    for (position, use_value) in uses.iter().enumerate() {
        let use_json = serde_json::to_string(use_value)
            .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO native_attachment_refs (
                id, attachment_id, owner_type, owner_id, position, use_json, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(new_id())
        .bind(use_value.attachment_id())
        .bind(owner_type)
        .bind(owner_id)
        .bind(position as i64)
        .bind(use_json)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
        sqlx::query(
            "UPDATE native_attachments SET unreferenced_at = NULL, updated_at = $2 WHERE id = $1",
        )
        .bind(use_value.attachment_id())
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
    }
    for id in previous {
        if uses.iter().any(|item| item.attachment_id() == id) {
            continue;
        }
        let refs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM native_attachment_refs WHERE attachment_id = $1",
        )
        .bind(&id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
        let leases: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM native_attachment_leases WHERE attachment_id = $1",
        )
        .bind(&id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
        if refs == 0 && leases == 0 {
            sqlx::query(
                "UPDATE native_attachments SET unreferenced_at = COALESCE(unreferenced_at, $2), updated_at = $2 WHERE id = $1",
            )
            .bind(&id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
        }
    }
    Ok(())
}

pub(crate) async fn load_owner_uses_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_type: &str,
    owner_id: &str,
) -> Result<Vec<AttachmentUse>, MediaError> {
    let rows = sqlx::query(
        r#"
        SELECT use_json FROM native_attachment_refs
        WHERE owner_type = $1 AND owner_id = $2
        ORDER BY position
        "#,
    )
    .bind(owner_type)
    .bind(owner_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let json: String = row.get("use_json");
            serde_json::from_str(&json)
                .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))
        })
        .collect()
}

pub(crate) async fn load_owner_uses(
    pool: &sqlx::SqlitePool,
    owner_type: &str,
    owner_id: &str,
) -> Result<Vec<AttachmentUse>, MediaError> {
    let rows = sqlx::query(
        r#"
        SELECT use_json FROM native_attachment_refs
        WHERE owner_type = $1 AND owner_id = $2
        ORDER BY position
        "#,
    )
    .bind(owner_type)
    .bind(owner_id)
    .fetch_all(pool)
    .await
    .map_err(|error| MediaError::new(NOT_FOUND, error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let json: String = row.get("use_json");
            serde_json::from_str(&json)
                .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))
        })
        .collect()
}
