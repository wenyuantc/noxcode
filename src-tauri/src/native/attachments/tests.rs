use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use super::service::{AttachmentService, ImportFault, Subject};
use crate::app::shared::now_sqlite;
use crate::db::test_support::setup_migrated_pool;
use crate::native::media_error::{
    ATTACHMENT_DELETING, CHUNK_CONFLICT, CHUNK_OUT_OF_ORDER, CHUNK_TOO_LARGE,
    DECLARED_SIZE_EXCEEDED, IMPORT_INVALID, INTEGRITY_MISMATCH, INVALID_CHUNK, PATH_ESCAPE,
    PREVIEW_EXPIRED, PREVIEW_REVOKED, SOURCE_CHANGED, UNAUTHORIZED,
};
use crate::native::media_limits::{MAX_CHUNK_BYTES, MAX_ORIGINAL_BYTES};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

async fn setup(instance: &str) -> (tempfile::TempDir, SqlitePool, AttachmentService) {
    let dir = tempfile::tempdir().expect("temp");
    let pool = setup_migrated_pool().await;
    let service = AttachmentService::new(dir.path().to_path_buf(), pool.clone(), instance);
    (dir, pool, service)
}

fn png_bytes() -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([10, 20, 30])));
    let mut cursor = Cursor::new(Vec::new());
    image.write_to(&mut cursor, ImageFormat::Png).expect("png");
    cursor.into_inner()
}

