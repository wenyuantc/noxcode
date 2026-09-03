//! 工具结果 artifact：超过模型预算的大输出完整落盘，模型只拿到头 / 尾预览。
//!
//! 文件放在 `$APPCONFIG/artifacts/<session_record_id>/<artifact_id>.txt`，
//! 索引写 `native_tool_artifacts` 表。Read 工具允许读取 artifact 目录，模型可
//! 按需分段查看完整内容。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::model::types::ToolCall;
use crate::native::tools::contract::{PreviewDirection, ResultStrategy, ToolContract};

pub const ARTIFACTS_DIR_NAME: &str = "artifacts";
pub const DEFAULT_ARTIFACT_RETENTION_DAYS: i32 = 7;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub id: String,
    pub session_record_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub bytes: i64,
    pub path: String,
    pub created_at: String,
}

pub type ArtifactRecorder = Arc<dyn Fn(ArtifactRecord) + Send + Sync>;

/// 每个会话一个存储目录。`recorder` 负责把索引行写进 SQLite（会话层注入）。
#[derive(Clone)]
pub struct ArtifactStore {
    session_record_id: String,
    dir: PathBuf,
    recorder: Option<ArtifactRecorder>,
}

impl std::fmt::Debug for ArtifactStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactStore")
            .field("session_record_id", &self.session_record_id)
            .field("dir", &self.dir)
            .finish()
    }
}

pub fn artifacts_root(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(ARTIFACTS_DIR_NAME)
}

impl ArtifactStore {
    pub fn new(app_config_dir: &Path, session_record_id: &str) -> Self {
        Self {
            session_record_id: session_record_id.to_string(),
            dir: artifacts_root(app_config_dir).join(sanitize_segment(session_record_id)),
            recorder: None,
        }
    }

    pub fn with_recorder(mut self, recorder: ArtifactRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 把完整输出写成文件并登记索引。
    pub fn store(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> Result<ArtifactRecord, String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| format!("创建 artifact 目录失败: {error}"))?;
        let id = new_id();
        let path = self.dir.join(format!("{id}.txt"));
        std::fs::write(&path, content.as_bytes())
            .map_err(|error| format!("写入 artifact 失败: {error}"))?;
        let record = ArtifactRecord {
            id,
            session_record_id: self.session_record_id.clone(),
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            bytes: content.len() as i64,
            path: path.to_string_lossy().into_owned(),
            created_at: now_sqlite(),
        };
        if let Some(recorder) = &self.recorder {
            recorder(record.clone());
        }
        Ok(record)
    }
}

fn sanitize_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "session".to_string()
    } else {
        cleaned
    }
}

/// 按字符边界截取预览。
pub fn preview(text: &str, max_bytes: usize, direction: PreviewDirection) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    match direction {
        PreviewDirection::Head => {
            let mut end = max_bytes;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_string()
        }
        PreviewDirection::Tail => {
            let mut start = text.len() - max_bytes;
            while start < text.len() && !text.is_char_boundary(start) {
                start += 1;
            }
            text[start..].to_string()
        }
    }
}

/// 结果预算裁决：超过 `max_model_bytes` 且策略为 Artifact 时落盘并返回预览 + 提示；
/// 其余情况原样返回，由调用方的 token 截断兜底。
pub fn bound_with_artifact(
    store: Option<&ArtifactStore>,
    contract: &ToolContract,
    call: &ToolCall,
    output: &str,
) -> String {
    let budget = contract.result_budget;
    if output.len() <= budget.max_model_bytes {
        return output.to_string();
    }
    if budget.strategy != ResultStrategy::Artifact {
        return output.to_string();
    }
    let Some(store) = store else {
        return output.to_string();
    };
    let shown = preview(output, budget.max_model_bytes, budget.preview);
    match store.store(&call.id, &call.name, output) {
        Ok(record) => {
            let where_shown = match budget.preview {
                PreviewDirection::Head => "开头",
                PreviewDirection::Tail => "末尾",
            };
            format!(
                "{shown}\n\n…[输出共 {} 字节，已保存为 artifact {}；此处仅显示{where_shown} {} 字节。需要完整内容时用 Read 读取 {}]",
                record.bytes,
                record.id,
                shown.len(),
                record.path
            )
        }
        Err(error) => {
            eprintln!("[native] 保存 artifact 失败: {error}");
            output.to_string()
        }
    }
}

pub async fn insert_artifact_row(pool: &SqlitePool, record: &ArtifactRecord) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO native_tool_artifacts (id, session_record_id, tool_call_id, tool_name, bytes, path, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&record.id)
    .bind(&record.session_record_id)
    .bind(&record.tool_call_id)
    .bind(&record.tool_name)
    .bind(record.bytes)
    .bind(&record.path)
    .bind(&record.created_at)
    .execute(pool)
    .await
    .map_err(|error| format!("写入 artifact 索引失败: {error}"))?;
    Ok(())
}

pub async fn list_session_artifacts(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Vec<ArtifactRecord>, String> {
    sqlx::query_as::<_, (String, String, String, String, i64, String, String)>(
        "SELECT id, session_record_id, tool_call_id, tool_name, bytes, path, created_at
         FROM native_tool_artifacts WHERE session_record_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_record_id)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| ArtifactRecord {
                id: row.0,
                session_record_id: row.1,
                tool_call_id: row.2,
                tool_name: row.3,
                bytes: row.4,
                path: row.5,
                created_at: row.6,
            })
            .collect()
    })
    .map_err(|error| format!("读取 artifact 索引失败: {error}"))
}

