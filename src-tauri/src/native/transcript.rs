#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use sqlx::SqlitePool;

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
    let mut owned = messages.to_vec();
    crate::native::history::commit_model_context(
        pool,
        crate::native::history::HistoryWrite {
            session_record_id,
            profile_id: meta.profile_id.as_deref(),
            workspace_id: meta.workspace_id.as_deref(),
            model: &meta.model,
            turns: meta.turns,
            messages: &mut owned,
            turn_id: None,
            attempt_id: None,
            expected_revision: None,
            request_id: None,
            links: &[],
            legacy_baseline: false,
        },
    )
    .await
    .map(|_| ())
}

pub async fn load_transcript(
    pool: &SqlitePool,
    resume_session_id: &str,
) -> Result<Option<Vec<Message>>, String> {
    let id = resume_session_id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    crate::native::history::ensure_legacy_imported(pool, id).await?;
    match crate::native::history::load_projection(pool, id).await? {
        Some(messages) if !messages.is_empty() => Ok(Some(messages)),
        _ => Ok(None),
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
                history_id: String::new(),
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

    #[tokio::test]
    async fn save_in_progress_user_turn_round_trip() {
        let pool = setup_migrated_pool().await;
        let messages = vec![Message::system("sys"), Message::user("分析项目")];
        save_transcript(&pool, "sess-mid", &messages, &meta())
            .await
            .expect("save");
        let loaded = load_transcript(&pool, "sess-mid")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].role, Role::User);
        assert_eq!(loaded[0].content, "分析项目");
    }

    #[tokio::test]
    async fn save_completed_tool_round_round_trip() {
        let pool = setup_migrated_pool().await;
        let messages = vec![
            Message::user("分析项目"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "Read".to_string(),
                    arguments: r#"{"file_path":"README.md"}"#.to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
                history_id: String::new(),
            },
            Message::tool_result("call_1", "readme contents"),
        ];
        save_transcript(&pool, "sess-tools", &messages, &meta())
            .await
            .expect("save");
        let loaded = load_transcript(&pool, "sess-tools")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].content, "分析项目");
        assert_eq!(loaded[1].tool_calls[0].id, "call_1");
        assert_eq!(loaded[2].content, "readme contents");
    }

    #[tokio::test]
    async fn in_progress_orphan_tool_call_is_stripped_but_user_kept() {
        let pool = setup_migrated_pool().await;
        let messages = vec![
            Message::user("分析项目"),
            Message {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_open".to_string(),
                    name: "Bash".to_string(),
                    arguments: r#"{"command":"go test"}"#.to_string(),
                }],
                tool_call_id: String::new(),
                name: String::new(),
                reasoning_content: String::new(),
                images: Vec::new(),
                history_id: String::new(),
            },
        ];
        save_transcript(&pool, "sess-orphan", &messages, &meta())
            .await
            .expect("save");
        let loaded = load_transcript(&pool, "sess-orphan")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, "分析项目");
        assert!(!loaded
            .iter()
            .any(|message| !message.tool_calls.is_empty() || message.role == Role::Tool));
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