fn sha(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn subject(session: Option<&str>, draft: Option<&str>, instance: &str) -> Subject {
    Subject {
        session_id: session.map(str::to_string),
        draft_id: draft.map(str::to_string),
        instance_id: instance.to_string(),
    }
}

async fn seed_branch(pool: &SqlitePool, session: &str, branch: &str, messages: &[&str]) {
    let now = now_sqlite();
    sqlx::query(
        r#"
        INSERT INTO native_history_branches (
            id, session_record_id, revision, active, sealed, legacy_baseline, gaps_json,
            format_version, created_at, updated_at
        ) VALUES ($1, $2, 1, 1, 0, 0, '[]', 1, $3, $3)
        "#,
    )
    .bind(branch)
    .bind(session)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    let retained = serde_json::json!(messages);
    let projection = serde_json::json!({
        "format_version": 1,
        "items": messages.iter().map(|id| serde_json::json!({"message_id": id})).collect::<Vec<_>>(),
        "retained_message_ids": retained,
    });
    sqlx::query(
        r#"
        INSERT INTO native_context_anchors (
            branch_id, session_record_id, revision, projection_json, format_version, updated_at
        ) VALUES ($1, $2, 1, $3, 1, $4)
        "#,
    )
    .bind(branch)
    .bind(session)
    .bind(projection.to_string())
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn tc_att_001_managed_bytes_survive_source_removal() {
    let (dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let source = dir.path().join("source.png");
    fs::write(&source, &bytes).unwrap();
    let imported = service
        .import_path(&source, "draft-1", MAX_ORIGINAL_BYTES)
        .await
        .unwrap();
    fs::remove_file(&source).unwrap();
    let loaded = service.read_bytes(&imported.id).await.unwrap();
    assert_eq!(loaded, bytes);
    assert_eq!(imported.sha256, sha(&bytes));
    assert_eq!(imported.mime, "image/png");
    assert_eq!((imported.width, imported.height), (Some(2), Some(2)));
    assert_eq!(service.ready_count().await.unwrap(), 1);
}

#[tokio::test]
async fn tc_att_002_retrying_same_chunk_does_not_duplicate_prefix() {
    let (_dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let split = bytes.len() / 2;
    let import_id = service
        .begin_import("draft-1", "a.png", bytes.len() as u64, "image/png")
        .await
        .unwrap();
    let first = service
        .append_chunk(&import_id, 0, &bytes[..split])
        .await
        .unwrap();
    let retried = service
        .append_chunk(&import_id, 0, &bytes[..split])
        .await
        .unwrap();
    assert_eq!(first, retried);
    let done = service
        .append_chunk(&import_id, split as u64, &bytes[split..])
        .await
        .unwrap();
    assert_eq!(done, bytes.len() as u64);
    let imported = service.finish_import(&import_id).await.unwrap();
    assert_eq!(service.read_bytes(&imported.id).await.unwrap(), bytes);
    assert_eq!(imported.sha256, sha(&bytes));
    assert_eq!(service.ready_count().await.unwrap(), 1);
    assert_eq!(service.lease_count(&imported.id).await.unwrap(), 1);
    let again = service.finish_import(&import_id).await.unwrap();
    assert_eq!(again.id, imported.id);
    assert_eq!(service.lease_count(&imported.id).await.unwrap(), 1);
}

#[tokio::test]
async fn tc_att_003_bad_chunks_leave_confirmed_prefix() {
    let (_dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let import_id = service
        .begin_import("draft-1", "a.png", bytes.len() as u64, "image/png")
        .await
        .unwrap();
    service
        .append_chunk(&import_id, 0, &bytes[..4])
        .await
        .unwrap();
    let cases = [
        (
            service
                .append_chunk(&import_id, 0, b"nope")
                .await
                .unwrap_err()
                .code,
            CHUNK_CONFLICT,
        ),
        (
            service
                .append_chunk(&import_id, 8, b"skip")
                .await
                .unwrap_err()
                .code,
            CHUNK_OUT_OF_ORDER,
        ),
        (
            service
                .append_chunk_base64(&import_id, 4, "!!!!")
                .await
                .unwrap_err()
                .code,
            INVALID_CHUNK,
        ),
        (
            service
                .append_chunk(&import_id, 4, &vec![1; MAX_CHUNK_BYTES + 1])
                .await
                .unwrap_err()
                .code,
            CHUNK_TOO_LARGE,
        ),
    ];
    for (actual, expected) in cases {
        assert_eq!(actual, expected);
    }
    let confirmed = service
        .append_chunk(&import_id, 4, &bytes[4..])
        .await
        .unwrap();
    assert_eq!(confirmed, bytes.len() as u64);
    let imported = service.finish_import(&import_id).await.unwrap();
    assert_eq!(service.read_bytes(&imported.id).await.unwrap(), bytes);
}

#[tokio::test]
async fn tc_att_004_cancel_and_finish_are_ordered_by_the_import_lock() {
    let (_dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let service = Arc::new(service);
    for cancel_first in [false, true] {
        let import_id = service
            .begin_import("draft-1", "a.png", bytes.len() as u64, "image/png")
            .await
            .unwrap();
        service.append_chunk(&import_id, 0, &bytes).await.unwrap();
        service.pauses().arm("finish-enter");
        service.pauses().arm("cancel-enter");
        let finish_service = service.clone();
        let cancel_service = service.clone();
        let finish_id = import_id.clone();
        let cancel_id = import_id.clone();
        let finish = tokio::spawn(async move { finish_service.finish_import(&finish_id).await });
        let cancel = tokio::spawn(async move { cancel_service.cancel_import(&cancel_id).await });
        service.pauses().until_entered("finish-enter").await;
        service.pauses().until_entered("cancel-enter").await;
        if cancel_first {
            service.pauses().release("cancel-enter");
            assert_eq!(cancel.await.unwrap().unwrap(), "cancelled");
            service.pauses().release("finish-enter");
            assert_eq!(finish.await.unwrap().unwrap_err().code, IMPORT_INVALID);
        } else {
            service.pauses().release("finish-enter");
            let imported = finish.await.unwrap().unwrap();
            service.pauses().release("cancel-enter");
            assert_eq!(cancel.await.unwrap().unwrap(), "ready");
            assert_eq!(service.read_bytes(&imported.id).await.unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn tc_att_005_invalid_imports_do_not_commit_and_tamper_is_detected() {
    let (dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let good = service
        .begin_import("draft-keep", "keep.png", bytes.len() as u64, "image/png")
        .await
        .unwrap();
    service.append_chunk(&good, 0, &bytes).await.unwrap();
    let kept = service.finish_import(&good).await.unwrap();

    let empty = service
        .begin_import("draft-2", "empty.png", 0, "image/png")
        .await
        .unwrap();
    assert_eq!(
        service.finish_import(&empty).await.unwrap_err().code,
        IMPORT_INVALID
    );

    let mismatch = service
        .begin_import("draft-3", "short.png", 32, "image/png")
        .await
        .unwrap();
    service
        .append_chunk(&mismatch, 0, &bytes[..4])
        .await
        .unwrap();
    assert_eq!(
        service.finish_import(&mismatch).await.unwrap_err().code,
        IMPORT_INVALID
    );

    let corrupt = service
        .begin_import("draft-4", "bad.png", 5, "image/png")
        .await
        .unwrap();
    service.append_chunk(&corrupt, 0, b"hello").await.unwrap();
    assert_eq!(
        service.finish_import(&corrupt).await.unwrap_err().code,
        IMPORT_INVALID
    );
    assert_eq!(service.ready_count().await.unwrap(), 1);
    assert_eq!(service.read_bytes(&kept.id).await.unwrap(), bytes);

    let mut object = None;
    for entry in walk(dir.path()) {
        if entry.extension().and_then(|ext| ext.to_str()) == Some("bin") {
            object = Some(entry);
        }
    }
    let object = object.expect("managed object");
    let mut stored = fs::read(&object).unwrap();
    stored[0] ^= 0xff;
    fs::write(&object, stored).unwrap();
    assert_eq!(
        service.read_bytes(&kept.id).await.unwrap_err().code,
        INTEGRITY_MISMATCH
    );
}

#[tokio::test]
async fn tc_att_006_faults_leave_no_ready_row_and_orphans_are_recoverable() {
    let (dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let source = dir.path().join("source.png");
    fs::write(&source, &bytes).unwrap();
    service.arm_fault(ImportFault::BeforeTempWrite);
    assert!(service
        .import_path(&source, "draft", MAX_ORIGINAL_BYTES)
        .await
        .is_err());
    assert_eq!(service.ready_count().await.unwrap(), 0);
    assert!(source.is_file());

    service.arm_fault(ImportFault::AfterRename);
    assert!(service
        .import_path(&source, "draft", MAX_ORIGINAL_BYTES)
        .await
        .is_err());
    assert_eq!(service.ready_count().await.unwrap(), 0);
    let orphans = service.recover_orphans().await.unwrap();
    assert_eq!(orphans.len(), 1);
    assert!(service.recover_orphans().await.unwrap().is_empty());
    assert!(source.is_file());

    service.arm_fault(ImportFault::BeforeDbCommit);
    assert!(service
        .import_path(&source, "draft", MAX_ORIGINAL_BYTES)
        .await
        .is_err());
    assert_eq!(service.ready_count().await.unwrap(), 0);
    assert_eq!(service.recover_orphans().await.unwrap().len(), 1);
    assert!(source.is_file());
}

#[tokio::test]
async fn tc_att_008_source_change_and_oversize_do_not_commit() {
    let (dir, _pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let source = dir.path().join("source.png");
    fs::write(&source, &bytes).unwrap();
    service.pauses().arm("source-read");
    let service = Arc::new(service);
    let reading = service.clone();
    let path = source.clone();
    let task = tokio::spawn(async move {
        reading
            .import_path(&path, "draft", MAX_ORIGINAL_BYTES)
            .await
    });
    service.pauses().until_entered("source-read").await;
    fs::write(&source, b"changed-bytes").unwrap();
    service.pauses().release("source-read");
    assert_eq!(task.await.unwrap().unwrap_err().code, SOURCE_CHANGED);
    assert_eq!(service.ready_count().await.unwrap(), 0);

    let huge = dir.path().join("huge.png");
    fs::write(&huge, vec![1; 8]).unwrap();
    assert_eq!(
        service
            .import_path(&huge, "draft", 4)
            .await
            .unwrap_err()
            .code,
        DECLARED_SIZE_EXCEEDED
    );
    assert_eq!(service.ready_count().await.unwrap(), 0);
}

#[tokio::test]
async fn tc_auth_001_visibility_follows_branch_membership_not_workspace() {
    let (_dir, pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let imported = service
        .import_path(
            &write_png(&_dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    seed_branch(&pool, "session-a", "branch-a", &["m1"]).await;
    seed_branch(&pool, "fork", "branch-fork", &["m1"]).await;
    seed_branch(&pool, "other", "branch-other", &["m9"]).await;
    service
        .add_use(
            &imported.id,
            "message",
            "m1",
            0,
            r#"{"kind":"image","attachment_id":"x"}"#,
        )
        .await
        .unwrap();
    service.release_draft("draft").await.unwrap();
    assert!(service
        .authorize(
            &subject(Some("session-a"), None, "instance-a"),
            &imported.id
        )
        .await
        .is_ok());
    assert!(service
        .authorize(&subject(Some("fork"), None, "instance-a"), &imported.id)
        .await
        .is_ok());
    assert_eq!(
        service
            .authorize(&subject(Some("other"), None, "instance-a"), &imported.id)
            .await
            .unwrap_err()
            .code,
        UNAUTHORIZED
    );
    assert_eq!(
        service
            .authorize(&subject(None, None, "instance-a"), &imported.id)
            .await
            .unwrap_err()
            .code,
        UNAUTHORIZED
    );
    sqlx::query("UPDATE native_history_branches SET deleted_at = $1 WHERE id = 'branch-a'")
        .bind(now_sqlite())
        .execute(&pool)
        .await
        .unwrap();
    assert!(service
        .authorize(&subject(Some("fork"), None, "instance-a"), &imported.id)
        .await
        .is_ok());
}

#[tokio::test]
async fn tc_auth_002_preview_range_expiry_and_revoke() {
    let (_dir, pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let imported = service
        .import_path(
            &write_png(&_dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    seed_branch(&pool, "session-a", "branch-a", &["m1"]).await;
    service
        .add_use(&imported.id, "message", "m1", 0, "{}")
        .await
        .unwrap();
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let clock = Arc::new(Mutex::new(start));
    let clock_for_service = clock.clone();
    service.set_clock(move || *clock_for_service.lock().expect("clock"));
    let token = service
        .issue_preview(
            &subject(Some("session-a"), Some("draft"), "instance-a"),
            &imported.id,
            Some("branch-a"),
            "preview",
        )
        .await
        .unwrap();
    let ranged = service
        .read_preview(&token, Some("branch-a"), Some((0, 4)))
        .await
        .unwrap();
    assert_eq!(ranged, bytes[..4]);
    assert_eq!(
        service
            .read_preview(&token, Some("other-branch"), None)
            .await
            .unwrap_err()
            .code,
        PREVIEW_REVOKED
    );
    *clock.lock().expect("clock") = start + ChronoDuration::seconds(5 * 60);
    assert_eq!(
        service
            .read_preview(&token, Some("branch-a"), None)
            .await
            .unwrap_err()
            .code,
        PREVIEW_EXPIRED
    );
    *clock.lock().expect("clock") = start;
    let fresh = service
        .issue_preview(
            &subject(Some("session-a"), Some("draft"), "instance-a"),
            &imported.id,
            Some("branch-a"),
            "preview",
        )
        .await
        .unwrap();
    service.revoke_preview(&fresh).await.unwrap();
    assert_eq!(
        service
            .read_preview(&fresh, Some("branch-a"), None)
            .await
            .unwrap_err()
            .code,
        PREVIEW_REVOKED
    );
    let renewed = service
        .issue_preview(
            &subject(Some("session-a"), Some("draft"), "instance-a"),
            &imported.id,
            Some("branch-a"),
            "preview",
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .read_preview(&renewed, Some("branch-a"), None)
            .await
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn tc_auth_004_rejects_path_escape_without_touching_sentinel() {
    let (dir, pool, service) = setup("instance-a").await;
    let sentinel = dir.path().join("sentinel.txt");
    fs::write(&sentinel, b"secret-sentinel").unwrap();
    let bytes = png_bytes();
    let imported = service
        .import_path(
            &write_png(&dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE native_attachments SET relative_path = $1 WHERE id = $2")
        .bind("../sentinel.txt")
        .bind(&imported.id)
        .execute(&pool)
        .await
        .unwrap();
    let error = service.read_bytes(&imported.id).await.unwrap_err();
    assert_eq!(error.code, PATH_ESCAPE);
    assert!(!error.message.contains("secret-sentinel"));
    assert_eq!(fs::read(&sentinel).unwrap(), b"secret-sentinel");
    let shard = dir.path().join("objects").join("aa");
    let _ = shard;
    fs::remove_dir_all(dir.path().join("objects")).unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dir.path().join("objects")).unwrap();
    let escaped = service
        .import_path(
            &write_png(&dir, "b.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await;
    assert_eq!(escaped.unwrap_err().code, PATH_ESCAPE);
    assert_eq!(fs::read(&sentinel).unwrap(), b"secret-sentinel");
    assert_eq!(service.lease_count(&imported.id).await.unwrap(), 1);
}

#[tokio::test]
async fn tc_hist_001_fork_shares_refs_and_deleted_source_keeps_fork_media() {
    let (dir, pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let first = service
        .import_path(
            &write_png(&dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    let second = service
        .import_path(
            &write_png(&dir, "b.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    seed_branch(&pool, "source", "source-branch", &["m1", "m2"]).await;
    seed_branch(&pool, "fork", "fork-branch", &["m1"]).await;
    service
        .add_use(&first.id, "message", "m1", 0, "{}")
        .await
        .unwrap();
    service
        .add_use(&second.id, "message", "m2", 0, "{}")
        .await
        .unwrap();
    service.release_draft("draft").await.unwrap();
    service.seal_session("source").await.unwrap();
    assert!(service
        .authorize(&subject(Some("fork"), None, "instance-a"), &first.id)
        .await
        .is_ok());
    assert_eq!(
        service
            .authorize(&subject(Some("fork"), None, "instance-a"), &second.id)
            .await
            .unwrap_err()
            .code,
        UNAUTHORIZED
    );
    sqlx::query("UPDATE native_history_branches SET deleted_at = NULL, sealed = 1 WHERE id = 'source-branch'")
        .execute(&pool)
        .await
        .unwrap();
    let removed = service.gc().await.unwrap();
    assert!(!removed.contains(&second.id));
    assert_eq!(walk(&dir.path().join("objects")).len(), 2);
}

#[tokio::test]
async fn tc_hist_002_ref_and_gc_obey_barrier_order() {
    let (dir, pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let imported = service
        .import_path(
            &write_png(&dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    seed_branch(&pool, "session", "branch", &["m-new"]).await;
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    service.set_clock(move || start);
    service.release_draft("draft").await.unwrap();
    service.set_clock(move || start + ChronoDuration::seconds(24 * 60 * 60));
    let service = Arc::new(service);
    service.pauses().arm("gc-enter");
    service.pauses().arm("ref-enter");
    let gc_service = service.clone();
    let ref_service = service.clone();
    let id = imported.id.clone();
    let gc = tokio::spawn(async move { gc_service.gc().await });
    let refer =
        tokio::spawn(async move { ref_service.add_use(&id, "message", "m-new", 0, "{}").await });
    service.pauses().until_entered("gc-enter").await;
    service.pauses().until_entered("ref-enter").await;
    service.pauses().release("ref-enter");
    refer.await.unwrap().unwrap();
    service.pauses().release("gc-enter");
    assert!(gc.await.unwrap().unwrap().is_empty());

    service.pauses().arm("gc-enter");
    service.pauses().arm("ref-enter");
    service.clear_owner("message", "m-new").await.unwrap();
    service.set_clock(move || start + ChronoDuration::seconds(48 * 60 * 60));
    let gc_service = service.clone();
    let ref_service = service.clone();
    let id = imported.id.clone();
    let gc = tokio::spawn(async move { gc_service.gc().await });
    let refer =
        tokio::spawn(async move { ref_service.add_use(&id, "message", "m-late", 0, "{}").await });
    service.pauses().until_entered("gc-enter").await;
    service.pauses().until_entered("ref-enter").await;
    service.pauses().release("gc-enter");
    let removed = gc.await.unwrap().unwrap();
    assert!(removed.contains(&imported.id));
    service.pauses().release("ref-enter");
    let error = refer.await.unwrap().unwrap_err();
    assert!(error.code == ATTACHMENT_DELETING || error.code == "not_found");
}

#[tokio::test]
async fn tc_hist_003_gc_uses_last_release_plus_24_hours() {
    let (dir, pool, service) = setup("instance-a").await;
    let bytes = png_bytes();
    let imported = service
        .import_path(
            &write_png(&dir, "a.png", &bytes),
            "draft",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    let protected = service
        .import_path(
            &write_png(&dir, "b.png", &bytes),
            "draft-live",
            MAX_ORIGINAL_BYTES,
        )
        .await
        .unwrap();
    seed_branch(&pool, "session", "branch", &["m1"]).await;
    service
        .add_use(&protected.id, "message", "m1", 0, "{}")
        .await
        .unwrap();
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let clock = Arc::new(Mutex::new(start));
    let clock_for_service = clock.clone();
    service.set_clock(move || *clock_for_service.lock().expect("clock"));
    service.release_draft("draft").await.unwrap();
    *clock.lock().expect("clock") =
        start + ChronoDuration::seconds(24 * 60 * 60) - ChronoDuration::seconds(1);
    assert!(service.gc().await.unwrap().is_empty());
    assert!(service.read_bytes(&imported.id).await.is_ok());
    *clock.lock().expect("clock") = start + ChronoDuration::seconds(24 * 60 * 60);
    assert_eq!(service.gc().await.unwrap(), vec![imported.id.clone()]);
    assert!(service.read_bytes(&protected.id).await.is_ok());
}

fn write_png(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
