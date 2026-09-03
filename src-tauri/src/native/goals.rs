//! 会话目标（Goal）：一条持久化的「当前目标 + 进度清单」，模型用 `Goal` 工具维护，
//! `GoalRead` 读取；事件流写 `[GOAL] {json}` 供前端展示。
//!
//! 另含 `ReadSessionContext`：读取同工作区其它会话的标题与最近对话摘录，用于跨会话接续。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::model::types::{Message, Role};

pub const GOAL_LINE_PREFIX: &str = "[GOAL] ";
const MAX_CHECKLIST_ITEMS: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalChecklistItem {
    pub item: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NativeGoalRecord {
    pub id: String,
    pub session_record_id: String,
    pub workspace_id: Option<String>,
    pub title: String,
    pub status: String,
    pub progress_json: String,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeGoal {
    pub id: String,
    pub session_record_id: String,
    pub title: String,
    /// `active` / `completed` / `cleared`。
    pub status: String,
    pub checklist: Vec<GoalChecklistItem>,
    pub note: Option<String>,
    pub updated_at: String,
}

impl NativeGoal {
    fn from_record(record: NativeGoalRecord) -> Self {
        let checklist = serde_json::from_str(&record.progress_json).unwrap_or_default();
        Self {
            id: record.id,
            session_record_id: record.session_record_id,
            title: record.title,
            status: record.status,
            checklist,
            note: record.note,
            updated_at: record.updated_at,
        }
    }

    pub fn line(&self) -> String {
        format!(
            "{GOAL_LINE_PREFIX}{}",
            serde_json::to_string(self).unwrap_or_default()
        )
    }

    pub fn describe(&self) -> String {
        let done = self.checklist.iter().filter(|item| item.done).count();
        let mut lines = vec![format!(
            "目标：{}（{}，{done}/{} 完成）",
            self.title,
            self.status,
            self.checklist.len()
        )];
        for item in &self.checklist {
            lines.push(format!(
                "- [{}] {}",
                if item.done { "x" } else { " " },
                item.item
            ));
        }
        if let Some(note) = self.note.as_deref().filter(|note| !note.trim().is_empty()) {
            lines.push(format!("备注：{note}"));
        }
        lines.join("\n")
    }
}

pub async fn current_goal(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Option<NativeGoal>, String> {
    let record = sqlx::query_as::<_, NativeGoalRecord>(
        "SELECT * FROM native_goals WHERE session_record_id = $1 AND status != 'cleared' ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取目标失败: {error}"))?;
    Ok(record.map(NativeGoal::from_record))
}

fn normalize_checklist(items: Vec<GoalChecklistItem>) -> Vec<GoalChecklistItem> {
    items
        .into_iter()
        .filter(|item| !item.item.trim().is_empty())
        .map(|item| GoalChecklistItem {
            item: item.item.trim().chars().take(200).collect(),
            done: item.done,
        })
        .take(MAX_CHECKLIST_ITEMS)
        .collect()
}

/// 设置 / 更新 / 完成 / 清除目标。返回最新目标（清除时返回 None）。
pub async fn apply_goal_action(
    pool: &SqlitePool,
    session_record_id: &str,
    workspace_id: Option<&str>,
    action: &str,
    title: Option<&str>,
    checklist: Option<Vec<GoalChecklistItem>>,
    note: Option<&str>,
) -> Result<Option<NativeGoal>, String> {
    let existing = current_goal(pool, session_record_id).await?;
    let now = now_sqlite();
    match action {
        "set" => {
            let title = title
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .ok_or_else(|| "设置目标需要 title".to_string())?;
            if let Some(existing) = existing {
                sqlx::query(
                    "UPDATE native_goals SET status = 'cleared', updated_at = $1 WHERE id = $2",
                )
                .bind(&now)
                .bind(&existing.id)
                .execute(pool)
                .await
                .map_err(|error| format!("更新目标失败: {error}"))?;
            }
            let id = new_id();
            let checklist = normalize_checklist(checklist.unwrap_or_default());
            sqlx::query(
                "INSERT INTO native_goals (id, session_record_id, workspace_id, title, status, progress_json, note, created_at, updated_at) VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8)",
            )
            .bind(&id)
            .bind(session_record_id)
            .bind(workspace_id)
            .bind(title)
            .bind(serde_json::to_string(&checklist).unwrap_or_else(|_| "[]".to_string()))
            .bind(note.map(str::trim).filter(|item| !item.is_empty()))
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .map_err(|error| format!("创建目标失败: {error}"))?;
            current_goal(pool, session_record_id).await
        }
        "update" | "complete" => {
            let Some(existing) = existing else {
                return Err("当前没有目标，先用 action=set 设置".to_string());
            };
            let title = title
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or(existing.title.clone());
            let mut checklist = match checklist {
                Some(items) => normalize_checklist(items),
                None => existing.checklist.clone(),
            };
            let status = if action == "complete" {
                for item in &mut checklist {
                    item.done = true;
                }
                "completed"
            } else {
                "active"
            };
            let note = match note {
                Some(value) => Some(value.trim().to_string()).filter(|item| !item.is_empty()),
                None => existing.note.clone(),
            };
            sqlx::query(
                "UPDATE native_goals SET title = $1, status = $2, progress_json = $3, note = $4, updated_at = $5 WHERE id = $6",
            )
            .bind(&title)
            .bind(status)
            .bind(serde_json::to_string(&checklist).unwrap_or_else(|_| "[]".to_string()))
            .bind(&note)
            .bind(&now)
            .bind(&existing.id)
            .execute(pool)
            .await
            .map_err(|error| format!("更新目标失败: {error}"))?;
            current_goal(pool, session_record_id).await
        }
        "clear" => {
            if let Some(existing) = existing {
                sqlx::query(
                    "UPDATE native_goals SET status = 'cleared', updated_at = $1 WHERE id = $2",
                )
                .bind(&now)
                .bind(&existing.id)
                .execute(pool)
                .await
                .map_err(|error| format!("清除目标失败: {error}"))?;
            }
            Ok(None)
        }
        other => Err(format!(
            "未知 action：{other}，应为 set / update / complete / clear"
        )),
    }
}

pub fn parse_checklist(value: Option<&Value>) -> Option<Vec<GoalChecklistItem>> {
    let items = value?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(GoalChecklistItem {
                    item: text.clone(),
                    done: false,
                }),
                Value::Object(map) => Some(GoalChecklistItem {
                    item: map.get("item").and_then(Value::as_str)?.to_string(),
                    done: map.get("done").and_then(Value::as_bool).unwrap_or(false),
                }),
                _ => None,
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// ReadSessionContext
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContextSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub status: String,
    pub started_at: String,
    pub turns: i64,
    pub last_assistant: String,
}

#[derive(Debug, Clone, FromRow)]
struct SessionRow {
    id: String,
    title: Option<String>,
    status: String,
    started_at: String,
}

fn digest_text(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let prefix: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{prefix}…")
    }
}

/// 同工作区最近的会话（不含当前），附最后一条助手回复摘录。
pub async fn list_recent_sessions(
    pool: &SqlitePool,
    workspace_id: &str,
    exclude_session_id: &str,
    limit: usize,
) -> Result<Vec<SessionContextSummary>, String> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, title, status, started_at FROM agent_sessions WHERE workspace_id = $1 AND id != $2 ORDER BY started_at DESC LIMIT $3",
    )
    .bind(workspace_id)
    .bind(exclude_session_id)
    .bind(limit.clamp(1, 50) as i64)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取会话列表失败: {error}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (turns, last_assistant) =
            match crate::native::transcript::load_transcript(pool, &row.id).await? {
                Some(messages) => {
                    let turns = messages
                        .iter()
                        .filter(|message| message.role == Role::User)
                        .count() as i64;
                    let last = messages
                        .iter()
                        .rev()
                        .find(|message| {
                            message.role == Role::Assistant && !message.content.trim().is_empty()
                        })
                        .map(|message| digest_text(&message.content, 240))
                        .unwrap_or_default();
                    (turns, last)
                }
                None => (0, String::new()),
            };
        out.push(SessionContextSummary {
            session_id: row.id,
            title: row.title,
            status: row.status,
            started_at: row.started_at,
            turns,
            last_assistant,
        });
    }
    Ok(out)
}