/// 删除某会话的全部 artifact（文件 + 索引）。
pub async fn delete_session_artifacts(
    pool: &SqlitePool,
    app_config_dir: &Path,
    session_record_id: &str,
) -> Result<usize, String> {
    let records = list_session_artifacts(pool, session_record_id).await?;
    for record in &records {
        let _ = std::fs::remove_file(&record.path);
    }
    sqlx::query("DELETE FROM native_tool_artifacts WHERE session_record_id = $1")
        .bind(session_record_id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除 artifact 索引失败: {error}"))?;
    let dir = artifacts_root(app_config_dir).join(sanitize_segment(session_record_id));
    let _ = std::fs::remove_dir(&dir);
    Ok(records.len())
}

/// 清理超过保留期的 artifact。`retention_days == 0` 表示不清理。
pub async fn prune_expired_artifacts(
    pool: &SqlitePool,
    app_config_dir: &Path,
    retention_days: i32,
) -> Result<usize, String> {
    if retention_days <= 0 {
        return Ok(0);
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));
    let cutoff_text = cutoff.format("%Y-%m-%d %H:%M:%S").to_string();
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, path FROM native_tool_artifacts WHERE created_at < $1",
    )
    .bind(&cutoff_text)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取过期 artifact 失败: {error}"))?;
    for (_, path) in &rows {
        let _ = std::fs::remove_file(path);
    }
    sqlx::query("DELETE FROM native_tool_artifacts WHERE created_at < $1")
        .bind(&cutoff_text)
        .execute(pool)
        .await
        .map_err(|error| format!("删除过期 artifact 索引失败: {error}"))?;
    // 顺带清掉索引里没有、但目录里残留的旧文件（例如崩溃后未登记的）。
    prune_orphan_files(&artifacts_root(app_config_dir), retention_days);
    Ok(rows.len())
}

fn prune_orphan_files(root: &Path, retention_days: i32) {
    let Ok(sessions) = std::fs::read_dir(root) else {
        return;
    };
    let max_age = Duration::from_secs(u64::from(retention_days.max(0) as u32) * 86_400);
    let now = SystemTime::now();
    for session_dir in sessions.flatten() {
        let path = session_dir.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let file_path = file.path();
            let expired = file
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age);
            if expired {
                let _ = std::fs::remove_file(file_path);
            }
        }
        let _ = std::fs::remove_dir(&path);
    }
}

/// 启动时按设置清理过期 artifact。
pub fn spawn_startup_prune<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(dir) = app.path().app_config_dir() else {
            return;
        };
        let retention = crate::native::settings::load_native_settings(&app)
            .map(|settings| settings.artifact_retention_days)
            .unwrap_or(DEFAULT_ARTIFACT_RETENTION_DAYS);
        let Ok(pool) = crate::app::shared::sqlite_pool(&app).await else {
            return;
        };
        match prune_expired_artifacts(&pool, &dir, retention).await {
            Ok(count) if count > 0 => eprintln!("[native] 已清理 {count} 个过期 artifact"),
            Ok(_) => {}
            Err(error) => eprintln!("[native] 清理 artifact 失败: {error}"),
        }
    });
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|item| item.as_millis() as u64)
        .unwrap_or(0)
}

/// 进程内唯一的后缀（纳秒 + 计数器），供并行测试建临时目录用。
pub fn unique_suffix() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|item| item.as_nanos())
        .unwrap_or(0);
    format!(
        "{nanos}-{}",
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::tools::contract::builtin_contract;
    use std::sync::Mutex;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noxcode-artifacts-{}", unique_suffix()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn preview_respects_char_boundaries_in_both_directions() {
        let text = "汉字abc汉字";
        assert_eq!(preview(text, 4, PreviewDirection::Head), "汉");
        assert_eq!(preview(text, 4, PreviewDirection::Tail), "字");
        assert_eq!(preview("short", 100, PreviewDirection::Head), "short");
    }

    #[test]
    fn bash_output_over_budget_is_stored_and_previewed_from_tail() {
        let dir = temp_dir();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let sink = recorded.clone();
        let store = ArtifactStore::new(&dir, "sess/1").with_recorder(Arc::new(move |record| {
            sink.lock().expect("lock").push(record);
        }));
        let contract = builtin_contract("Bash").expect("bash").clone();
        let call = ToolCall {
            id: "call-1".to_string(),
            name: "Bash".to_string(),
            arguments: String::new(),
        };
        let output = format!(
            "{}END",
            "x".repeat(contract.result_budget.max_model_bytes + 10)
        );
        let bounded = bound_with_artifact(Some(&store), &contract, &call, &output);
        assert!(bounded.contains("已保存为 artifact"));
        assert!(bounded.contains("末尾"));
        assert!(bounded.starts_with("xxx"));
        assert!(bounded.contains("END\n\n…["));
        let records = recorded.lock().expect("lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool_name, "Bash");
        assert_eq!(records[0].bytes as usize, output.len());
        assert_eq!(
            std::fs::read_to_string(&records[0].path).expect("read"),
            output
        );
        assert!(records[0].path.contains("sess_1"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn truncate_strategy_and_small_outputs_pass_through() {
        let dir = temp_dir();
        let store = ArtifactStore::new(&dir, "s");
        let read = builtin_contract("Read").expect("read").clone();
        let call = ToolCall::default();
        let big = "y".repeat(read.result_budget.max_model_bytes + 1);
        assert_eq!(bound_with_artifact(Some(&store), &read, &call, &big), big);
        let bash = builtin_contract("Bash").expect("bash").clone();
        assert_eq!(
            bound_with_artifact(Some(&store), &bash, &call, "small"),
            "small"
        );
        assert_eq!(bound_with_artifact(None, &bash, &call, &big), big);
        let _ = std::fs::remove_dir_all(dir);
    }
}
