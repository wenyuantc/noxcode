//! 已接收的普通输入、queue 和 steer。草稿本身仍只活在进程内。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::app::shared::now_sqlite;
use crate::native::attachments::replace_owner_uses;
use crate::native::media_error::{
    MediaError, IDEMPOTENCY_CONFLICT, INPUT_NOT_EDITABLE, NOT_FOUND, REVISION_CONFLICT,
};
use crate::native::model::types::AttachmentUse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncomingInput {
    pub id: String,
    pub session_id: String,
    pub branch_id: Option<String>,
    pub turn_id: Option<String>,
    pub mode: String,
    pub text: String,
    pub attachments: Vec<AttachmentUse>,
    pub queue_position: Option<i64>,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InputReceipt {
    pub id: String,
    pub status: String,
    pub revision: i64,
    pub payload_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingInput {
    pub id: String,
    pub session_id: String,
    pub mode: String,
    pub text: String,
    pub status: String,
    pub revision: i64,
    pub queue_position: Option<i64>,
    pub attachments: Vec<AttachmentUse>,
    pub error_code: Option<String>,
}

pub(crate) fn payload_hash(input: &IncomingInput) -> String {
    let canonical = json!({
        "text": input.text,
        "mode": input.mode,
        "attachments": input.attachments,
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) async fn accept_input(
    pool: &SqlitePool,
    input: &IncomingInput,
) -> Result<InputReceipt, MediaError> {
    let hash = payload_hash(input);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    if let Some(existing) = load_row(&mut tx, &input.id).await? {
        if existing.receipt.payload_hash == hash {
            tx.commit().await.map_err(storage)?;
            return Ok(existing.receipt);
        }
        return Err(MediaError::new(
            IDEMPOTENCY_CONFLICT,
            "同一输入标识的内容不一致",
        ));
    }
    let now = now_sqlite();
    let prepare = json!({
        "instance_id": input.instance_id,
        "applied_edit_ids": [],
    });
    sqlx::query(
        r#"
        INSERT INTO native_input_submissions (
            id, session_record_id, branch_id, turn_id, mode, text, payload_hash, revision,
            status, queue_position, prepare_json, created_at, updated_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,1,'accepted',$8,$9,$10,$10)
        "#,
    )
    .bind(&input.id)
    .bind(&input.session_id)
    .bind(&input.branch_id)
    .bind(&input.turn_id)
    .bind(&input.mode)
    .bind(&input.text)
    .bind(&hash)
    .bind(input.queue_position)
    .bind(prepare.to_string())
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    replace_owner_uses(&mut tx, "input", &input.id, &input.attachments, &now).await?;
    if input.mode == "steer" {
        sqlx::query("UPDATE agent_sessions SET pending_plan_json = NULL WHERE id = $1")
            .bind(&input.session_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    tx.commit().await.map_err(storage)?;
    Ok(InputReceipt {
        id: input.id.clone(),
        status: "accepted".to_string(),
        revision: 1,
        payload_hash: hash,
    })
}

pub(crate) async fn edit_input(
    pool: &SqlitePool,
    input_id: &str,
    edit_id: &str,
    expected_revision: i64,
    text: Option<&str>,
    attachments: Option<&[AttachmentUse]>,
) -> Result<InputReceipt, MediaError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let row = load_row(&mut tx, input_id)
        .await?
        .ok_or_else(|| MediaError::new(NOT_FOUND, "输入不存在"))?;
    if row.receipt.status != "accepted" && row.receipt.status != "blocked" {
        return Err(MediaError::new(INPUT_NOT_EDITABLE, "输入已不能编辑"));
    }
    let mut prepare: Value =
        serde_json::from_str(&row.prepare_json).unwrap_or_else(|_| json!({"applied_edit_ids": []}));
    if prepare
        .get("applied_edit_ids")
        .and_then(Value::as_array)
        .is_some_and(|edits| edits.iter().any(|item| item.as_str() == Some(edit_id)))
    {
        tx.commit().await.map_err(storage)?;
        return Ok(row.receipt);
    }
    if row.receipt.revision != expected_revision {
        return Err(MediaError::new(REVISION_CONFLICT, "输入修订已变化"));
    }
    let now = now_sqlite();
    let next_text = text.unwrap_or(row.text.as_str());
    if let Some(attachments) = attachments {
        replace_owner_uses(&mut tx, "input", input_id, attachments, &now).await?;
    }
    if let Some(edits) = prepare
        .get_mut("applied_edit_ids")
        .and_then(Value::as_array_mut)
    {
        edits.push(Value::String(edit_id.to_string()));
    }
    let revision = row.receipt.revision + 1;
    sqlx::query(
        r#"
        UPDATE native_input_submissions
        SET text = $2, revision = $3, prepare_json = $4, updated_at = $5
        WHERE id = $1
        "#,
    )
    .bind(input_id)
    .bind(next_text)
    .bind(revision)
    .bind(prepare.to_string())
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(InputReceipt {
        id: input_id.to_string(),
        status: row.receipt.status,
        revision,
        payload_hash: row.receipt.payload_hash,
    })
}

pub(crate) async fn mark_applied(
    pool: &SqlitePool,
    input_id: &str,
    message_id: &str,
) -> Result<bool, MediaError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let row = load_row(&mut tx, input_id)
        .await?
        .ok_or_else(|| MediaError::new(NOT_FOUND, "输入不存在"))?;
    if row.receipt.status == "applied" {
        tx.commit().await.map_err(storage)?;
        return Ok(false);
    }
    sqlx::query(
        r#"
        UPDATE native_input_submissions
        SET status = 'applied', applied_message_id = $2, updated_at = $3
        WHERE id = $1 AND status != 'applied'
        "#,
    )
    .bind(input_id)
    .bind(message_id)
    .bind(now_sqlite())
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(true)
}

pub(crate) async fn set_input_status(
    pool: &SqlitePool,
    input_id: &str,
    status: &str,
    error_code: Option<&str>,
) -> Result<(), MediaError> {
    sqlx::query(
        "UPDATE native_input_submissions SET status = $2, error_code = $3, updated_at = $4 WHERE id = $1",
    )
    .bind(input_id)
    .bind(status)
    .bind(error_code)
    .bind(now_sqlite())
    .execute(pool)
    .await
    .map_err(storage)?;
    Ok(())
}

pub(crate) async fn interrupt_foreign_steers(
    pool: &SqlitePool,
    session_id: &str,
    instance_id: &str,
) -> Result<u64, MediaError> {
    let rows = sqlx::query(
        r#"
        SELECT id, prepare_json FROM native_input_submissions
        WHERE session_record_id = $1 AND mode = 'steer' AND status = 'accepted'
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(storage)?;
    let mut changed = 0u64;
    for row in rows {
        let prepare: Value = serde_json::from_str(row.get::<String, _>("prepare_json").as_str())
            .unwrap_or(Value::Null);
        let owner = prepare
            .get("instance_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if owner == instance_id {
            continue;
        }
        let id: String = row.get("id");
        set_input_status(pool, &id, "interrupted", Some("interrupted")).await?;
        changed += 1;
    }
    Ok(changed)
}

pub(crate) async fn pending_inputs(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<PendingInput>, MediaError> {
    let rows = sqlx::query(
        r#"
        SELECT id, session_record_id, mode, text, status, revision, queue_position, error_code
        FROM native_input_submissions
        WHERE session_record_id = $1 AND status IN ('accepted', 'blocked', 'interrupted')
        ORDER BY COALESCE(queue_position, 0), created_at
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(storage)?;
    let mut pending = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        pending.push(PendingInput {
            attachments: crate::native::attachments::load_owner_uses(pool, "input", &id).await?,
            id,
            session_id: row.get("session_record_id"),
            mode: row.get("mode"),
            text: row.get("text"),
            status: row.get("status"),
            revision: row.get("revision"),
            queue_position: row.get("queue_position"),
            error_code: row.get("error_code"),
        });
    }
    Ok(pending)
}

async fn load_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
) -> Result<Option<InputReceiptAndText>, MediaError> {
    let row = sqlx::query(
        "SELECT id, status, revision, payload_hash, text, prepare_json FROM native_input_submissions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?;
    Ok(row.map(|row| InputReceiptAndText {
        receipt: InputReceipt {
            id: row.get("id"),
            status: row.get("status"),
            revision: row.get("revision"),
            payload_hash: row.get("payload_hash"),
        },
        text: row.get("text"),
        prepare_json: row.get("prepare_json"),
    }))
}

struct InputReceiptAndText {
    receipt: InputReceipt,
    text: String,
    prepare_json: String,
}

fn storage(error: sqlx::Error) -> MediaError {
    MediaError::new(
        crate::native::media_error::STORAGE_FAILED,
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

    use super::*;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::attachments::AttachmentService;
    use crate::native::media_error::{IDEMPOTENCY_CONFLICT, INPUT_NOT_EDITABLE, REVISION_CONFLICT};
    use crate::native::media_limits::MAX_ORIGINAL_BYTES;
    use crate::native::model::types::{AttachmentUse, PdfUseMode};

    async fn image_id(dir: &std::path::Path, service: &AttachmentService, name: &str) -> String {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([1, 2, 3])));
        let mut cursor = Cursor::new(Vec::new());
        image.write_to(&mut cursor, ImageFormat::Png).unwrap();
        let path = dir.join(name);
        fs::write(&path, cursor.into_inner()).unwrap();
        service
            .import_path(&path, "draft", MAX_ORIGINAL_BYTES)
            .await
            .unwrap()
            .id
    }

    fn incoming(id: &str, text: &str, attachments: Vec<AttachmentUse>) -> IncomingInput {
        IncomingInput {
            id: id.to_string(),
            session_id: "sess".to_string(),
            branch_id: None,
            turn_id: Some("turn".to_string()),
            mode: "queue".to_string(),
            text: text.to_string(),
            attachments,
            queue_position: Some(1),
            instance_id: "instance-a".to_string(),
        }
    }

    #[tokio::test]
    async fn tc_in_001_same_payload_is_accepted_once() {
        let dir = tempfile::tempdir().unwrap();
        let pool = setup_migrated_pool().await;
        let service = AttachmentService::new(dir.path().to_path_buf(), pool.clone(), "instance-a");
        let first = image_id(dir.path(), &service, "a.png").await;
        let second = image_id(dir.path(), &service, "b.png").await;
        let uses = vec![
            AttachmentUse::Image {
                attachment_id: first.clone(),
            },
            AttachmentUse::Image {
                attachment_id: second.clone(),
            },
        ];
        let input = incoming("U", "hello", uses);
        let accepted = accept_input(&pool, &input).await.unwrap();
        let retried = accept_input(&pool, &input).await.unwrap();
        assert_eq!(accepted, retried);
        let pending = pending_inputs(&pool, "sess").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0]
                .attachments
                .iter()
                .map(AttachmentUse::attachment_id)
                .collect::<Vec<_>>(),
            vec![first.as_str(), second.as_str()]
        );
        assert!(mark_applied(&pool, "U", "msg-1").await.unwrap());
        assert!(!mark_applied(&pool, "U", "msg-1").await.unwrap());
    }

    #[tokio::test]
    async fn tc_in_002_changed_payload_conflicts_without_rereading_source() {
        let dir = tempfile::tempdir().unwrap();
        let pool = setup_migrated_pool().await;
        let service = AttachmentService::new(dir.path().to_path_buf(), pool.clone(), "instance-a");
        let first = image_id(dir.path(), &service, "a.png").await;
        let second = image_id(dir.path(), &service, "b.png").await;
        let original = incoming(
            "U",
            "hello",
            vec![
                AttachmentUse::Image {
                    attachment_id: first.clone(),
                },
                AttachmentUse::Image {
                    attachment_id: second.clone(),
                },
            ],
        );
        accept_input(&pool, &original).await.unwrap();
        fs::remove_file(dir.path().join("a.png")).unwrap();
        let mut changed = original.clone();
        changed.text = "edited".to_string();
        assert_eq!(
            accept_input(&pool, &changed).await.unwrap_err().code,
            IDEMPOTENCY_CONFLICT
        );
        changed = original.clone();
        changed.attachments.reverse();
        assert_eq!(
            accept_input(&pool, &changed).await.unwrap_err().code,
            IDEMPOTENCY_CONFLICT
        );
        changed.attachments = vec![AttachmentUse::Pdf {
            attachment_id: first.clone(),
            mode: PdfUseMode::Text,
            pages: Some(vec![1]),
        }];
        assert_eq!(
            accept_input(&pool, &changed).await.unwrap_err().code,
            IDEMPOTENCY_CONFLICT
        );
        let pending = pending_inputs(&pool, "sess").await.unwrap();
        assert_eq!(pending[0].text, "hello");
        assert_eq!(pending[0].queue_position, Some(1));
        assert_eq!(pending[0].attachments.len(), 2);
    }

    #[tokio::test]
    async fn tc_in_003_queue_edit_keeps_hash_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let pool = setup_migrated_pool().await;
        let service = AttachmentService::new(dir.path().to_path_buf(), pool.clone(), "instance-a");
        let first = image_id(dir.path(), &service, "a.png").await;
        let input = incoming(
            "U",
            "hello",
            vec![AttachmentUse::Image {
                attachment_id: first.clone(),
            }],
        );
        let accepted = accept_input(&pool, &input).await.unwrap();
        let edited = edit_input(&pool, "U", "edit-1", 1, Some("changed"), None)
            .await
            .unwrap();
        assert_eq!(edited.revision, 2);
        assert_eq!(edited.payload_hash, accepted.payload_hash);
        assert_eq!(
            pending_inputs(&pool, "sess").await.unwrap()[0]
                .attachments
                .len(),
            1
        );
        let again = edit_input(&pool, "U", "edit-1", 2, Some("other"), None)
            .await
            .unwrap();
        assert_eq!(again.revision, 2);
        assert_eq!(
            edit_input(&pool, "U", "edit-2", 1, Some("stale"), None)
                .await
                .unwrap_err()
                .code,
            REVISION_CONFLICT
        );
        edit_input(&pool, "U", "edit-3", 2, None, Some(&[]))
            .await
            .unwrap();
        assert!(pending_inputs(&pool, "sess").await.unwrap()[0]
            .attachments
            .is_empty());
        mark_applied(&pool, "U", "msg").await.unwrap();
        assert_eq!(
            edit_input(&pool, "U", "edit-4", 3, Some("nope"), None)
                .await
                .unwrap_err()
                .code,
            INPUT_NOT_EDITABLE
        );
    }
}
