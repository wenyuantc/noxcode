#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use sqlx::SqlitePool;

use crate::app::shared::now_sqlite;
use crate::native::agent::truncate::sanitize_tool_message_pairs;
use crate::native::model::types::{Message, Role};

#[derive(Debug, Clone)]
pub struct NativeTranscriptMeta {
    pub profile_id: Option<String>,
    pub workspace_id: Option<String>,
    pub model: String,
    pub turns: u32,
}

pub fn prepare_transcript_messages(messages: &[Message]) -> Vec<Message> {
    let mut prepared: Vec<Message> = messages
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .map(|mut message| {
            message.images.clear();
            message
        })
        .collect();
    sanitize_tool_message_pairs(&mut prepared);
    prepared
}

pub fn transcript_fingerprint(messages: &[Message]) -> u64 {
    let prepared = prepare_transcript_messages(messages);
    let json = serde_json::to_string(&prepared).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    hasher.finish()
}

pub async fn save_transcript(
    pool: &SqlitePool,
    session_record_id: &str,
    messages: &[Message],
    meta: &NativeTranscriptMeta,
) -> Result<(), String> {
    if session_record_id.trim().is_empty() {
        return Err("会话标识不能为空".to_string());
    }
    let prepared = prepare_transcript_messages(messages);
    let messages_json = serde_json::to_string(&prepared)
        .map_err(|error| format!("序列化会话上下文失败: {error}"))?;
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
    .bind(meta.profile_id.as_deref())
    .bind(meta.workspace_id.as_deref())
    .bind(&meta.model)
    .bind(i64::from(meta.turns))
    .bind(&messages_json)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("保存会话上下文失败: {error}"))?;
    Ok(())
}

pub async fn load_transcript(
    pool: &SqlitePool,
    resume_session_id: &str,
) -> Result<Option<Vec<Message>>, String> {
    let id = resume_session_id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    let row = sqlx::query_as::<_, (String,)>(
        r#"
        SELECT messages_json
        FROM native_session_transcripts
        WHERE session_record_id = $1 AND deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话上下文失败: {error}"))?;
    let Some((messages_json,)) = row else {
        return Ok(None);
    };
    let mut messages: Vec<Message> = serde_json::from_str(&messages_json)
        .map_err(|error| format!("解析会话上下文失败: {error}"))?;
    for message in &mut messages {
        message.images.clear();
    }
    sanitize_tool_message_pairs(&mut messages);
    if messages.is_empty() {
        Ok(None)
    } else {
        Ok(Some(messages))
    }
}

pub async fn has_transcript(pool: &SqlitePool, session_record_id: &str) -> Result<bool, String> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(1)
        FROM native_session_transcripts
        WHERE session_record_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(session_record_id)
    .fetch_one(pool)
    .await
    .map_err(|error| format!("查询会话上下文失败: {error}"))?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::model::types::{Message, ToolCall};

    fn meta() -> NativeTranscriptMeta {
        NativeTranscriptMeta {
            profile_id: Some("prof-1".to_string()),
            workspace_id: Some("ws-1".to_string()),
            model: "gpt-4o".to_string(),
            turns: 2,
        }
    }

    #[test]
    fn prepare_strips_system_images_and_orphaned_tools() {
        let messages = vec![
            Message::system("rules"),
            Message::user_with_images(
                "look",
                vec![crate::native::model::types::NativeImage {
                    name: "a.png".to_string(),
                    mime_type: "image/png".to_string(),
                    data_base64: "AAAA".to_string(),
                }],
            ),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
            },
        ];
        let prepared = prepare_transcript_messages(&messages);
        assert!(!prepared.iter().any(|message| message.role == Role::System));
        assert!(prepared[0].images.is_empty());
        assert!(!prepared
            .iter()
            .any(|message| { message.role == Role::Assistant && !message.tool_calls.is_empty() }));
    }

    #[tokio::test]
    async fn save_and_load_round_trip() {
        let pool = setup_migrated_pool().await;
        let messages = vec![
            Message::system("sys"),
            Message::user("fix login"),
            Message::assistant_text("done"),
        ];
        save_transcript(&pool, "sess-1", &messages, &meta())
            .await
            .expect("save");
        assert!(has_transcript(&pool, "sess-1").await.expect("has"));
        let loaded = load_transcript(&pool, "sess-1")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].content, "fix login");
        assert_eq!(loaded[1].content, "done");
        assert!(load_transcript(&pool, "missing")
            .await
            .expect("missing")
            .is_none());
    }

    #[test]
    fn fingerprint_changes_only_when_messages_change() {
        let messages = vec![Message::user("fix login"), Message::assistant_text("done")];
        assert_eq!(
            transcript_fingerprint(&messages),
            transcript_fingerprint(&messages)
        );
        let mut next = messages.clone();
        next.push(Message::assistant_text("more"));
        assert_ne!(
            transcript_fingerprint(&messages),
            transcript_fingerprint(&next)
        );
    }
}
