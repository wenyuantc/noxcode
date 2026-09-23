//! 追加式会话历史。模型上下文是投影，压缩和重建都不得改写已提交消息的身份。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::agent::truncate::sanitize_tool_message_pairs;
use crate::native::model::types::{
    AttachmentUse, Message, NativeImage, PdfUseMode, Role, ToolCall,
};

const FORMAT_VERSION: i64 = 1;
const LOCK_ATTEMPTS: u32 = 5;

fn is_lock_error(message: &str) -> bool {
    message.contains("database is locked") || message.contains("database is busy")
}

async fn begin_write(
    pool: &SqlitePool,
) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, String> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| save_error(error.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityGap {
    pub kind: String,
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryLink {
    pub message_id: String,
    pub kind: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryCommitReceipt {
    pub session_record_id: String,
    pub branch_id: String,
    pub revision: i64,
    pub message_ids: Vec<String>,
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryBoundary {
    pub message_id: String,
    pub ordinal: i64,
    pub role: String,
    pub turn_id: Option<String>,
    pub selectable_before: bool,
    pub selectable_after: bool,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryEdge {
    Before,
    After,
}

impl BoundaryEdge {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|item| !item.is_empty()) {
            None | Some("after") => Ok(Self::After),
            Some("before") => Ok(Self::Before),
            Some(other) => Err(save_error(format!("未知的边界位置: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryBoundaries {
    pub session_record_id: String,
    pub branch_id: String,
    pub revision: i64,
    pub legacy_baseline: bool,
    pub gaps: Vec<CapabilityGap>,
    pub boundaries: Vec<HistoryBoundary>,
}

pub struct HistoryWrite<'a> {
    pub session_record_id: &'a str,
    pub profile_id: Option<&'a str>,
    pub workspace_id: Option<&'a str>,
    pub model: &'a str,
    pub turns: u32,
    pub messages: &'a mut [Message],
    pub turn_id: Option<&'a str>,
    pub attempt_id: Option<&'a str>,
    pub expected_revision: Option<i64>,
    pub request_id: Option<&'a str>,
    pub links: &'a [HistoryLink],
    pub legacy_baseline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ContextOverride {
    content: String,
    reasoning_content: String,
    tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProjectionItem {
    message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_override: Option<ContextOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredAnchor {
    format_version: i64,
    items: Vec<ProjectionItem>,
    #[serde(default)]
    retained_message_ids: Vec<String>,
}

struct BranchRow {
    id: String,
    revision: i64,
    legacy_baseline: bool,
    gaps: Vec<CapabilityGap>,
    format_version: i64,
}

struct RawMessage {
    id: String,
    branch_id: String,
    turn_id: Option<String>,
    attempt_id: Option<String>,
    ordinal: i64,
    role: String,
    content: String,
    tool_calls_json: String,
    tool_call_id: String,
    name: String,
    reasoning_content: String,
    images_unrecoverable: bool,
    format_version: i64,
}

fn save_error(message: impl AsRef<str>) -> String {
    let message = message.as_ref();
    if message.starts_with("保存会话历史失败") {
        message.to_string()
    } else {
        format!("保存会话历史失败: {message}")
    }
}

fn gap(kind: &str, reason: &str) -> CapabilityGap {
    CapabilityGap {
        kind: kind.to_string(),
        status: "unrecoverable".to_string(),
        reason: reason.to_string(),
    }
}

fn legacy_gaps() -> Vec<CapabilityGap> {
    vec![
        gap(
            "prior_history",
            "覆盖式 transcript 在压缩后无法还原已丢弃的原文",
        ),
        gap("images", "旧 transcript 不保留图片，不能还原"),
        gap("checkpoints", "旧检查点没有消息级归属，不能推测关联"),
    ]
}

fn merge_gap(gaps: &mut Vec<CapabilityGap>, incoming: CapabilityGap) {
    if !gaps
        .iter()
        .any(|item| item.kind == incoming.kind && item.reason == incoming.reason)
    {
        gaps.push(incoming);
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn parse_role(value: &str) -> Result<Role, String> {
    match value {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        _ => Err(save_error(format!("未知消息角色: {value}"))),
    }
}

fn tool_calls_json(message: &Message) -> Result<String, String> {
    serde_json::to_string(&message.tool_calls).map_err(|error| save_error(error.to_string()))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|item| !item.is_empty())
}

impl RawMessage {
    fn to_message(&self) -> Result<Message, String> {
        if self.format_version != FORMAT_VERSION {
            return Err(save_error(format!(
                "不支持的历史格式版本 {}",
                self.format_version
            )));
        }
        let tool_calls = serde_json::from_str(&self.tool_calls_json)
            .map_err(|error| save_error(format!("解析历史消息失败: {error}")))?;
        Ok(Message {
            role: parse_role(&self.role)?,
            content: self.content.clone(),
            tool_calls,
            tool_call_id: self.tool_call_id.clone(),
            name: self.name.clone(),
            reasoning_content: self.reasoning_content.clone(),
            images: Vec::new(),
            media: Vec::new(),
            history_id: self.id.clone(),
        })
    }
}

fn structure_matches(raw: &RawMessage, message: &Message) -> Result<bool, String> {
    Ok(raw.role == role_name(message.role)
        && raw.tool_call_id == message.tool_call_id
        && raw.name == message.name
        && raw.tool_calls_json == tool_calls_json(message)?)
}

fn exact_match(raw: &RawMessage, message: &Message) -> Result<bool, String> {
    Ok(structure_matches(raw, message)?
        && raw.content == message.content
        && raw.reasoning_content == message.reasoning_content)
}

fn compact_override(raw: &RawMessage, message: &Message) -> Result<bool, String> {
    let tool_result_or_call = message.role == Role::Tool
        || (message.role == Role::Assistant && !message.tool_calls.is_empty());
    Ok(tool_result_or_call
        && structure_matches(raw, message)?
        && (raw.content != message.content || raw.reasoning_content != message.reasoning_content))
}

fn pair_flags(messages: &[Message]) -> Vec<(bool, bool)> {
    let mut open = HashSet::new();
    let mut flags = Vec::with_capacity(messages.len());
    for message in messages {
        let before = open.is_empty();
        if message.role == Role::Assistant {
            for call in &message.tool_calls {
                if !call.id.is_empty() {
                    open.insert(call.id.clone());
                }
            }
        }
        if message.role == Role::Tool && !message.tool_call_id.is_empty() {
            open.remove(&message.tool_call_id);
        }
        flags.push((before, open.is_empty()));
    }
    flags
}

fn override_for(raw: &RawMessage, message: &Message) -> Result<Option<ContextOverride>, String> {
    if raw.content == message.content
        && raw.reasoning_content == message.reasoning_content
        && raw.tool_calls_json == tool_calls_json(message)?
    {
        Ok(None)
    } else {
        Ok(Some(ContextOverride {
            content: message.content.clone(),
            reasoning_content: message.reasoning_content.clone(),
            tool_calls: message.tool_calls.clone(),
        }))
    }
}

pub async fn commit_model_context(
    pool: &SqlitePool,
    mut request: HistoryWrite<'_>,
) -> Result<HistoryCommitReceipt, String> {
    let session_record_id = request.session_record_id.trim();
    if session_record_id.is_empty() {
        return Err(save_error("会话标识不能为空"));
    }
    let original_ids: Vec<String> = request
        .messages
        .iter()
        .map(|message| message.history_id.clone())
        .collect();
    let mut delay = Duration::from_millis(40);
    let mut last_error = String::new();
    for attempt in 0..LOCK_ATTEMPTS {
        if attempt > 0 {
            for (message, id) in request.messages.iter_mut().zip(&original_ids) {
                message.history_id.clone_from(id);
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(400));
        }
        let mut tx = match begin_write(pool).await {
            Ok(tx) => tx,
            Err(error) if is_lock_error(&error) && attempt + 1 < LOCK_ATTEMPTS => {
                last_error = error;
                continue;
            }
            Err(error) => return Err(error),
        };
        let receipt = match commit_in_tx(&mut tx, session_record_id, &mut request).await {
            Ok(receipt) => receipt,
            Err(error) if is_lock_error(&error) && attempt + 1 < LOCK_ATTEMPTS => {
                last_error = error;
                continue;
            }
            Err(error) => return Err(error),
        };
        match tx.commit().await {
            Ok(()) => return Ok(receipt),
            Err(error) => {
                let message = save_error(error.to_string());
                if is_lock_error(&message) && attempt + 1 < LOCK_ATTEMPTS {
                    last_error = message;
                    continue;
                }
                return Err(message);
            }
        }
    }
    Err(last_error)
}

async fn commit_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    request: &mut HistoryWrite<'_>,
) -> Result<HistoryCommitReceipt, String> {
    if let Some(receipt) = replay_request(tx, session_record_id, request.request_id).await? {
        return Ok(receipt);
    }
    let had_images = request.messages.iter().any(images_lack_attachment_ref);
    let mut branch = match load_active_branch(tx, session_record_id).await? {
        Some(branch) => branch,
        None => {
            if request
                .expected_revision
                .is_some_and(|expected| expected != 0)
            {
                return Err(save_error("分支修订号不匹配"));
            }
            insert_branch(tx, session_record_id, request.legacy_baseline).await?
        }
    };
    if let Some(expected) = request.expected_revision {
        if expected != branch.revision {
            return Err(save_error("分支修订号不匹配"));
        }
    }
    let mut anchor = load_anchor(tx, &branch.id).await?;
    let mut rows = load_messages(tx, &branch.id, &anchor.retained_message_ids).await?;
    let mut by_id: HashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id.clone(), index))
        .collect();
    let mut cursor = 0usize;
    let mut next_ordinal = rows.iter().map(|row| row.ordinal).max().unwrap_or(0);
    let mut inserted = Vec::new();
    let durable: Vec<usize> = request
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != Role::System)
        .map(|(index, _)| index)
        .collect();

    for index in durable {
        let history_id = request.messages[index].history_id.clone();
        if !history_id.is_empty() {
            let Some(raw_index) = by_id.get(&history_id).copied() else {
                return Err(save_error(format!("未知的消息身份 {history_id}")));
            };
            if !structure_matches(&rows[raw_index], &request.messages[index])? {
                return Err(save_error("不能改写已提交的工具调用"));
            }
            if let Some(position) = anchor.items[cursor..]
                .iter()
                .position(|item| item.message_id == history_id)
            {
                cursor += position + 1;
            }
            continue;
        }
        if let Some(position) = find_match(
            &anchor.items,
            cursor,
            &rows,
            &by_id,
            &request.messages[index],
            false,
        )? {
            let message_id = anchor.items[position].message_id.clone();
            request.messages[index].history_id = message_id;
            cursor = position + 1;
            continue;
        }
        if let Some(position) = find_match(
            &anchor.items,
            cursor,
            &rows,
            &by_id,
            &request.messages[index],
            true,
        )? {
            let message_id = anchor.items[position].message_id.clone();
            request.messages[index].history_id = message_id;
            cursor = position + 1;
            continue;
        }
        next_ordinal += 1;
        let turn_id = request.turn_id;
        let attempt_id = request.attempt_id;
        let row = insert_message(
            tx,
            session_record_id,
            &branch.id,
            turn_id,
            attempt_id,
            &request.messages[index],
            next_ordinal,
        )
        .await?;
        request.messages[index].history_id = row.id.clone();
        inserted.push(row.id.clone());
        by_id.insert(row.id.clone(), rows.len());
        rows.push(row);
    }

    let mut model_messages = Vec::new();
    for message in request
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
    {
        model_messages.push(message.clone());
    }
    sanitize_tool_message_pairs(&mut model_messages);
    let mut items = Vec::new();
    for message in &model_messages {
        let Some(raw_index) = by_id.get(&message.history_id).copied() else {
            return Err(save_error("历史投影引用了不存在的消息"));
        };
        items.push(ProjectionItem {
            message_id: message.history_id.clone(),
            context_override: override_for(&rows[raw_index], message)?,
        });
    }
    let mut retained = anchor.retained_message_ids.clone();
    for row in &rows {
        if row.branch_id == branch.id && !retained.iter().any(|id| id == &row.id) {
            retained.push(row.id.clone());
        }
    }
    for id in &inserted {
        if !retained.iter().any(|existing| existing == id) {
            retained.push(id.clone());
        }
    }
    let stored = StoredAnchor {
        format_version: FORMAT_VERSION,
        items: items.clone(),
        retained_message_ids: retained,
    };
    let projection_json =
        serde_json::to_string(&stored).map_err(|error| save_error(error.to_string()))?;
    let previous_json = serde_json::to_string(&StoredAnchor {
        format_version: FORMAT_VERSION,
        items: anchor.items.clone(),
        retained_message_ids: anchor.retained_message_ids.clone(),
    })
    .unwrap_or_default();
    let gaps_before = branch.gaps.clone();
    if had_images {
        merge_gap(
            &mut branch.gaps,
            gap("images", "图片未持久化，不能从历史还原"),
        );
    }
    if request.legacy_baseline && !branch.legacy_baseline {
        for item in legacy_gaps() {
            merge_gap(&mut branch.gaps, item);
        }
        branch.legacy_baseline = true;
    }
    let gaps_json =
        serde_json::to_string(&branch.gaps).map_err(|error| save_error(error.to_string()))?;
    let links = request.links;
    let links_added = write_links(tx, &branch.id, links, request, &rows, &by_id).await?;
    let changed = projection_json != previous_json
        || !inserted.is_empty()
        || links_added
        || branch.gaps != gaps_before;
    let revision = if changed {
        branch.revision + 1
    } else {
        branch.revision
    };
    if changed {
        let updated = sqlx::query(
            r#"
            UPDATE native_history_branches
            SET revision = $1, gaps_json = $2, legacy_baseline = $3, updated_at = $4
            WHERE id = $5 AND revision = $6 AND deleted_at IS NULL
            "#,
        )
        .bind(revision)
        .bind(&gaps_json)
        .bind(i64::from(branch.legacy_baseline))
        .bind(now_sqlite())
        .bind(&branch.id)
        .bind(branch.revision)
        .execute(&mut **tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
        if updated.rows_affected() != 1 {
            return Err(save_error("分支修订号不匹配"));
        }
        sqlx::query(
            r#"
            INSERT INTO native_context_anchors (
                branch_id, session_record_id, revision, projection_json, format_version, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(branch_id) DO UPDATE SET
                revision = excluded.revision,
                projection_json = excluded.projection_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&branch.id)
        .bind(session_record_id)
        .bind(revision)
        .bind(&projection_json)
        .bind(FORMAT_VERSION)
        .bind(now_sqlite())
        .execute(&mut **tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    }
    let message_ids: Vec<String> = request
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
        .map(|message| message.history_id.clone())
        .collect();
    upsert_transcript(tx, session_record_id, request, &model_messages).await?;
    if let Some(request_id) = nonempty(request.request_id) {
        let ids_json =
            serde_json::to_string(&message_ids).map_err(|error| save_error(error.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO native_history_requests (
                request_id, branch_id, revision, message_ids_json, created_at
            ) VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(request_id)
        .bind(&branch.id)
        .bind(revision)
        .bind(ids_json)
        .bind(now_sqlite())
        .execute(&mut **tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    }
    anchor.items = items;
    Ok(HistoryCommitReceipt {
        session_record_id: session_record_id.to_string(),
        branch_id: branch.id,
        revision,
        message_ids,
        changed,
    })
}

fn find_match(
    items: &[ProjectionItem],
    cursor: usize,
    rows: &[RawMessage],
    by_id: &HashMap<String, usize>,
    message: &Message,
    override_only: bool,
) -> Result<Option<usize>, String> {
    if override_only {
        let Some(item) = items.get(cursor) else {
            return Ok(None);
        };
        let Some(raw_index) = by_id.get(&item.message_id).copied() else {
            return Ok(None);
        };
        return Ok(compact_override(&rows[raw_index], message)?.then_some(cursor));
    }
    for (offset, item) in items.iter().enumerate().skip(cursor) {
        let Some(raw_index) = by_id.get(&item.message_id).copied() else {
            continue;
        };
        if exact_match(&rows[raw_index], message)? {
            return Ok(Some(offset));
        }
    }
    Ok(None)
}

async fn replay_request(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    request_id: Option<&str>,
) -> Result<Option<HistoryCommitReceipt>, String> {
    let Some(request_id) = nonempty(request_id) else {
        return Ok(None);
    };
    let row = sqlx::query(
        r#"
        SELECT r.branch_id, r.revision, r.message_ids_json, b.session_record_id, b.deleted_at
        FROM native_history_requests r
        JOIN native_history_branches b ON b.id = r.branch_id
        WHERE r.request_id = $1
        "#,
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.get::<String, _>("session_record_id") != session_record_id {
        return Err(save_error("幂等请求与会话不一致"));
    }
    if row.get::<Option<String>, _>("deleted_at").is_some() {
        return Err(save_error("幂等请求指向的分支已删除"));
    }
    let message_ids = serde_json::from_str(row.get("message_ids_json"))
        .map_err(|error| save_error(format!("解析幂等请求失败: {error}")))?;
    Ok(Some(HistoryCommitReceipt {
        session_record_id: session_record_id.to_string(),
        branch_id: row.get("branch_id"),
        revision: row.get("revision"),
        message_ids,
        changed: false,
    }))
}

async fn load_active_branch(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
) -> Result<Option<BranchRow>, String> {
    let row = sqlx::query(
        r#"
        SELECT id, revision, legacy_baseline, gaps_json, format_version
        FROM native_history_branches
        WHERE session_record_id = $1 AND active = 1 AND deleted_at IS NULL
        "#,
    )
    .bind(session_record_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let format_version: i64 = row.get("format_version");
    if format_version != FORMAT_VERSION {
        return Err(save_error(format!("不支持的历史格式版本 {format_version}")));
    }
    let gaps = serde_json::from_str(row.get("gaps_json"))
        .map_err(|error| save_error(format!("解析历史能力边界失败: {error}")))?;
    Ok(Some(BranchRow {
        id: row.get("id"),
        revision: row.get("revision"),
        legacy_baseline: row.get::<i64, _>("legacy_baseline") == 1,
        gaps,
        format_version,
    }))
}

async fn insert_branch(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    legacy_baseline: bool,
) -> Result<BranchRow, String> {
    let id = new_id();
    let now = now_sqlite();
    let gaps = if legacy_baseline {
        legacy_gaps()
    } else {
        Vec::new()
    };
    let gaps_json = serde_json::to_string(&gaps).map_err(|error| save_error(error.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO native_history_branches (
            id, session_record_id, revision, active, sealed, legacy_baseline,
            gaps_json, format_version, created_at, updated_at
        ) VALUES ($1, $2, 0, 1, 0, $3, $4, $5, $6, $6)
        "#,
    )
    .bind(&id)
    .bind(session_record_id)
    .bind(i64::from(legacy_baseline))
    .bind(&gaps_json)
    .bind(FORMAT_VERSION)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    Ok(BranchRow {
        id,
        revision: 0,
        legacy_baseline,
        gaps,
        format_version: FORMAT_VERSION,
    })
}

async fn load_anchor(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    branch_id: &str,
) -> Result<StoredAnchor, String> {
    let row = sqlx::query(
        "SELECT projection_json, format_version FROM native_context_anchors WHERE branch_id = $1",
    )
    .bind(branch_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let Some(row) = row else {
        return Ok(StoredAnchor {
            format_version: FORMAT_VERSION,
            items: Vec::new(),
            retained_message_ids: Vec::new(),
        });
    };
    let format_version: i64 = row.get("format_version");
    if format_version != FORMAT_VERSION {
        return Err(save_error(format!("不支持的历史格式版本 {format_version}")));
    }
    serde_json::from_str(row.get("projection_json"))
        .map_err(|error| save_error(format!("解析上下文锚点失败: {error}")))
}

async fn load_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    branch_id: &str,
    extra_ids: &[String],
) -> Result<Vec<RawMessage>, String> {
    let mut rows = query_messages(tx, "branch_id = $1", branch_id).await?;
    let known: HashSet<String> = rows.iter().map(|row| row.id.clone()).collect();
    for id in extra_ids {
        if known.contains(id) {
            continue;
        }
        rows.extend(query_messages(tx, "id = $1", id).await?);
    }
    rows.sort_by_key(|row| (row.ordinal, row.id.clone()));
    Ok(rows)
}

async fn query_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    predicate: &str,
    value: &str,
) -> Result<Vec<RawMessage>, String> {
    let sql = format!(
        r#"
        SELECT id, branch_id, turn_id, attempt_id, ordinal, role, content, tool_calls_json,
               tool_call_id, name, reasoning_content, images_unrecoverable, format_version
        FROM native_history_messages
        WHERE {predicate}
        "#
    );
    let fetched = sqlx::query(&sql)
        .bind(value)
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    fetched
        .into_iter()
        .map(|row| {
            let format_version: i64 = row.get("format_version");
            if format_version != FORMAT_VERSION {
                return Err(save_error(format!("不支持的历史格式版本 {format_version}")));
            }
            Ok(RawMessage {
                id: row.get("id"),
                branch_id: row.get("branch_id"),
                turn_id: row.get("turn_id"),
                attempt_id: row.get("attempt_id"),
                ordinal: row.get("ordinal"),
                role: row.get("role"),
                content: row.get("content"),
                tool_calls_json: row.get("tool_calls_json"),
                tool_call_id: row.get("tool_call_id"),
                name: row.get("name"),
                reasoning_content: row.get("reasoning_content"),
                images_unrecoverable: row.get::<i64, _>("images_unrecoverable") == 1,
                format_version,
            })
        })
        .collect()
}

fn images_lack_attachment_ref(message: &Message) -> bool {
    message
        .images
        .iter()
        .any(|image| image.attachment_id.is_empty())
}

fn uses_from_images(images: &[NativeImage]) -> Vec<AttachmentUse> {
    images
        .iter()
        .filter(|image| !image.attachment_id.is_empty())
        .map(|image| {
            if image.mime_type == "application/pdf" {
                AttachmentUse::Pdf {
                    attachment_id: image.attachment_id.clone(),
                    mode: PdfUseMode::Auto,
                    pages: None,
                }
            } else if image.mime_type.starts_with("video/") {
                AttachmentUse::Video {
                    attachment_id: image.attachment_id.clone(),
                }
            } else {
                AttachmentUse::Image {
                    attachment_id: image.attachment_id.clone(),
                }
            }
        })
        .collect()
}

async fn insert_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    branch_id: &str,
    turn_id: Option<&str>,
    attempt_id: Option<&str>,
    message: &Message,
    ordinal: i64,
) -> Result<RawMessage, String> {
    let id = new_id();
    let calls = tool_calls_json(message)?;
    let images_unrecoverable = images_lack_attachment_ref(message);
    sqlx::query(
        r#"
        INSERT INTO native_history_messages (
            id, session_record_id, branch_id, turn_id, attempt_id, ordinal, role, content,
            tool_calls_json, tool_call_id, name, reasoning_content, images_unrecoverable,
            format_version, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        "#,
    )
    .bind(&id)
    .bind(session_record_id)
    .bind(branch_id)
    .bind(nonempty(turn_id))
    .bind(nonempty(attempt_id))
    .bind(ordinal)
    .bind(role_name(message.role))
    .bind(&message.content)
    .bind(&calls)
    .bind(&message.tool_call_id)
    .bind(&message.name)
    .bind(&message.reasoning_content)
    .bind(i64::from(images_unrecoverable))
    .bind(FORMAT_VERSION)
    .bind(now_sqlite())
    .execute(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let media = if message.media.is_empty() {
        uses_from_images(&message.images)
    } else {
        message.media.clone()
    };
    if !media.is_empty() {
        crate::native::attachments::replace_owner_uses(tx, "message", &id, &media, &now_sqlite())
            .await
            .map_err(|error| save_error(error.to_string()))?;
    }
    Ok(RawMessage {
        id,
        branch_id: branch_id.to_string(),
        turn_id: nonempty(turn_id).map(ToOwned::to_owned),
        attempt_id: nonempty(attempt_id).map(ToOwned::to_owned),
        ordinal,
        role: role_name(message.role).to_string(),
        content: message.content.clone(),
        tool_calls_json: calls,
        tool_call_id: message.tool_call_id.clone(),
        name: message.name.clone(),
        reasoning_content: message.reasoning_content.clone(),
        images_unrecoverable,
        format_version: FORMAT_VERSION,
    })
}

async fn write_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    branch_id: &str,
    extra: &[HistoryLink],
    request: &HistoryWrite<'_>,
    rows: &[RawMessage],
    by_id: &HashMap<String, usize>,
) -> Result<bool, String> {
    let mut links = Vec::new();
    for row in rows.iter().filter(|row| row.branch_id == branch_id) {
        let message = row.to_message()?;
        for call in &message.tool_calls {
            if !call.id.is_empty() {
                links.push(HistoryLink {
                    message_id: row.id.clone(),
                    kind: "tool_call".to_string(),
                    target_id: call.id.clone(),
                });
            }
        }
        if message.role == Role::Tool && !message.tool_call_id.is_empty() {
            links.push(HistoryLink {
                message_id: row.id.clone(),
                kind: "tool_result".to_string(),
                target_id: message.tool_call_id.clone(),
            });
        }
        if let Some(attempt_id) = nonempty(request.attempt_id) {
            if row.attempt_id.as_deref() == Some(attempt_id) {
                links.push(HistoryLink {
                    message_id: row.id.clone(),
                    kind: "attempt".to_string(),
                    target_id: attempt_id.to_string(),
                });
            }
        }
    }
    for link in extra {
        if by_id.get(&link.message_id).is_none() {
            return Err(save_error(format!("关联的消息不存在: {}", link.message_id)));
        }
        links.push(link.clone());
    }
    let mut added = false;
    for link in links {
        let inserted = sqlx::query(
            r#"
            INSERT OR IGNORE INTO native_history_links (
                id, branch_id, message_id, link_kind, target_id, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(new_id())
        .bind(branch_id)
        .bind(&link.message_id)
        .bind(&link.kind)
        .bind(&link.target_id)
        .bind(now_sqlite())
        .execute(&mut **tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
        added = added || inserted.rows_affected() > 0;
    }
    Ok(added)
}

async fn upsert_transcript(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    request: &HistoryWrite<'_>,
    messages: &[Message],
) -> Result<(), String> {
    let mut stored = messages.to_vec();
    for message in &mut stored {
        message.images.clear();
    }
    let messages_json =
        serde_json::to_string(&stored).map_err(|error| save_error(error.to_string()))?;
    let now = now_sqlite();
    sqlx::query(
        r#"
        INSERT INTO native_session_transcripts (
            session_record_id, profile_id, workspace_id, model, turns,
            messages_json, created_at, updated_at, deleted_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $7, NULL)
        ON CONFLICT(session_record_id) DO UPDATE SET
            profile_id = excluded.profile_id,
            workspace_id = excluded.workspace_id,
            model = excluded.model,
            turns = excluded.turns,
            messages_json = excluded.messages_json,
            updated_at = excluded.updated_at,
            deleted_at = NULL
        "#,
    )
    .bind(session_record_id)
    .bind(request.profile_id)
    .bind(request.workspace_id)
    .bind(request.model)
    .bind(i64::from(request.turns))
    .bind(messages_json)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    Ok(())
}

pub async fn load_projection(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Option<Vec<Message>>, String> {
    let session_record_id = session_record_id.trim();
    if session_record_id.is_empty() {
        return Ok(None);
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| save_error(error.to_string()))?;
    let Some(branch) = load_active_branch(&mut tx, session_record_id).await? else {
        return Ok(None);
    };
    let anchor = load_anchor(&mut tx, &branch.id).await?;
    let rows = load_messages(&mut tx, &branch.id, &anchor.retained_message_ids).await?;
    let mut messages = Vec::new();
    for item in &anchor.items {
        let raw = rows
            .iter()
            .find(|row| row.id == item.message_id)
            .ok_or_else(|| save_error("历史投影引用了不存在的消息"))?;
        let mut message = raw.to_message()?;
        if let Some(override_value) = &item.context_override {
            message.content = override_value.content.clone();
            message.reasoning_content = override_value.reasoning_content.clone();
            message.tool_calls = override_value.tool_calls.clone();
        }
        if !message.history_id.is_empty() {
            message.media = crate::native::attachments::load_owner_uses_tx(
                &mut tx,
                "message",
                &message.history_id,
            )
            .await
            .map_err(|error| save_error(error.to_string()))?;
        }
        messages.push(message);
    }
    Ok(Some(messages))
}

pub async fn active_branch_id(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Option<String>, String> {
    let id = sqlx::query_scalar::<_, String>(
        r#"
        SELECT id FROM native_history_branches
        WHERE session_record_id = $1 AND active = 1 AND deleted_at IS NULL
        "#,
    )
    .bind(session_record_id.trim())
    .fetch_optional(pool)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    Ok(id)
}

pub async fn ensure_legacy_imported(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<bool, String> {
    let session_record_id = session_record_id.trim();
    if session_record_id.is_empty() || active_branch_id(pool, session_record_id).await?.is_some() {
        return Ok(false);
    }
    let row = sqlx::query(
        r#"
        SELECT profile_id, workspace_id, model, turns, messages_json
        FROM native_session_transcripts
        WHERE session_record_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let Some(row) = row else {
        return Ok(false);
    };
    let mut messages: Vec<Message> = serde_json::from_str(row.get("messages_json"))
        .map_err(|error| save_error(format!("解析旧会话上下文失败: {error}")))?;
    for message in &mut messages {
        message.history_id.clear();
        message.images.clear();
    }
    let model: String = row.get("model");
    let profile_id: Option<String> = row.get("profile_id");
    let workspace_id: Option<String> = row.get("workspace_id");
    let turns: i64 = row.get("turns");
    commit_model_context(
        pool,
        HistoryWrite {
            session_record_id,
            profile_id: profile_id.as_deref(),
            workspace_id: workspace_id.as_deref(),
            model: &model,
            turns: u32::try_from(turns).unwrap_or(0),
            messages: &mut messages,
            turn_id: None,
            attempt_id: None,
            expected_revision: None,
            request_id: None,
            links: &[],
            legacy_baseline: true,
        },
    )
    .await?;
    Ok(true)
}

pub async fn list_boundaries(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<HistoryBoundaries, String> {
    let session_record_id = session_record_id.trim();
    ensure_legacy_imported(pool, session_record_id).await?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| save_error(error.to_string()))?;
    let Some(branch) = load_active_branch(&mut tx, session_record_id).await? else {
        return Ok(HistoryBoundaries {
            session_record_id: session_record_id.to_string(),
            branch_id: String::new(),
            revision: 0,
            legacy_baseline: false,
            gaps: Vec::new(),
            boundaries: Vec::new(),
        });
    };
    let anchor = load_anchor(&mut tx, &branch.id).await?;
    let rows = load_messages(&mut tx, &branch.id, &anchor.retained_message_ids).await?;
    let ordered = order_retained(&rows, &anchor.retained_message_ids)?;
    let raw_messages = ordered
        .iter()
        .map(|row| row.to_message())
        .collect::<Result<Vec<_>, _>>()?;
    let flags = pair_flags(&raw_messages);
    let mut boundaries = Vec::new();
    for (row, flag) in ordered.iter().zip(flags) {
        boundaries.push(HistoryBoundary {
            message_id: row.id.clone(),
            ordinal: row.ordinal,
            role: row.role.clone(),
            turn_id: row.turn_id.clone(),
            selectable_before: flag.0,
            selectable_after: flag.1,
            preview: preview_text(&row.content),
        });
    }
    let mut gaps = branch.gaps;
    let checkpoint_ids = sqlx::query_scalar::<_, String>(
        r#"
        SELECT target_id FROM native_history_links
        WHERE branch_id = $1 AND link_kind = 'checkpoint'
        "#,
    )
    .bind(&branch.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    for checkpoint_id in checkpoint_ids {
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM git_checkpoints WHERE id = $1")
                .bind(&checkpoint_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| save_error(error.to_string()))?;
        if exists == 0 {
            merge_gap(
                &mut gaps,
                gap(
                    "checkpoints",
                    &format!("检查点 {checkpoint_id} 已不存在，不能自动关联"),
                ),
            );
        }
    }
    Ok(HistoryBoundaries {
        session_record_id: session_record_id.to_string(),
        branch_id: branch.id,
        revision: branch.revision,
        legacy_baseline: branch.legacy_baseline,
        gaps,
        boundaries,
    })
}

fn preview_text(content: &str) -> String {
    let mut chars = content.chars();
    let text: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}

fn rewind_already_applied(
    ordered: &[&RawMessage],
    retained: &[String],
    stored_boundary: Option<&str>,
    requested: &str,
    edge: BoundaryEdge,
) -> Result<bool, String> {
    let Some(position) = ordered.iter().position(|row| row.id == requested) else {
        return Ok(stored_boundary == Some(requested));
    };
    let messages = ordered
        .iter()
        .map(|row| row.to_message())
        .collect::<Result<Vec<_>, _>>()?;
    let flags = pair_flags(&messages);
    let end = match edge {
        BoundaryEdge::After => {
            if !flags[position].1 {
                return Ok(false);
            }
            position + 1
        }
        BoundaryEdge::Before => {
            if !flags[position].0 {
                return Ok(false);
            }
            position
        }
    };
    let cut = ordered[..end]
        .iter()
        .map(|row| row.id.as_str())
        .collect::<Vec<_>>();
    Ok(cut.len() == retained.len() && cut.iter().zip(retained).all(|(id, kept)| *id == kept))
}

fn cut_retained(
    ordered: &[&RawMessage],
    boundary_message_id: &str,
    edge: BoundaryEdge,
) -> Result<Vec<String>, String> {
    let position = ordered
        .iter()
        .position(|row| row.id == boundary_message_id)
        .ok_or_else(|| save_error("该消息不在当前分支"))?;
    let messages = ordered
        .iter()
        .map(|row| row.to_message())
        .collect::<Result<Vec<_>, _>>()?;
    let flags = pair_flags(&messages);
    let end = match edge {
        BoundaryEdge::After => {
            if !flags[position].1 {
                return Err(save_error("该位置会拆开工具调用"));
            }
            position + 1
        }
        BoundaryEdge::Before => {
            if !flags[position].0 {
                return Err(save_error("该位置会拆开工具调用"));
            }
            position
        }
    };
    Ok(ordered[..end].iter().map(|row| row.id.clone()).collect())
}

async fn detach_branch_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_record_id: &str,
    source_branch_id: &str,
) -> Result<(), String> {
    sqlx::query(
        r#"
        UPDATE agent_sessions
        SET pending_plan_json = NULL,
            approved_plan_json = CASE
                WHEN approved_plan_json IS NULL THEN NULL
                ELSE json_set(approved_plan_json, '$.status', 'cancelled')
            END
        WHERE id = $1
        "#,
    )
    .bind(session_record_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    sqlx::query(
        r#"
        UPDATE native_goals
        SET bound_branch_id = COALESCE(NULLIF(bound_branch_id, ''), $1)
        WHERE session_record_id = $2 AND status = 'completed'
        "#,
    )
    .bind(source_branch_id)
    .bind(session_record_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    Ok(())
}

fn order_retained<'a>(
    rows: &'a [RawMessage],
    retained: &[String],
) -> Result<Vec<&'a RawMessage>, String> {
    if retained.is_empty() {
        return Ok(rows.iter().collect());
    }
    let mut ordered = Vec::new();
    for id in retained {
        let row = rows
            .iter()
            .find(|row| row.id == *id)
            .ok_or_else(|| save_error("历史投影引用了不存在的消息"))?;
        ordered.push(row);
    }
    Ok(ordered)
}

pub struct BranchReference<'a> {
    pub new_session_id: &'a str,
    pub source_branch_id: &'a str,
    pub boundary_message_id: Option<&'a str>,
    pub profile_id: Option<&'a str>,
    pub workspace_id: Option<&'a str>,
    pub model: &'a str,
    pub turns: u32,
    pub edge: BoundaryEdge,
    pub expected_revision: Option<i64>,
    pub request_id: Option<&'a str>,
    /// 回退：封存当前活动分支并在同一会话建立新分支，不删除后续历史。
    pub seal_source: bool,
}

pub async fn create_referencing_branch(
    pool: &SqlitePool,
    reference: BranchReference<'_>,
) -> Result<HistoryCommitReceipt, String> {
    let BranchReference {
        new_session_id,
        source_branch_id,
        boundary_message_id,
        profile_id,
        workspace_id,
        model,
        turns,
        edge,
        expected_revision,
        request_id,
        seal_source,
    } = reference;
    let new_session_id = new_session_id.trim();
    if new_session_id.is_empty() {
        return Err(save_error("会话标识不能为空"));
    }
    let mut tx = begin_write(pool).await?;
    if let Some(receipt) = replay_request(&mut tx, new_session_id, request_id).await? {
        tx.commit()
            .await
            .map_err(|error| save_error(error.to_string()))?;
        return Ok(receipt);
    }
    let source = sqlx::query(
        r#"
        SELECT session_record_id, format_version, revision, active, boundary_message_id
        FROM native_history_branches
        WHERE id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(source_branch_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?
    .ok_or_else(|| save_error("来源分支不存在"))?;
    let format_version: i64 = source.get("format_version");
    if format_version != FORMAT_VERSION {
        return Err(save_error(format!("不支持的历史格式版本 {format_version}")));
    }
    let source_session_id: String = source.get("session_record_id");
    let source_revision: i64 = source.get("revision");
    if seal_source && source_session_id != new_session_id {
        return Err(save_error("回退必须保留当前会话"));
    }
    if seal_source && source.get::<i64, _>("active") != 1 {
        return Err(save_error("来源分支已不是活动分支"));
    }
    if !seal_source && load_active_branch(&mut tx, new_session_id).await?.is_some() {
        return Err(save_error("目标会话已有活动分支"));
    }
    let anchor = load_anchor(&mut tx, source_branch_id).await?;
    let rows = load_messages(&mut tx, source_branch_id, &anchor.retained_message_ids).await?;
    let ordered = order_retained(&rows, &anchor.retained_message_ids)?;
    let requested = nonempty(boundary_message_id);
    if seal_source {
        if let Some(requested) = requested {
            let stored_boundary = source.get::<Option<String>, _>("boundary_message_id");
            if rewind_already_applied(
                &ordered,
                &anchor.retained_message_ids,
                stored_boundary.as_deref(),
                requested,
                edge,
            )? {
                let message_ids = anchor
                    .items
                    .iter()
                    .map(|item| item.message_id.clone())
                    .collect();
                tx.commit()
                    .await
                    .map_err(|error| save_error(error.to_string()))?;
                return Ok(HistoryCommitReceipt {
                    session_record_id: new_session_id.to_string(),
                    branch_id: source_branch_id.to_string(),
                    revision: source_revision,
                    message_ids,
                    changed: false,
                });
            }
        }
    }
    if let Some(expected) = expected_revision {
        if expected != source_revision {
            return Err(save_error("分支修订号不匹配"));
        }
    }
    if seal_source {
        let sealed = sqlx::query(
            r#"
            UPDATE native_history_branches
            SET active = 0, sealed = 1, updated_at = $1
            WHERE id = $2 AND session_record_id = $3 AND active = 1 AND deleted_at IS NULL
            "#,
        )
        .bind(now_sqlite())
        .bind(source_branch_id)
        .bind(new_session_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
        if sealed.rows_affected() != 1 {
            return Err(save_error("来源分支已不是活动分支"));
        }
    }
    let retained_ids = match requested {
        Some(boundary_message_id) => cut_retained(&ordered, boundary_message_id, edge)?,
        None => ordered.iter().map(|row| row.id.clone()).collect(),
    };
    let retained_set: HashSet<&str> = retained_ids.iter().map(String::as_str).collect();
    let items = if boundary_message_id.is_some() {
        let prefix = ordered
            .iter()
            .filter(|row| retained_set.contains(row.id.as_str()))
            .map(|row| row.to_message())
            .collect::<Result<Vec<_>, _>>()?;
        let mut sanitized = prefix;
        sanitize_tool_message_pairs(&mut sanitized);
        sanitized
            .into_iter()
            .map(|message| ProjectionItem {
                message_id: message.history_id,
                context_override: None,
            })
            .collect()
    } else {
        anchor
            .items
            .into_iter()
            .filter(|item| retained_set.contains(item.message_id.as_str()))
            .collect()
    };
    let branch_id = new_id();
    let now = now_sqlite();
    let stored = StoredAnchor {
        format_version: FORMAT_VERSION,
        items,
        retained_message_ids: retained_ids,
    };
    let projection_json =
        serde_json::to_string(&stored).map_err(|error| save_error(error.to_string()))?;
    let boundary = nonempty(boundary_message_id)
        .map(ToOwned::to_owned)
        .or_else(|| stored.retained_message_ids.last().cloned());
    sqlx::query(
        r#"
        INSERT INTO native_history_branches (
            id, session_record_id, source_session_id, source_branch_id, boundary_message_id,
            revision, active, sealed, legacy_baseline, gaps_json, format_version, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, 1, 1, 0, 0, '[]', $6, $7, $7)
        "#,
    )
    .bind(&branch_id)
    .bind(new_session_id)
    .bind(&source_session_id)
    .bind(source_branch_id)
    .bind(&boundary)
    .bind(FORMAT_VERSION)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO native_context_anchors (
            branch_id, session_record_id, revision, projection_json, format_version, updated_at
        ) VALUES ($1, $2, 1, $3, $4, $5)
        "#,
    )
    .bind(&branch_id)
    .bind(new_session_id)
    .bind(&projection_json)
    .bind(FORMAT_VERSION)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let source_links = sqlx::query(
        "SELECT message_id, link_kind, target_id FROM native_history_links WHERE branch_id = $1",
    )
    .bind(source_branch_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    for link in source_links {
        let message_id: String = link.get("message_id");
        if !stored
            .retained_message_ids
            .iter()
            .any(|id| id == &message_id)
        {
            continue;
        }
        sqlx::query(
            r#"
            INSERT INTO native_history_links (
                id, branch_id, message_id, link_kind, target_id, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(new_id())
        .bind(&branch_id)
        .bind(&message_id)
        .bind(link.get::<String, _>("link_kind"))
        .bind(link.get::<String, _>("target_id"))
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    }
    let mut projected = Vec::new();
    for item in &stored.items {
        let raw = rows
            .iter()
            .find(|row| row.id == item.message_id)
            .ok_or_else(|| save_error("历史投影引用了不存在的消息"))?;
        let mut message = raw.to_message()?;
        if let Some(override_value) = &item.context_override {
            message.content = override_value.content.clone();
            message.reasoning_content = override_value.reasoning_content.clone();
            message.tool_calls = override_value.tool_calls.clone();
        }
        projected.push(message);
    }
    let write = HistoryWrite {
        session_record_id: new_session_id,
        profile_id,
        workspace_id,
        model,
        turns,
        messages: &mut [],
        turn_id: None,
        attempt_id: None,
        expected_revision: None,
        request_id: None,
        links: &[],
        legacy_baseline: false,
    };
    upsert_transcript(&mut tx, new_session_id, &write, &projected).await?;
    if seal_source {
        detach_branch_state(&mut tx, new_session_id, source_branch_id).await?;
    }
    let message_ids: Vec<String> = stored
        .items
        .iter()
        .map(|item| item.message_id.clone())
        .collect();
    if let Some(request_id) = nonempty(request_id) {
        let ids_json =
            serde_json::to_string(&message_ids).map_err(|error| save_error(error.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO native_history_requests (
                request_id, branch_id, revision, message_ids_json, created_at
            ) VALUES ($1, $2, 1, $3, $4)
            "#,
        )
        .bind(request_id)
        .bind(&branch_id)
        .bind(ids_json)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|error| save_error(error.to_string()))?;
    Ok(HistoryCommitReceipt {
        session_record_id: new_session_id.to_string(),
        branch_id,
        revision: 1,
        message_ids,
        changed: true,
    })
}

pub async fn delete_history_branch(pool: &SqlitePool, branch_id: &str) -> Result<(), String> {
    let updated = sqlx::query(
        r#"
        UPDATE native_history_branches
        SET deleted_at = $1, active = 0, updated_at = $1
        WHERE id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(now_sqlite())
    .bind(branch_id)
    .execute(pool)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    if updated.rows_affected() != 1 {
        return Err(save_error("分支不存在"));
    }
    Ok(())
}

pub async fn purge_unreferenced_history(pool: &SqlitePool, cutoff: &str) -> Result<u64, String> {
    let mut tx = begin_write(pool).await?;
    let live_branches = sqlx::query(
        "SELECT id, boundary_message_id FROM native_history_branches WHERE deleted_at IS NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| save_error(error.to_string()))?;
    let mut protected = HashSet::new();
    let mut live_ids = HashSet::new();
    for branch in &live_branches {
        let id: String = branch.get("id");
        live_ids.insert(id.clone());
        if let Some(boundary) = branch.get::<Option<String>, _>("boundary_message_id") {
            protected.insert(boundary);
        }
        let anchor = load_anchor(&mut tx, &id).await?;
        for item in anchor.items {
            protected.insert(item.message_id);
        }
        protected.extend(anchor.retained_message_ids);
    }
    let messages = sqlx::query("SELECT id, branch_id, created_at FROM native_history_messages")
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| save_error(error.to_string()))?;
    let mut removed = Vec::new();
    for message in messages {
        let id: String = message.get("id");
        let branch_id: String = message.get("branch_id");
        let created_at: String = message.get("created_at");
        if created_at.as_str() < cutoff
            && !protected.contains(&id)
            && !live_ids.contains(&branch_id)
        {
            removed.push(id);
        }
    }
    for id in &removed {
        sqlx::query("DELETE FROM native_history_messages WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|error| save_error(error.to_string()))?;
        sqlx::query("DELETE FROM native_history_links WHERE message_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|error| save_error(error.to_string()))?;
    }
    let count = removed.len() as u64;
    tx.commit()
        .await
        .map_err(|error| save_error(error.to_string()))?;
    Ok(count)
}

#[tauri::command]
pub async fn list_native_history_boundaries(
    app: tauri::AppHandle,
    session_record_id: String,
) -> Result<HistoryBoundaries, String> {
    let pool = crate::app::shared::sqlite_pool(&app).await?;
    list_boundaries(&pool, &session_record_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::agent::compact::compact_local;
    use crate::native::model::types::{Message, NativeImage, Role, ToolCall};
    use sqlx::SqlitePool;

    fn write<'a>(
        session_record_id: &'a str,
        messages: &'a mut [Message],
        expected_revision: Option<i64>,
        request_id: Option<&'a str>,
        links: &'a [HistoryLink],
    ) -> HistoryWrite<'a> {
        HistoryWrite {
            session_record_id,
            profile_id: None,
            workspace_id: None,
            model: "test-model",
            turns: 1,
            messages,
            turn_id: Some("turn-1"),
            attempt_id: None,
            expected_revision,
            request_id,
            links,
            legacy_baseline: false,
        }
    }

    fn assistant_call(id: &str) -> Message {
        let mut message = Message::assistant_text("");
        message.tool_calls = vec![ToolCall {
            id: id.to_string(),
            name: "Read".to_string(),
            arguments: "{}".to_string(),
        }];
        message
    }

    async fn message_count(pool: &SqlitePool, session_id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(1) FROM native_history_messages WHERE session_record_id = $1",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn restart_keeps_identity_after_compaction() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![
            Message::user("turn one"),
            assistant_call("call_a"),
            Message::tool_result("call_a", "UNIQUE_TOOL_RESULT_7f3a"),
            Message::user("turn two"),
            Message::assistant_text("answer two"),
        ];
        let first = commit_model_context(&pool, write("sess", &mut messages, Some(0), None, &[]))
            .await
            .unwrap();
        assert!(first.changed);
        let preserved_user = messages[3].history_id.clone();
        let preserved_answer = messages[4].history_id.clone();
        let tool_id = messages[2].history_id.clone();
        assert!(compact_local(&mut messages));
        let second = commit_model_context(&pool, write("sess", &mut messages, Some(1), None, &[]))
            .await
            .unwrap();
        assert!(second.changed);
        let projection = load_projection(&pool, "sess").await.unwrap().unwrap();
        assert!(projection
            .iter()
            .any(|message| message.history_id == preserved_user));
        assert!(projection
            .iter()
            .any(|message| message.history_id == preserved_answer));
        assert!(!projection
            .iter()
            .any(|message| message.history_id == tool_id));
        let raw: String =
            sqlx::query_scalar("SELECT content FROM native_history_messages WHERE id = $1")
                .bind(&tool_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(raw, "UNIQUE_TOOL_RESULT_7f3a");
        assert_eq!(messages[1].history_id, preserved_user);
    }

    #[tokio::test]
    async fn storage_failure_does_not_publish_or_advance() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![Message::user("hello")];
        let mut published = Vec::new();
        let receipt = commit_model_context(&pool, write("sess", &mut messages, Some(0), None, &[]))
            .await
            .unwrap();
        if receipt.changed {
            published.push(receipt.revision);
        }
        let mut again = messages.clone();
        again[0].content = "changed".to_string();
        let error = commit_model_context(&pool, write("sess", &mut again, Some(0), None, &[]))
            .await
            .unwrap_err();
        assert!(error.contains("分支修订号不匹配"));
        assert_eq!(published, vec![1]);
        let revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM native_history_branches WHERE session_record_id = 'sess'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(revision, 1);
        assert_eq!(message_count(&pool, "sess").await, 1);
        pool.close().await;
        let closed = commit_model_context(&pool, write("sess", &mut again, None, None, &[]))
            .await
            .unwrap_err();
        assert!(closed.contains("保存会话历史失败"));
        assert_eq!(published.len(), 1);
    }

    #[tokio::test]
    async fn legacy_transcript_marks_unrecoverable_gaps_without_guessing() {
        let pool = setup_migrated_pool().await;
        sqlx::query(
            r#"
            INSERT INTO native_session_transcripts (
                session_record_id, model, turns, messages_json, created_at, updated_at
            ) VALUES ('old', 'm', 1, $1, '2020-01-01 00:00:00', '2020-01-01 00:00:00')
            "#,
        )
        .bind(
            serde_json::to_string(&vec![
                Message::user("kept"),
                Message::assistant_text("answer"),
            ])
            .unwrap(),
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO workspaces (id, name) VALUES ('ws', 'ws')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agent_sessions (id, workspace_id) VALUES ('old', 'ws')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            r#"
            INSERT INTO git_checkpoints (
                id, session_id, workspace_id, seq, ref_name, commit_oid
            ) VALUES ('cp-old', 'old', 'ws', 1, 'refs/noxcode/checkpoints/old', 'abc')
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        let loaded = crate::native::transcript::load_transcript(&pool, "old")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "kept");
        let view = list_boundaries(&pool, "old").await.unwrap();
        assert!(view.legacy_baseline);
        for kind in ["prior_history", "images", "checkpoints"] {
            assert!(
                view.gaps
                    .iter()
                    .any(|gap| gap.kind == kind && gap.status == "unrecoverable"),
                "missing {kind}"
            );
        }
        let links: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM native_history_links WHERE link_kind = 'checkpoint'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(links, 0);
    }

    #[tokio::test]
    async fn referenced_history_survives_source_delete_and_purge() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![
            Message::user("one"),
            Message::user("two"),
            Message::user("three"),
        ];
        commit_model_context(&pool, write("source", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let source_branch = active_branch_id(&pool, "source").await.unwrap().unwrap();
        create_referencing_branch(
            &pool,
            BranchReference {
                new_session_id: "fork",
                source_branch_id: &source_branch,
                boundary_message_id: Some(&messages[1].history_id),
                profile_id: None,
                workspace_id: None,
                model: "m",
                turns: 1,
                edge: BoundaryEdge::After,
                expected_revision: None,
                request_id: None,
                seal_source: false,
            },
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO agent_sessions (id) VALUES ('source')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM agent_sessions WHERE id = 'source'")
            .execute(&pool)
            .await
            .unwrap();
        let projection = load_projection(&pool, "fork").await.unwrap().unwrap();
        assert_eq!(projection.len(), 2);
        assert_eq!(projection[1].history_id, messages[1].history_id);
        delete_history_branch(&pool, &source_branch).await.unwrap();
        sqlx::query("UPDATE native_history_messages SET created_at = '2000-01-01 00:00:00'")
            .execute(&pool)
            .await
            .unwrap();
        let removed = purge_unreferenced_history(&pool, "2020-01-01 00:00:00")
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let surviving: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM native_history_messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(surviving, 2);
        let still = load_projection(&pool, "fork").await.unwrap().unwrap();
        assert_eq!(still.len(), 2);
    }

    fn reference<'a>(
        new_session_id: &'a str,
        source_branch_id: &'a str,
        boundary_message_id: Option<&'a str>,
        edge: BoundaryEdge,
        expected_revision: Option<i64>,
        request_id: Option<&'a str>,
        seal_source: bool,
    ) -> BranchReference<'a> {
        BranchReference {
            new_session_id,
            source_branch_id,
            boundary_message_id,
            profile_id: None,
            workspace_id: None,
            model: "m",
            turns: 1,
            edge,
            expected_revision,
            request_id,
            seal_source,
        }
    }

    #[tokio::test]
    async fn rewind_seals_the_old_branch_and_keeps_later_history() {
        let pool = setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id, pending_plan_json, approved_plan_json) VALUES ('sess', '{\"request_id\":\"req\",\"plan\":\"正文\",\"created_at\":\"t\"}', '{\"authorization_id\":\"a\",\"request_id\":\"req\",\"body\":\"正文\",\"feedback\":\"\",\"cwd\":\"/tmp\",\"path\":\"/tmp/p.md\",\"cwd_resolved\":true,\"content_hash\":\"h\",\"saved_hash\":\"h\",\"status\":\"saved\",\"ai_channel_id\":\"c\",\"model\":\"m\"}')")
            .execute(&pool)
            .await
            .unwrap();
        let mut messages = vec![
            Message::user("one"),
            Message::user("two"),
            Message::user("three"),
        ];
        let committed = commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        sqlx::query("INSERT INTO native_goals (id, session_record_id, title, status, progress_json, bound_branch_id) VALUES ('goal', 'sess', '完成登录', 'completed', '[]', $1)")
            .bind(&committed.branch_id)
            .execute(&pool)
            .await
            .unwrap();
        let later = messages[2].history_id.clone();
        let receipt = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &committed.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(committed.revision),
                Some("rewind-1"),
                true,
            ),
        )
        .await
        .unwrap();
        assert_ne!(receipt.branch_id, committed.branch_id);
        assert_eq!(receipt.message_ids, vec![messages[0].history_id.clone()]);
        let sealed: (i64, i64) =
            sqlx::query_as("SELECT active, sealed FROM native_history_branches WHERE id = $1")
                .bind(&committed.branch_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sealed, (0, 1));
        let still: i64 =
            sqlx::query_scalar("SELECT COUNT(1) FROM native_history_messages WHERE id = $1")
                .bind(&later)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(still, 1);
        let projection = load_projection(&pool, "sess").await.unwrap().unwrap();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].content, "one");
        let stale = commit_model_context(&pool, write("sess", &mut messages, Some(0), None, &[]))
            .await
            .unwrap_err();
        assert!(stale.contains("分支修订号不匹配"));
        let again = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &committed.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(committed.revision),
                Some("rewind-1"),
                true,
            ),
        )
        .await
        .unwrap();
        assert_eq!(again.branch_id, receipt.branch_id);
        assert!(!again.changed);
        let pending: Option<String> =
            sqlx::query_scalar("SELECT pending_plan_json FROM agent_sessions WHERE id = 'sess'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(pending.is_none());
        let approved: String =
            sqlx::query_scalar("SELECT approved_plan_json FROM agent_sessions WHERE id = 'sess'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(approved.contains("正文"));
        assert!(approved.contains("cancelled"));
        let stored_goal: String =
            sqlx::query_scalar("SELECT status FROM native_goals WHERE id = 'goal'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_goal, "completed");
        let visible = crate::native::goals::current_goal(&pool, "sess")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(visible.status, "completed");
        assert!(!visible.counts_as_complete(Some(receipt.branch_id.as_str())));
    }

    #[tokio::test]
    async fn fork_before_message_omits_it_and_keeps_the_source_branch() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![Message::user("keep"), Message::user("drop")];
        let committed =
            commit_model_context(&pool, write("source", &mut messages, None, None, &[]))
                .await
                .unwrap();
        let forked = create_referencing_branch(
            &pool,
            reference(
                "fork",
                &committed.branch_id,
                Some(&messages[1].history_id),
                BoundaryEdge::Before,
                Some(committed.revision),
                Some("fork-1"),
                false,
            ),
        )
        .await
        .unwrap();
        let projection = load_projection(&pool, "fork").await.unwrap().unwrap();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].content, "keep");
        let source_active: i64 =
            sqlx::query_scalar("SELECT active FROM native_history_branches WHERE id = $1")
                .bind(&committed.branch_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(source_active, 1);
        let traced: (String, String) = sqlx::query_as(
            "SELECT source_branch_id, boundary_message_id FROM native_history_branches WHERE id = $1",
        )
        .bind(&forked.branch_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(traced.0, committed.branch_id);
        assert_eq!(traced.1, messages[1].history_id);
        let split = create_referencing_branch(
            &pool,
            reference(
                "bad",
                &committed.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(99),
                None,
                false,
            ),
        )
        .await
        .unwrap_err();
        assert!(split.contains("分支修订号不匹配"));
    }

    #[tokio::test]
    async fn rewind_rejects_a_split_tool_pair() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![
            Message::user("go"),
            assistant_call("call_b"),
            Message::tool_result("call_b", "ok"),
        ];
        let committed = commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let error = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &committed.branch_id,
                Some(&messages[1].history_id),
                BoundaryEdge::After,
                Some(committed.revision),
                Some("split"),
                true,
            ),
        )
        .await
        .unwrap_err();
        assert!(error.contains("拆开工具调用"));
        let active = active_branch_id(&pool, "sess").await.unwrap().unwrap();
        assert_eq!(active, committed.branch_id);
    }

    #[tokio::test]
    async fn orphan_tool_call_stays_in_history_but_not_in_projection() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![Message::user("分析项目"), assistant_call("call_open")];
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let call_id = messages[1].history_id.clone();
        let projection = load_projection(&pool, "sess").await.unwrap().unwrap();
        assert!(projection
            .iter()
            .all(|message| message.tool_calls.is_empty()));
        let raw: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM native_history_messages WHERE id = $1 AND content = ''",
        )
        .bind(&call_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(raw, 1);
        let again = commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        assert!(!again.changed);
        assert_eq!(message_count(&pool, "sess").await, 2);
    }

    #[tokio::test]
    async fn idempotent_request_does_not_append_twice() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![Message::user("once")];
        let first = commit_model_context(
            &pool,
            write("sess", &mut messages, None, Some("req-1"), &[]),
        )
        .await
        .unwrap();
        messages.push(Message::user("second"));
        let replay = commit_model_context(
            &pool,
            write("sess", &mut messages, Some(99), Some("req-1"), &[]),
        )
        .await
        .unwrap();
        assert!(!replay.changed);
        assert_eq!(replay.revision, first.revision);
        assert_eq!(message_count(&pool, "sess").await, 1);
    }

    #[tokio::test]
    async fn tool_pair_boundary_is_not_selectable_and_missing_checkpoint_is_explicit() {
        let pool = setup_migrated_pool().await;
        sqlx::query("INSERT INTO workspaces (id, name) VALUES ('ws', 'ws')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agent_sessions (id, workspace_id) VALUES ('sess', 'ws')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            r#"
            INSERT INTO git_checkpoints (
                id, session_id, workspace_id, seq, ref_name, commit_oid
            ) VALUES ('cp-1', 'sess', 'ws', 1, 'refs/noxcode/checkpoints/1', 'abc')
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut messages = vec![
            Message::user("go"),
            assistant_call("call_b"),
            Message::tool_result("call_b", "ok"),
        ];
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let boundaries = list_boundaries(&pool, "sess").await.unwrap();
        let call = boundaries
            .boundaries
            .iter()
            .find(|item| item.role == "assistant")
            .unwrap();
        assert!(!call.selectable_after);
        let result = boundaries
            .boundaries
            .iter()
            .find(|item| item.role == "tool")
            .unwrap();
        assert!(result.selectable_after);
        let link = HistoryLink {
            message_id: messages[2].history_id.clone(),
            kind: "checkpoint".to_string(),
            target_id: "cp-1".to_string(),
        };
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[link]))
            .await
            .unwrap();
        sqlx::query("DELETE FROM agent_sessions WHERE id = 'sess'")
            .execute(&pool)
            .await
            .unwrap();
        let after = list_boundaries(&pool, "sess").await.unwrap();
        assert!(after.gaps.iter().any(|gap| {
            gap.kind == "checkpoints"
                && gap.reason.contains("cp-1")
                && gap.status == "unrecoverable"
        }));
        assert_eq!(after.boundaries.len(), 3);
    }

    #[tokio::test]
    async fn image_bytes_are_not_stored_and_are_marked_unrecoverable() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![Message::user_with_images(
            "look",
            vec![NativeImage {
                name: "a.png".to_string(),
                mime_type: "image/png".to_string(),
                data_base64: "AAAA".to_string(),
                attachment_id: String::new(),
                page: None,
                time_range: None,
            }],
        )];
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let stored: (String, i64) = sqlx::query_as(
            "SELECT content, images_unrecoverable FROM native_history_messages WHERE session_record_id = 'sess'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.0, "look");
        assert_eq!(stored.1, 1);
        let blob: String = sqlx::query_scalar(
            "SELECT content || tool_calls_json || reasoning_content FROM native_history_messages",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!blob.contains("AAAA"));
        let view = list_boundaries(&pool, "sess").await.unwrap();
        assert!(view
            .gaps
            .iter()
            .any(|gap| gap.kind == "images" && gap.status == "unrecoverable"));
    }

    #[tokio::test]
    async fn microcompact_override_keeps_raw_tool_result() {
        let pool = setup_migrated_pool().await;
        let mut messages = vec![
            Message::user("read"),
            assistant_call("call_c"),
            Message::tool_result("call_c", "full tool output"),
        ];
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let tool_id = messages[2].history_id.clone();
        messages[2].content = "stub".to_string();
        commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let raw: String =
            sqlx::query_scalar("SELECT content FROM native_history_messages WHERE id = $1")
                .bind(&tool_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(raw, "full tool output");
        let projection = load_projection(&pool, "sess").await.unwrap().unwrap();
        let projected = projection
            .iter()
            .find(|message| message.history_id == tool_id)
            .unwrap();
        assert_eq!(projected.content, "stub");
        assert_eq!(projected.role, Role::Tool);
    }

    #[tokio::test]
    async fn repeated_rewind_stays_on_the_current_boundary() {
        let pool = setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id) VALUES ('sess')")
            .execute(&pool)
            .await
            .unwrap();
        let mut messages = vec![Message::user("one"), Message::user("two")];
        let committed = commit_model_context(&pool, write("sess", &mut messages, None, None, &[]))
            .await
            .unwrap();
        let first = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &committed.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(committed.revision),
                Some("rw-1"),
                true,
            ),
        )
        .await
        .unwrap();
        assert!(first.changed);
        let second = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &first.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(first.revision),
                Some("rw-2"),
                true,
            ),
        )
        .await
        .unwrap();
        assert!(!second.changed);
        assert_eq!(second.branch_id, first.branch_id);
        let branches: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM native_history_branches WHERE session_record_id = 'sess'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(branches, 2);
        let before = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &first.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::Before,
                Some(first.revision),
                Some("rw-before"),
                true,
            ),
        )
        .await
        .unwrap();
        assert!(before.changed);
        assert!(load_projection(&pool, "sess")
            .await
            .unwrap()
            .unwrap()
            .is_empty());
        let again = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &before.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::Before,
                Some(before.revision),
                Some("rw-before-2"),
                true,
            ),
        )
        .await
        .unwrap();
        assert!(!again.changed);
        assert_eq!(again.branch_id, before.branch_id);
        let keep = create_referencing_branch(
            &pool,
            reference(
                "sess",
                &before.branch_id,
                Some(&messages[0].history_id),
                BoundaryEdge::After,
                Some(before.revision),
                Some("rw-after-gone"),
                true,
            ),
        )
        .await
        .unwrap();
        assert!(!keep.changed);
        assert_eq!(keep.branch_id, before.branch_id);
        let branches_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM native_history_branches WHERE session_record_id = 'sess'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(branches_after, 3);
    }

    #[tokio::test]
    async fn fork_reloads_attachment_bytes_and_missing_file_is_diagnosed() {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine;

        let dir = tempfile::tempdir().unwrap();
        let pool = setup_migrated_pool().await;
        let service = crate::native::attachments::AttachmentService::new(
            dir.path().to_path_buf(),
            pool.clone(),
            "composer",
        );
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([4, 5, 6]),
        ));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        let bytes = cursor.into_inner();
        let stored = service
            .import_bytes("shot.png", &bytes, "composer")
            .await
            .unwrap();
        let encoded = BASE64.encode(&bytes);
        let mut messages = vec![Message::user_with_images(
            "look",
            vec![NativeImage {
                name: "shot.png".to_string(),
                mime_type: "image/png".to_string(),
                data_base64: encoded.clone(),
                attachment_id: stored.id.clone(),
                page: None,
                time_range: None,
            }],
        )];
        let committed =
            commit_model_context(&pool, write("source", &mut messages, None, None, &[]))
                .await
                .unwrap();
        service.release_draft("composer").await.unwrap();
        let flag: i64 = sqlx::query_scalar(
            "SELECT images_unrecoverable FROM native_history_messages WHERE session_record_id = 'source'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(flag, 0);
        let stored_text: String = sqlx::query_scalar(
            "SELECT content FROM native_history_messages WHERE session_record_id = 'source'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!stored_text.contains(&encoded));
        let gaps = list_boundaries(&pool, "source").await.unwrap();
        assert!(gaps.gaps.iter().all(|gap| gap.kind != "images"));

        let mut resumed = load_projection(&pool, "source").await.unwrap().unwrap();
        assert_eq!(resumed[0].media.len(), 1);
        assert!(resumed[0].images.is_empty());
        crate::native::images::hydrate_message_media(&pool, dir.path(), &mut resumed)
            .await
            .unwrap();
        assert_eq!(
            BASE64
                .decode(resumed[0].images[0].data_base64.trim())
                .unwrap(),
            bytes
        );

        create_referencing_branch(
            &pool,
            reference(
                "fork",
                &committed.branch_id,
                None,
                BoundaryEdge::After,
                Some(committed.revision),
                Some("fork-media"),
                false,
            ),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE native_history_branches SET deleted_at = $1 WHERE id = $2")
            .bind(crate::app::shared::now_sqlite())
            .bind(&committed.branch_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(service.gc().await.unwrap().is_empty());
        let relative: String =
            sqlx::query_scalar("SELECT relative_path FROM native_attachments WHERE id = $1")
                .bind(&stored.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(dir.path().join(&relative).is_file());

        let mut forked = load_projection(&pool, "fork").await.unwrap().unwrap();
        assert_eq!(forked[0].media[0].attachment_id(), stored.id);
        crate::native::images::hydrate_message_media(&pool, dir.path(), &mut forked)
            .await
            .unwrap();
        assert_eq!(
            BASE64
                .decode(forked[0].images[0].data_base64.trim())
                .unwrap(),
            bytes
        );

        std::fs::remove_file(dir.path().join(relative)).unwrap();
        let mut missing = load_projection(&pool, "fork").await.unwrap().unwrap();
        crate::native::images::hydrate_message_media(&pool, dir.path(), &mut missing)
            .await
            .unwrap();
        assert!(missing[0].images.is_empty());
        assert!(missing[0].content.contains("[附件缺失]"));
        assert!(missing[0].content.contains(&stored.id));
        assert!(!missing[0].content.contains(&encoded));
    }

    #[tokio::test]
    async fn commit_survives_a_writer_that_commits_during_the_transaction() {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        use std::str::FromStr;
        use std::time::{Duration, Instant};

        let path = std::env::temp_dir().join(format!(
            "noxcode-history-lock-{}-{}.db",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .unwrap()
            .create_if_missing(true)
            .busy_timeout(Duration::from_millis(300))
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        for migration in crate::db::migrations::get_all_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }

        let mut holder = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *holder)
            .await
            .unwrap();
        let worker = pool.clone();
        let task = tokio::spawn(async move {
            let mut messages = vec![Message::user("while locked")];
            commit_model_context(&worker, write("sess-lock", &mut messages, None, None, &[])).await
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        sqlx::query("COMMIT").execute(&mut *holder).await.unwrap();
        drop(holder);

        let receipt = task
            .await
            .unwrap()
            .expect("history commit should wait out the other writer");
        assert!(receipt.changed);
        let loaded = load_projection(&pool, "sess-lock").await.unwrap().unwrap();
        assert_eq!(loaded[0].content, "while locked");

        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