/// 某个会话的对话摘录（只含用户 / 助手文本，最近 `limit` 条）。
pub async fn session_digest(
    pool: &SqlitePool,
    session_id: &str,
    limit: usize,
) -> Result<String, String> {
    let Some(messages) = crate::native::transcript::load_transcript(pool, session_id).await? else {
        return Err(format!("会话 {session_id} 没有可读取的上下文"));
    };
    let relevant: Vec<&Message> = messages
        .iter()
        .filter(|message| {
            matches!(message.role, Role::User | Role::Assistant)
                && !message.content.trim().is_empty()
        })
        .collect();
    let start = relevant.len().saturating_sub(limit.clamp(1, 60));
    let lines: Vec<String> = relevant[start..]
        .iter()
        .map(|message| {
            let label = if message.role == Role::User {
                "用户"
            } else {
                "助手"
            };
            format!("{label}：{}", digest_text(&message.content, 1_200))
        })
        .collect();
    if lines.is_empty() {
        return Ok("（该会话没有可展示的对话）".to_string());
    }
    Ok(lines.join("\n\n"))
}

pub fn format_session_list(items: &[SessionContextSummary]) -> String {
    if items.is_empty() {
        return "当前工作区没有其它会话。".to_string();
    }
    items
        .iter()
        .map(|item| {
            format!(
                "- {} 「{}」 {} {} 轮 {}{}",
                item.session_id,
                item.title.as_deref().unwrap_or("(无标题)"),
                item.started_at,
                item.turns,
                item.status,
                if item.last_assistant.is_empty() {
                    String::new()
                } else {
                    format!("｜最后回复：{}", item.last_assistant)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::transcript::{save_transcript, NativeTranscriptMeta};

    async fn seed_session(pool: &SqlitePool, id: &str, title: &str) {
        sqlx::query(
            "INSERT INTO agent_sessions (id, workspace_id, title, status) VALUES ($1, 'ws-1', $2, 'exited')",
        )
        .bind(id)
        .bind(title)
        .execute(pool)
        .await
        .expect("session");
    }

    #[tokio::test]
    async fn goal_lifecycle_and_line() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("workspace");
        seed_session(&pool, "s-1", "目标测试").await;
        assert!(current_goal(&pool, "s-1").await.unwrap().is_none());
        let err = apply_goal_action(&pool, "s-1", Some("ws-1"), "update", None, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("先用 action=set"));
        let goal = apply_goal_action(
            &pool,
            "s-1",
            Some("ws-1"),
            "set",
            Some("修复登录 bug"),
            parse_checklist(Some(
                &serde_json::json!(["复现", {"item": "修复", "done": false}]),
            )),
            Some("先看日志"),
        )
        .await
        .unwrap()
        .expect("goal");
        assert_eq!(goal.title, "修复登录 bug");
        assert_eq!(goal.checklist.len(), 2);
        assert!(goal.line().starts_with(GOAL_LINE_PREFIX));
        assert!(goal.describe().contains("0/2 完成"));
        let updated = apply_goal_action(
            &pool,
            "s-1",
            Some("ws-1"),
            "update",
            None,
            parse_checklist(Some(
                &serde_json::json!([{"item": "复现", "done": true}, "修复"]),
            )),
            None,
        )
        .await
        .unwrap()
        .expect("goal");
        assert_eq!(updated.id, goal.id);
        assert!(updated.checklist[0].done);
        assert_eq!(updated.note.as_deref(), Some("先看日志"));
        let completed = apply_goal_action(&pool, "s-1", None, "complete", None, None, None)
            .await
            .unwrap()
            .expect("goal");
        assert_eq!(completed.status, "completed");
        assert!(completed.checklist.iter().all(|item| item.done));
        assert!(
            apply_goal_action(&pool, "s-1", None, "clear", None, None, None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(current_goal(&pool, "s-1").await.unwrap().is_none());
        assert!(
            apply_goal_action(&pool, "s-1", None, "bogus", None, None, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn session_context_lists_and_digests() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("workspace");
        seed_session(&pool, "s-old", "旧会话").await;
        seed_session(&pool, "s-now", "当前").await;
        save_transcript(
            &pool,
            "s-old",
            &[
                Message::user("把登录改成 OAuth"),
                Message::assistant_text("已改好，测试通过。"),
            ],
            &NativeTranscriptMeta {
                profile_id: None,
                workspace_id: Some("ws-1".to_string()),
                model: "m".to_string(),
                turns: 1,
            },
        )
        .await
        .expect("save");
        let list = list_recent_sessions(&pool, "ws-1", "s-now", 10)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, "s-old");
        assert_eq!(list[0].turns, 1);
        assert!(list[0].last_assistant.contains("测试通过"));
        let text = format_session_list(&list);
        assert!(text.contains("旧会话"));
        let digest = session_digest(&pool, "s-old", 10).await.unwrap();
        assert!(digest.contains("用户：把登录改成 OAuth"));
        assert!(session_digest(&pool, "missing", 10).await.is_err());
    }
}
