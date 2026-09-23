use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::import_file::{relative_object_path, StagingImport};
use super::sniff;
use crate::app::shared::{new_id, SQLITE_DATETIME_FORMAT};
use crate::native::media_error::{
    MediaError, ATTACHMENT_DELETING, ATTACHMENT_MISSING, DECLARED_SIZE_EXCEEDED, IMPORT_INVALID,
    INTEGRITY_MISMATCH, INVALID_CHUNK, INVALID_RANGE, NOT_FOUND, PATH_ESCAPE, PREVIEW_EXPIRED,
    PREVIEW_REVOKED, SOURCE_CHANGED, STORAGE_FAILED, UNAUTHORIZED,
};
use crate::native::media_limits::PREVIEW_TTL_SECONDS;
use crate::native::test_pause::PauseHub;

type NowFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[cfg(any(test, feature = "media-faults"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportFault {
    BeforeTempWrite,
    AfterRename,
    BeforeDbCommit,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AttachmentDescriptor {
    pub id: String,
    pub sha256: String,
    pub byte_count: u64,
    pub mime: String,
    pub media_type: String,
    pub original_name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub page_count: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub animated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Subject {
    pub session_id: Option<String>,
    pub draft_id: Option<String>,
    pub instance_id: String,
}

pub(crate) struct AttachmentService {
    root: PathBuf,
    pool: SqlitePool,
    instance_id: String,
    clock: Mutex<NowFn>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pauses: PauseHub,
    #[cfg(any(test, feature = "media-faults"))]
    fault: Mutex<Option<ImportFault>>,
}

impl AttachmentService {
    pub(crate) fn new(root: PathBuf, pool: SqlitePool, instance_id: impl Into<String>) -> Self {
        Self {
            root,
            pool,
            instance_id: instance_id.into(),
            clock: Mutex::new(Arc::new(Utc::now)),
            locks: Mutex::new(HashMap::new()),
            pauses: PauseHub::default(),
            #[cfg(any(test, feature = "media-faults"))]
            fault: Mutex::new(None),
        }
    }

    pub(crate) fn pauses(&self) -> &PauseHub {
        &self.pauses
    }

    pub(crate) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(crate) fn set_clock<F>(&self, clock: F)
    where
        F: Fn() -> DateTime<Utc> + Send + Sync + 'static,
    {
        *self.clock.lock().expect("clock") = Arc::new(clock);
    }

    #[cfg(any(test, feature = "media-faults"))]
    pub(crate) fn arm_fault(&self, fault: ImportFault) {
        *self.fault.lock().expect("fault") = Some(fault);
    }

    pub(crate) async fn begin_import(
        &self,
        draft_id: &str,
        original_name: &str,
        declared_size: u64,
        mime: &str,
    ) -> Result<String, MediaError> {
        let import_id = new_id();
        let _guard = self.lock(&import_id).await;
        StagingImport::create(
            &self.root,
            &import_id,
            declared_size,
            original_name,
            mime,
            draft_id,
        )?;
        Ok(import_id)
    }

    pub(crate) async fn append_chunk(
        &self,
        import_id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, MediaError> {
        let _guard = self.lock(import_id).await;
        let mut staging = StagingImport::open(&self.root, import_id)?;
        staging.append(offset, bytes)
    }

    pub(crate) async fn append_chunk_base64(
        &self,
        import_id: &str,
        offset: u64,
        encoded: &str,
    ) -> Result<u64, MediaError> {
        let bytes = BASE64
            .decode(encoded.trim())
            .map_err(|_| MediaError::new(INVALID_CHUNK, "分块不是合法 base64"))?;
        self.append_chunk(import_id, offset, &bytes).await
    }

    pub(crate) async fn finish_import(
        &self,
        import_id: &str,
    ) -> Result<AttachmentDescriptor, MediaError> {
        self.pause("finish-enter").await;
        let _guard = self.lock(import_id).await;
        let mut staging = StagingImport::open(&self.root, import_id)?;
        if staging.meta.status == "ready" {
            let id = staging
                .meta
                .attachment_id
                .clone()
                .ok_or_else(|| MediaError::new(NOT_FOUND, "导入缺少附件"))?;
            return self.descriptor(&id).await;
        }
        if staging.meta.status == "cancelled" {
            return Err(MediaError::new(IMPORT_INVALID, "导入已取消"));
        }
        if staging.meta.confirmed != staging.meta.declared_size {
            return Err(MediaError::new(IMPORT_INVALID, "声明长度与实际不符"));
        }
        let bytes = staging.confirmed_bytes()?;
        self.store_ready(&mut staging, &bytes, "chunk").await
    }

    pub(crate) async fn cancel_import(&self, import_id: &str) -> Result<String, MediaError> {
        self.pause("cancel-enter").await;
        let _guard = self.lock(import_id).await;
        let mut staging = StagingImport::open(&self.root, import_id)?;
        if staging.meta.status == "ready" {
            return Ok("ready".to_string());
        }
        if staging.meta.status != "cancelled" {
            staging.remove_partial()?;
            staging.save_status("cancelled", None)?;
        }
        Ok("cancelled".to_string())
    }

    pub(crate) async fn import_path(
        &self,
        path: &Path,
        draft_id: &str,
        max_bytes: u64,
    ) -> Result<AttachmentDescriptor, MediaError> {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment.bin".to_string());
        let first = read_bounded(path, max_bytes)?;
        self.pause("source-read").await;
        let second = read_bounded(path, max_bytes)?;
        if first != second {
            return Err(MediaError::new(SOURCE_CHANGED, "来源文件在读取期间变化"));
        }
        if first.len() as u64 > max_bytes {
            return Err(MediaError::new(DECLARED_SIZE_EXCEEDED, "来源超过读取上限"));
        }
        let import_id = new_id();
        let mut staging = StagingImport::create(
            &self.root,
            &import_id,
            first.len() as u64,
            &name,
            "",
            draft_id,
        )?;
        staging.append(0, &first)?;
        self.store_ready(&mut staging, &first, "path").await
    }

    pub(crate) async fn import_bytes(
        &self,
        name: &str,
        bytes: &[u8],
        draft_id: &str,
    ) -> Result<AttachmentDescriptor, MediaError> {
        let import_id = new_id();
        let mut staging = StagingImport::create(
            &self.root,
            &import_id,
            bytes.len() as u64,
            name,
            "",
            draft_id,
        )?;
        staging.append(0, bytes)?;
        self.store_ready(&mut staging, bytes, "bytes").await
    }

    pub(crate) async fn describe(&self, id: &str) -> Result<AttachmentDescriptor, MediaError> {
        self.descriptor(id).await
    }

    async fn store_ready(
        &self,
        staging: &mut StagingImport,
        bytes: &[u8],
        source_kind: &str,
    ) -> Result<AttachmentDescriptor, MediaError> {
        let sniffed = sniff::inspect(bytes, &staging.meta.original_name, &staging.meta.mime)?;
        let id = new_id();
        let relative = relative_object_path(&id);
        self.write_object(&relative, bytes)?;
        #[cfg(any(test, feature = "media-faults"))]
        if self.consume_fault(ImportFault::AfterRename) {
            return Err(MediaError::new(STORAGE_FAILED, "rename 后注入失败"));
        }
        let sha = sha256_hex(bytes);
        let now = self.now_text();
        let mut tx = self.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO native_attachments (
                id, relative_path, sha256, byte_count, mime, media_type, original_name,
                source_kind, metadata_json, status, file_status, import_id, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'ready','available',$10,$11,$11)
            "#,
        )
        .bind(&id)
        .bind(&relative)
        .bind(&sha)
        .bind(bytes.len() as i64)
        .bind(&sniffed.mime)
        .bind(&sniffed.media_type)
        .bind(&staging.meta.original_name)
        .bind(source_kind)
        .bind(sniffed.metadata_json())
        .bind(staging.dir_id())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;
        insert_lease(
            &mut tx,
            NewLease {
                attachment_id: &id,
                kind: "draft",
                holder_id: &staging.meta.draft_id,
                instance_id: &self.instance_id,
                context_json: "{}",
                expires_at: None,
                now: &now,
            },
        )
        .await?;
        #[cfg(any(test, feature = "media-faults"))]
        if self.consume_fault(ImportFault::BeforeDbCommit) {
            return Err(MediaError::new(STORAGE_FAILED, "提交元数据前注入失败"));
        }
        tx.commit().await.map_err(sql_err)?;
        staging.save_status("ready", Some(id.clone()))?;
        Ok(descriptor_from(
            id,
            sha,
            bytes.len() as u64,
            &sniffed,
            &staging.meta.original_name,
        ))
    }

    pub(crate) async fn read_bytes(&self, id: &str) -> Result<Vec<u8>, MediaError> {
        let row = self.attachment_row(id).await?;
        let relative: String = row.get("relative_path");
        let expected: String = row.get("sha256");
        let file_status: String = row.get("file_status");
        if file_status == "missing" {
            return Err(MediaError::new(ATTACHMENT_MISSING, "附件文件缺失"));
        }
        let path = self.resolve_inside(&relative)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.mark_file_status(id, "missing").await?;
                return Err(MediaError::new(ATTACHMENT_MISSING, "附件文件缺失"));
            }
        };
        if sha256_hex(&bytes) != expected {
            self.mark_file_status(id, "forbidden").await?;
            return Err(MediaError::new(INTEGRITY_MISMATCH, "附件哈希不一致"));
        }
        Ok(bytes)
    }

    pub(crate) async fn release_draft(&self, draft_id: &str) -> Result<u64, MediaError> {
        let mut tx = self.begin().await?;
        let ids = self
            .lease_attachment_ids(&mut tx, "draft", draft_id)
            .await?;
        let removed = sqlx::query(
            "DELETE FROM native_attachment_leases WHERE holder_kind = 'draft' AND holder_id = $1 AND instance_id = $2",
        )
        .bind(draft_id)
        .bind(&self.instance_id)
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?
        .rows_affected();
        for id in &ids {
            self.refresh_chain(&mut tx, id).await?;
        }
        tx.commit().await.map_err(sql_err)?;
        Ok(removed)
    }

    pub(crate) async fn add_use(
        &self,
        attachment_id: &str,
        owner_type: &str,
        owner_id: &str,
        position: i64,
        use_json: &str,
    ) -> Result<(), MediaError> {
        if !matches!(owner_type, "message" | "input" | "tool_run" | "event") {
            return Err(MediaError::new(IMPORT_INVALID, "引用所有者无效"));
        }
        serde_json::from_str::<Value>(use_json)
            .map_err(|_| MediaError::new(IMPORT_INVALID, "引用参数不是 JSON"))?;
        self.pause("ref-enter").await;
        let mut tx = self.begin().await?;
        let status: String =
            sqlx::query_scalar("SELECT status FROM native_attachments WHERE id = $1")
                .bind(attachment_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(sql_err)?
                .ok_or_else(|| MediaError::new(NOT_FOUND, "附件不存在"))?;
        if status == "deleting" {
            return Err(MediaError::new(ATTACHMENT_DELETING, "附件正在回收"));
        }
        if status != "ready" {
            return Err(MediaError::new(NOT_FOUND, "附件不可引用"));
        }
        sqlx::query(
            r#"
            INSERT INTO native_attachment_refs (
                id, attachment_id, owner_type, owner_id, position, use_json, created_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(new_id())
        .bind(attachment_id)
        .bind(owner_type)
        .bind(owner_id)
        .bind(position)
        .bind(use_json)
        .bind(self.now_text())
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;
        self.refresh_chain(&mut tx, attachment_id).await?;
        tx.commit().await.map_err(sql_err)?;
        Ok(())
    }

    pub(crate) async fn clear_owner(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<(), MediaError> {
        let mut tx = self.begin().await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT attachment_id FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2",
        )
        .bind(owner_type)
        .bind(owner_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_err)?;
        sqlx::query("DELETE FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2")
            .bind(owner_type)
            .bind(owner_id)
            .execute(&mut *tx)
            .await
            .map_err(sql_err)?;
        for id in &ids {
            self.refresh_chain(&mut tx, id).await?;
        }
        tx.commit().await.map_err(sql_err)?;
        Ok(())
    }

    pub(crate) async fn authorize(
        &self,
        subject: &Subject,
        attachment_id: &str,
    ) -> Result<(), MediaError> {
        if self.draft_holds(subject, attachment_id).await? {
            return Ok(());
        }
        if let Some(session_id) = subject.session_id.as_deref().filter(|id| !id.is_empty()) {
            if self.session_holds(session_id, attachment_id).await? {
                return Ok(());
            }
        }
        Err(MediaError::new(UNAUTHORIZED, "无权读取附件"))
    }

    pub(crate) async fn issue_preview(
        &self,
        subject: &Subject,
        attachment_id: &str,
        branch_id: Option<&str>,
        purpose: &str,
    ) -> Result<String, MediaError> {
        self.authorize(subject, attachment_id).await?;
        let token = Uuid::new_v4().to_string();
        let expires = self.now() + chrono::Duration::seconds(PREVIEW_TTL_SECONDS);
        let context = serde_json::json!({
            "session_id": subject.session_id,
            "branch_id": branch_id,
            "purpose": purpose,
        })
        .to_string();
        let mut tx = self.begin().await?;
        let now = self.now_text();
        insert_lease(
            &mut tx,
            NewLease {
                attachment_id,
                kind: "preview",
                holder_id: &token,
                instance_id: &self.instance_id,
                context_json: &context,
                expires_at: Some(expires.format(SQLITE_DATETIME_FORMAT).to_string()),
                now: &now,
            },
        )
        .await?;
        tx.commit().await.map_err(sql_err)?;
        Ok(token)
    }

    pub(crate) async fn read_preview(
        &self,
        token: &str,
        branch_id: Option<&str>,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<u8>, MediaError> {
        let row = sqlx::query(
            r#"
            SELECT attachment_id, instance_id, context_json, expires_at
            FROM native_attachment_leases
            WHERE holder_kind = 'preview' AND holder_id = $1
            "#,
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?
        .ok_or_else(|| MediaError::new(PREVIEW_REVOKED, "预览授权不存在"))?;
        let instance: String = row.get("instance_id");
        if instance != self.instance_id {
            return Err(MediaError::new(PREVIEW_REVOKED, "预览授权不属于当前实例"));
        }
        if let Some(expires) = row.get::<Option<String>, _>("expires_at") {
            if self.now() >= parse_time(&expires)? {
                return Err(MediaError::new(PREVIEW_EXPIRED, "预览授权已过期"));
            }
        }
        let context: Value = serde_json::from_str(row.get::<String, _>("context_json").as_str())
            .unwrap_or(Value::Null);
        if let Some(bound) = context.get("branch_id").and_then(Value::as_str) {
            if branch_id != Some(bound) {
                return Err(MediaError::new(PREVIEW_REVOKED, "预览授权与分支不一致"));
            }
        }
        let attachment_id: String = row.get("attachment_id");
        let bytes = self.read_bytes(&attachment_id).await?;
        slice_range(&bytes, range)
    }

    pub(crate) async fn revoke_preview(&self, token: &str) -> Result<(), MediaError> {
        sqlx::query(
            "DELETE FROM native_attachment_leases WHERE holder_kind = 'preview' AND holder_id = $1",
        )
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(())
    }

    pub(crate) async fn gc(&self) -> Result<Vec<String>, MediaError> {
        self.pause("gc-enter").await;
        let now = self.now();
        let now_text = self.now_text();
        let mut tx = self.begin().await?;
        let rows = sqlx::query(
            "SELECT id, status, unreferenced_at, relative_path FROM native_attachments WHERE status IN ('ready', 'deleting')",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_err)?;
        let mut deleting = Vec::new();
        for row in rows {
            let id: String = row.get("id");
            let status: String = row.get("status");
            if status == "deleting" {
                deleting.push((id, row.get::<String, _>("relative_path")));
                continue;
            }
            if self.is_held(&mut tx, &id).await? {
                sqlx::query("UPDATE native_attachments SET unreferenced_at = NULL, updated_at = $2 WHERE id = $1")
                    .bind(&id)
                    .bind(&now_text)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                continue;
            }
            let stamp: Option<String> = row.get("unreferenced_at");
            let Some(stamp) = stamp else {
                sqlx::query("UPDATE native_attachments SET unreferenced_at = $2, updated_at = $2 WHERE id = $1 AND unreferenced_at IS NULL")
                    .bind(&id)
                    .bind(&now_text)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                continue;
            };
            let due = parse_time(&stamp)?
                + chrono::Duration::seconds(crate::native::media_limits::UNREFERENCED_TTL_SECONDS);
            if now >= due && !self.is_held(&mut tx, &id).await? {
                sqlx::query("UPDATE native_attachments SET status = 'deleting', updated_at = $2 WHERE id = $1 AND status = 'ready'")
                    .bind(&id)
                    .bind(&now_text)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                deleting.push((id, row.get("relative_path")));
            }
        }
        tx.commit().await.map_err(sql_err)?;
        let mut removed = Vec::new();
        for (id, relative) in deleting {
            if self.remove_managed(&relative).is_ok() {
                let mut tx = self.begin().await?;
                sqlx::query("DELETE FROM native_attachment_refs WHERE attachment_id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                sqlx::query("DELETE FROM native_attachment_leases WHERE attachment_id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                sqlx::query("DELETE FROM native_attachments WHERE id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(sql_err)?;
                tx.commit().await.map_err(sql_err)?;
                removed.push(id);
            }
        }
        Ok(removed)
    }

    pub(crate) async fn seal_session(&self, session_id: &str) -> Result<(), MediaError> {
        self.pause("seal-enter").await;
        let mut tx = self.begin().await?;
        let now = self.now_text();
        sqlx::query(
            "UPDATE native_history_branches SET deleted_at = $2, active = 0, updated_at = $2 WHERE session_record_id = $1 AND deleted_at IS NULL",
        )
        .bind(session_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;
        let input_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM native_input_submissions WHERE session_record_id = $1",
        )
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_err)?;
        let tool_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM native_tool_runs WHERE session_record_id = $1")
                .bind(session_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(sql_err)?;
        let event_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM agent_session_events WHERE session_id = $1")
                .bind(session_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(sql_err)?;
        let mut touched = Vec::new();
        for (kind, ids) in [
            ("input", input_ids),
            ("tool_run", tool_ids),
            ("event", event_ids),
        ] {
            for owner_id in ids {
                let attachments: Vec<String> = sqlx::query_scalar(
                    "SELECT attachment_id FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2",
                )
                .bind(kind)
                .bind(&owner_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(sql_err)?;
                touched.extend(attachments);
                sqlx::query(
                    "DELETE FROM native_attachment_refs WHERE owner_type = $1 AND owner_id = $2",
                )
                .bind(kind)
                .bind(&owner_id)
                .execute(&mut *tx)
                .await
                .map_err(sql_err)?;
            }
        }
        touched.sort();
        touched.dedup();
        for id in &touched {
            self.refresh_chain(&mut tx, id).await?;
        }
        tx.commit().await.map_err(sql_err)?;
        Ok(())
    }

    pub(crate) async fn recover_orphans(&self) -> Result<Vec<String>, MediaError> {
        let known = self.known_paths().await?;
        let mut orphans = Vec::new();
        let objects = self.root.join("objects");
        if objects.is_dir() {
            for entry in walk_files(&objects) {
                let relative = entry
                    .strip_prefix(&self.root)
                    .map_err(|_| MediaError::new(PATH_ESCAPE, "对象路径越界"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !known.contains(&relative) {
                    fs::remove_file(&entry)
                        .map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))?;
                    orphans.push(relative);
                }
            }
        }
        Ok(orphans)
    }

    pub(crate) async fn lease_count(&self, attachment_id: &str) -> Result<i64, MediaError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM native_attachment_leases WHERE attachment_id = $1",
        )
        .bind(attachment_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(count)
    }

    pub(crate) async fn ready_count(&self) -> Result<i64, MediaError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM native_attachments WHERE status = 'ready'")
                .fetch_one(&self.pool)
                .await
                .map_err(sql_err)?;
        Ok(count)
    }

    async fn pause(&self, name: &str) {
        self.pauses.wait(name).await;
    }

    async fn lock(&self, import_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let arc = {
            let mut locks = self.locks.lock().expect("import locks");
            locks
                .entry(import_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        arc.lock_owned().await
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock.lock().expect("clock"))()
    }

    fn now_text(&self) -> String {
        self.now().format(SQLITE_DATETIME_FORMAT).to_string()
    }

    async fn begin(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, MediaError> {
        self.pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sql_err)
    }

    #[cfg(any(test, feature = "media-faults"))]
    fn armed_fault(&self) -> Option<ImportFault> {
        *self.fault.lock().expect("fault")
    }

    #[cfg(any(test, feature = "media-faults"))]
    fn clear_fault(&self) {
        *self.fault.lock().expect("fault") = None;
    }

    #[cfg(any(test, feature = "media-faults"))]
    fn consume_fault(&self, expected: ImportFault) -> bool {
        if self.armed_fault() == Some(expected) {
            self.clear_fault();
            true
        } else {
            false
        }
    }

    fn write_object(&self, relative: &str, bytes: &[u8]) -> Result<(), MediaError> {
        #[cfg(any(test, feature = "media-faults"))]
        if self.armed_fault() == Some(ImportFault::BeforeTempWrite) {
            self.clear_fault();
            return Err(MediaError::new(STORAGE_FAILED, "临时写入前注入失败"));
        }
        if relative.contains("..") || relative.starts_with('/') {
            return Err(MediaError::new(PATH_ESCAPE, "对象路径非法"));
        }
        let mut cursor = self.root.clone();
        for component in Path::new(relative).components() {
            cursor.push(component);
            if cursor.exists()
                && cursor
                    .symlink_metadata()
                    .map(|meta| meta.file_type().is_symlink())
                    .unwrap_or(false)
            {
                return Err(MediaError::new(PATH_ESCAPE, "对象路径包含符号链接"));
            }
        }
        let dest = self.root.join(relative);
        let parent = dest
            .parent()
            .ok_or_else(|| MediaError::new(PATH_ESCAPE, "对象目录非法"))?;
        if parent.exists() {
            if parent.read_link().is_ok() {
                return Err(MediaError::new(PATH_ESCAPE, "对象目录是符号链接"));
            }
            if let (Ok(root), Ok(canon)) = (self.root.canonicalize(), parent.canonicalize()) {
                if !canon.starts_with(&root) {
                    return Err(MediaError::new(PATH_ESCAPE, "对象目录越界"));
                }
            }
        }
        fs::create_dir_all(parent)
            .map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))?;
        let tmp = dest.with_extension("partial");
        fs::write(&tmp, bytes)
            .map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))?;
        fs::rename(&tmp, &dest)
            .map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))?;
        Ok(())
    }

    fn resolve_inside(&self, relative: &str) -> Result<PathBuf, MediaError> {
        if relative.is_empty()
            || relative.contains("..")
            || relative.starts_with('/')
            || relative.contains('\\')
        {
            return Err(MediaError::new(PATH_ESCAPE, "相对路径非法"));
        }
        let root = self
            .root
            .canonicalize()
            .map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))?;
        let candidate = root.join(relative);
        if !candidate.exists() {
            return Err(MediaError::new(ATTACHMENT_MISSING, "附件文件缺失"));
        }
        let canon = candidate
            .canonicalize()
            .map_err(|_| MediaError::new(ATTACHMENT_MISSING, "附件文件缺失"))?;
        if !canon.starts_with(&root) {
            return Err(MediaError::new(PATH_ESCAPE, "路径越界"));
        }
        Ok(canon)
    }

    fn remove_managed(&self, relative: &str) -> Result<(), MediaError> {
        let path = match self.resolve_inside(relative) {
            Ok(path) => path,
            Err(error) if error.code == ATTACHMENT_MISSING => return Ok(()),
            Err(error) => return Err(error),
        };
        fs::remove_file(path).map_err(|error| MediaError::new(STORAGE_FAILED, error.to_string()))
    }

    async fn descriptor(&self, id: &str) -> Result<AttachmentDescriptor, MediaError> {
        let row = self.attachment_row(id).await?;
        let metadata: Value = serde_json::from_str(row.get::<String, _>("metadata_json").as_str())
            .unwrap_or(Value::Null);
        Ok(AttachmentDescriptor {
            id: row.get("id"),
            sha256: row.get("sha256"),
            byte_count: row.get::<i64, _>("byte_count") as u64,
            mime: row.get("mime"),
            media_type: row.get("media_type"),
            original_name: row.get("original_name"),
            width: metadata
                .get("width")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
            height: metadata
                .get("height")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
            page_count: metadata
                .get("page_count")
                .and_then(Value::as_u64)
                .map(|v| v as u32),
            duration_seconds: metadata.get("duration_seconds").and_then(Value::as_f64),
            animated: metadata
                .get("animated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    async fn attachment_row(&self, id: &str) -> Result<sqlx::sqlite::SqliteRow, MediaError> {
        sqlx::query("SELECT * FROM native_attachments WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_err)?
            .ok_or_else(|| MediaError::new(NOT_FOUND, "附件不存在"))
    }

    async fn mark_file_status(&self, id: &str, status: &str) -> Result<(), MediaError> {
        sqlx::query(
            "UPDATE native_attachments SET file_status = $2, updated_at = $3 WHERE id = $1",
        )
        .bind(id)
        .bind(status)
        .bind(self.now_text())
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(())
    }

    async fn lease_attachment_ids(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        kind: &str,
        holder_id: &str,
    ) -> Result<Vec<String>, MediaError> {
        sqlx::query_scalar(
            "SELECT attachment_id FROM native_attachment_leases WHERE holder_kind = $1 AND holder_id = $2 AND instance_id = $3",
        )
        .bind(kind)
        .bind(holder_id)
        .bind(&self.instance_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(sql_err)
    }

    async fn refresh_chain(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        attachment_id: &str,
    ) -> Result<(), MediaError> {
        let mut current = Some(attachment_id.to_string());
        let mut guard = 0;
        while let Some(id) = current {
            guard += 1;
            if guard > 16 {
                break;
            }
            let now = self.now_text();
            if self.is_held(tx, &id).await? {
                sqlx::query("UPDATE native_attachments SET unreferenced_at = NULL, updated_at = $2 WHERE id = $1")
                    .bind(&id)
                    .bind(&now)
                    .execute(&mut **tx)
                    .await
                    .map_err(sql_err)?;
            } else {
                sqlx::query(
                    "UPDATE native_attachments SET unreferenced_at = COALESCE(unreferenced_at, $2), updated_at = $2 WHERE id = $1",
                )
                .bind(&id)
                .bind(&now)
                .execute(&mut **tx)
                .await
                .map_err(sql_err)?;
            }
            current = sqlx::query_scalar(
                "SELECT parent_attachment_id FROM native_attachments WHERE id = $1",
            )
            .bind(&id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(sql_err)?
            .flatten();
        }
        Ok(())
    }

    async fn is_held(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        attachment_id: &str,
    ) -> Result<bool, MediaError> {
        let mut pending = vec![attachment_id.to_string()];
        let mut seen = Vec::new();
        while let Some(id) = pending.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            if self.live_ref_count(tx, &id).await? > 0 || self.live_lease_count(tx, &id).await? > 0
            {
                return Ok(true);
            }
            let children: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM native_attachments WHERE parent_attachment_id = $1",
            )
            .bind(&id)
            .fetch_all(&mut **tx)
            .await
            .map_err(sql_err)?;
            pending.extend(children);
        }
        Ok(false)
    }

    async fn live_lease_count(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        attachment_id: &str,
    ) -> Result<i64, MediaError> {
        let rows =
            sqlx::query("SELECT expires_at FROM native_attachment_leases WHERE attachment_id = $1")
                .bind(attachment_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(sql_err)?;
        let now = self.now();
        let mut count = 0i64;
        for row in rows {
            match row.get::<Option<String>, _>("expires_at") {
                None => count += 1,
                Some(expires) if now < parse_time(&expires)? => count += 1,
                Some(_) => {}
            }
        }
        Ok(count)
    }

    async fn live_ref_count(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        attachment_id: &str,
    ) -> Result<i64, MediaError> {
        let rows = sqlx::query(
            "SELECT owner_type, owner_id FROM native_attachment_refs WHERE attachment_id = $1",
        )
        .bind(attachment_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(sql_err)?;
        let mut count = 0i64;
        for row in rows {
            let kind: String = row.get("owner_type");
            let owner: String = row.get("owner_id");
            if self.ref_is_live(tx, &kind, &owner).await? {
                count += 1;
            }
        }
        Ok(count)
    }

    async fn ref_is_live(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        kind: &str,
        owner_id: &str,
    ) -> Result<bool, MediaError> {
        let live = match kind {
            "message" => self.message_retained(tx, owner_id).await?,
            "input" => {
                self.row_exists(
                    tx,
                    "SELECT 1 FROM native_input_submissions WHERE id = $1",
                    owner_id,
                )
                .await?
            }
            "tool_run" => {
                self.row_exists(tx, "SELECT 1 FROM native_tool_runs WHERE id = $1", owner_id)
                    .await?
            }
            "event" => {
                self.row_exists(
                    tx,
                    "SELECT 1 FROM agent_session_events WHERE id = $1",
                    owner_id,
                )
                .await?
            }
            _ => false,
        };
        Ok(live)
    }

    async fn row_exists(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        sql: &str,
        id: &str,
    ) -> Result<bool, MediaError> {
        let found: Option<i64> = sqlx::query_scalar(sql)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(sql_err)?;
        Ok(found.is_some())
    }

    async fn message_retained(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        message_id: &str,
    ) -> Result<bool, MediaError> {
        let rows = sqlx::query(
            r#"
            SELECT a.projection_json
            FROM native_context_anchors a
            JOIN native_history_branches b ON b.id = a.branch_id
            WHERE b.deleted_at IS NULL
            "#,
        )
        .fetch_all(&mut **tx)
        .await
        .map_err(sql_err)?;
        for row in rows {
            let json: String = row.get("projection_json");
            let value: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
            let retained = value
                .get("retained_message_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(message_id)));
            if retained {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn draft_holds(
        &self,
        subject: &Subject,
        attachment_id: &str,
    ) -> Result<bool, MediaError> {
        let Some(draft_id) = subject.draft_id.as_deref().filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM native_attachment_leases
            WHERE attachment_id = $1 AND holder_kind = 'draft' AND holder_id = $2 AND instance_id = $3
            "#,
        )
        .bind(attachment_id)
        .bind(draft_id)
        .bind(&subject.instance_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(count > 0)
    }

    async fn session_holds(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<bool, MediaError> {
        let mut tx = self.begin().await?;
        let rows = sqlx::query(
            "SELECT owner_type, owner_id FROM native_attachment_refs WHERE attachment_id = $1",
        )
        .bind(attachment_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(sql_err)?;
        for row in rows {
            let kind: String = row.get("owner_type");
            let owner: String = row.get("owner_id");
            let matches = match kind.as_str() {
                "message" => self.message_visible_to_session(&mut tx, session_id, &owner).await?,
                "input" => {
                    self.scoped_row(
                        &mut tx,
                        "SELECT 1 FROM native_input_submissions WHERE id = $1 AND session_record_id = $2",
                        &owner,
                        session_id,
                    )
                    .await?
                }
                "tool_run" => {
                    self.scoped_row(
                        &mut tx,
                        "SELECT 1 FROM native_tool_runs WHERE id = $1 AND session_record_id = $2",
                        &owner,
                        session_id,
                    )
                    .await?
                }
                "event" => {
                    self.scoped_row(
                        &mut tx,
                        "SELECT 1 FROM agent_session_events WHERE id = $1 AND session_id = $2",
                        &owner,
                        session_id,
                    )
                    .await?
                }
                _ => false,
            };
            if matches {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn message_visible_to_session(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        session_id: &str,
        message_id: &str,
    ) -> Result<bool, MediaError> {
        let rows = sqlx::query(
            r#"
            SELECT a.projection_json
            FROM native_context_anchors a
            JOIN native_history_branches b ON b.id = a.branch_id
            WHERE b.session_record_id = $1 AND b.deleted_at IS NULL
            "#,
        )
        .bind(session_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(sql_err)?;
        for row in rows {
            let json: String = row.get("projection_json");
            let value: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
            if value
                .get("retained_message_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(message_id)))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn scoped_row(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        sql: &str,
        id: &str,
        session_id: &str,
    ) -> Result<bool, MediaError> {
        let found: Option<i64> = sqlx::query_scalar(sql)
            .bind(id)
            .bind(session_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(sql_err)?;
        Ok(found.is_some())
    }

    async fn known_paths(&self) -> Result<std::collections::HashSet<String>, MediaError> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT relative_path FROM native_attachments")
            .fetch_all(&self.pool)
            .await
            .map_err(sql_err)?;
        Ok(rows.into_iter().collect())
    }
}

struct NewLease<'a> {
    attachment_id: &'a str,
    kind: &'a str,
    holder_id: &'a str,
    instance_id: &'a str,
    context_json: &'a str,
    expires_at: Option<String>,
    now: &'a str,
}

async fn insert_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    lease: NewLease<'_>,
) -> Result<(), MediaError> {
    if lease.holder_id.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO native_attachment_leases (
            id, attachment_id, holder_kind, holder_id, instance_id, context_json, expires_at, created_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
        "#,
    )
    .bind(new_id())
    .bind(lease.attachment_id)
    .bind(lease.kind)
    .bind(lease.holder_id)
    .bind(lease.instance_id)
    .bind(lease.context_json)
    .bind(lease.expires_at)
    .bind(lease.now)
    .execute(&mut **tx)
    .await
    .map_err(sql_err)?;
    Ok(())
}

fn descriptor_from(
    id: String,
    sha: String,
    byte_count: u64,
    sniffed: &super::sniff::Sniffed,
    original_name: &str,
) -> AttachmentDescriptor {
    AttachmentDescriptor {
        id,
        sha256: sha,
        byte_count,
        mime: sniffed.mime.clone(),
        media_type: sniffed.media_type.clone(),
        original_name: original_name.to_string(),
        width: sniffed.width,
        height: sniffed.height,
        page_count: sniffed.page_count,
        duration_seconds: sniffed.duration_seconds,
        animated: sniffed.animated,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, MediaError> {
    let naive = NaiveDateTime::parse_from_str(value, SQLITE_DATETIME_FORMAT)
        .map_err(|_| MediaError::new(STORAGE_FAILED, "时间无效"))?;
    Ok(naive.and_utc())
}

fn slice_range(bytes: &[u8], range: Option<(u64, u64)>) -> Result<Vec<u8>, MediaError> {
    let Some((start, end)) = range else {
        return Ok(bytes.to_vec());
    };
    if start > end || end > bytes.len() as u64 {
        return Err(MediaError::new(INVALID_RANGE, "Range 超出附件"));
    }
    Ok(bytes[start as usize..end as usize].to_vec())
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, MediaError> {
    let mut file =
        File::open(path).map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| MediaError::new(IMPORT_INVALID, error.to_string()))?;
    Ok(bytes)
}

fn sql_err(error: sqlx::Error) -> MediaError {
    MediaError::new(STORAGE_FAILED, error.to_string())
}

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_files(&path));
        } else if path.is_file() {
            files.push(path);
        }
    }
    files
}
