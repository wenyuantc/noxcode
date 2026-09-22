//! 从已提交边界恢复模型调用和工具执行。
//!
//! 服务端的 Responses 续接 id 不能代替这里的本地账本。已提交结果直接复用；
//! 还没开始的调用重新执行；已开始但没有结果的副作用标为结果未知，不自动重放。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use sqlx::{Row, SqlitePool};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::model::types::{Message, ToolCall};

pub const UNKNOWN_RESULT: &str = "结果未知：操作已开始但结果未提交，不会自动重放";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRunStatus {
    Planned,
    Started,
    Committed,
    Unknown,
}

impl ToolRunStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Started => "started",
            Self::Committed => "committed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "started" => Self::Started,
            "committed" => Self::Committed,
            "unknown" => Self::Unknown,
            _ => Self::Planned,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub side_effect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    pub turn_id: String,
    pub call_id: String,
    pub call_index: i64,
    pub name: String,
    pub arguments: String,
    pub side_effect: bool,
    pub status: ToolRunStatus,
    pub result_text: Option<String>,
    pub result_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryStep {
    Reuse {
        call_id: String,
        name: String,
        text: String,
        is_error: bool,
    },
    Execute {
        call_id: String,
        name: String,
        arguments: String,
    },
    Unknown {
        call_id: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAssembly {
    pub turn_id: Option<String>,
    pub assistant: Option<Message>,
    pub steps: Vec<RecoveryStep>,
}

pub struct RecoveryState {
    pool: SqlitePool,
    session_id: String,
    turn_id: Mutex<Option<String>>,
    max_attempts: u32,
    /// 只有尝试次数、没有本地工具计划时，服务端续接 id 不能充当提交锚点。
    missing_local_anchor: AtomicBool,
}

impl RecoveryState {
    pub fn new(pool: SqlitePool, session_id: impl Into<String>, max_attempts: u32) -> Self {
        Self {
            pool,
            session_id: session_id.into(),
            turn_id: Mutex::new(None),
            max_attempts: max_attempts.max(1),
            missing_local_anchor: AtomicBool::new(false),
        }
    }

    pub fn missing_local_anchor(&self) -> bool {
        self.missing_local_anchor.load(Ordering::SeqCst)
    }

    pub fn bind_turn(&self, turn_id: &str) {
        *self.turn_id.lock().expect("turn") = Some(turn_id.to_string());
    }

    pub fn turn(&self) -> Option<String> {
        self.turn_id.lock().expect("turn").clone()
    }

    pub async fn commit_plan(&self, calls: &[PlannedCall]) -> Result<(), String> {
        let Some(turn_id) = self.turn() else {
            return Ok(());
        };
        self.missing_local_anchor.store(false, Ordering::SeqCst);
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        for (index, call) in calls.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO native_tool_runs (
                    id, session_record_id, turn_id, call_id, call_index, name, arguments,
                    side_effect, status, result_error, updated_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'planned', 0, $9)
                ON CONFLICT(session_record_id, turn_id, call_id) DO NOTHING
                "#,
            )
            .bind(new_id())
            .bind(&self.session_id)
            .bind(&turn_id)
            .bind(&call.call_id)
            .bind(index as i64)
            .bind(&call.name)
            .bind(&call.arguments)
            .bind(i64::from(call.side_effect))
            .bind(now_sqlite())
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    pub async fn mark_started(&self, call_id: &str) -> Result<(), String> {
        self.set_status(call_id, "started", None, false, "planned")
            .await
    }

    pub async fn commit_result(
        &self,
        call_id: &str,
        text: &str,
        is_error: bool,
    ) -> Result<(), String> {
        let Some(turn_id) = self.turn() else {
            return Ok(());
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            UPDATE native_tool_runs
            SET status = 'committed', result_text = $1, result_error = $2, updated_at = $3
            WHERE session_record_id = $4 AND turn_id = $5 AND call_id = $6
              AND status IN ('planned', 'started')
            "#,
        )
        .bind(text)
        .bind(i64::from(is_error))
        .bind(now_sqlite())
        .bind(&self.session_id)
        .bind(turn_id)
        .bind(call_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    async fn set_status(
        &self,
        call_id: &str,
        status: &str,
        result_text: Option<&str>,
        result_error: bool,
        from: &str,
    ) -> Result<(), String> {
        let Some(turn_id) = self.turn() else {
            return Ok(());
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            UPDATE native_tool_runs
            SET status = $1, result_text = COALESCE($2, result_text), result_error = $3, updated_at = $4
            WHERE session_record_id = $5 AND turn_id = $6 AND call_id = $7 AND status = $8
            "#,
        )
        .bind(status)
        .bind(result_text)
        .bind(i64::from(result_error))
        .bind(now_sqlite())
        .bind(&self.session_id)
        .bind(turn_id)
        .bind(call_id)
        .bind(from)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    /// 把已开始但没有结果的副作用改成结果未知。只读调用保持未提交，恢复时可以再执行。
    pub async fn settle_interrupted(&self) -> Result<Vec<ToolRun>, String> {
        let Some(turn_id) = self.latest_open_turn().await? else {
            return Ok(Vec::new());
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            UPDATE native_tool_runs
            SET status = 'unknown', result_text = $1, result_error = 1, updated_at = $2
            WHERE session_record_id = $3 AND turn_id = $4 AND status = 'started' AND side_effect = 1
            "#,
        )
        .bind(UNKNOWN_RESULT)
        .bind(now_sqlite())
        .bind(&self.session_id)
        .bind(&turn_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        self.bind_turn(&turn_id);
        let runs = self.load_turn(&turn_id).await?;
        self.missing_local_anchor
            .store(runs.is_empty(), Ordering::SeqCst);
        Ok(runs)
    }

    pub fn assemble(existing: &[Message], runs: &[ToolRun]) -> RecoveryAssembly {
        if runs.is_empty() {
            return RecoveryAssembly {
                turn_id: None,
                assistant: None,
                steps: Vec::new(),
            };
        }
        let turn_id = runs.first().map(|run| run.turn_id.clone());
        let assistant_present = existing.iter().any(|message| {
            message
                .tool_calls
                .iter()
                .any(|call| runs.iter().any(|run| run.call_id == call.id))
        });
        let assistant = if assistant_present {
            None
        } else {
            let mut message = Message::assistant_text("");
            message.tool_calls = runs
                .iter()
                .map(|run| ToolCall {
                    id: run.call_id.clone(),
                    name: run.name.clone(),
                    arguments: run.arguments.clone(),
                })
                .collect();
            Some(message)
        };
        let steps = runs
            .iter()
            .map(|run| match run.status {
                ToolRunStatus::Committed => RecoveryStep::Reuse {
                    call_id: run.call_id.clone(),
                    name: run.name.clone(),
                    text: run.result_text.clone().unwrap_or_default(),
                    is_error: run.result_error,
                },
                ToolRunStatus::Unknown => RecoveryStep::Unknown {
                    call_id: run.call_id.clone(),
                    name: run.name.clone(),
                },
                ToolRunStatus::Started if run.side_effect => RecoveryStep::Unknown {
                    call_id: run.call_id.clone(),
                    name: run.name.clone(),
                },
                ToolRunStatus::Planned | ToolRunStatus::Started => RecoveryStep::Execute {
                    call_id: run.call_id.clone(),
                    name: run.name.clone(),
                    arguments: run.arguments.clone(),
                },
            })
            .collect();
        RecoveryAssembly {
            turn_id,
            assistant,
            steps,
        }
    }

    pub async fn used_now(&self) -> Result<u32, String> {
        let Some(turn_id) = self.turn() else {
            return Ok(0);
        };
        self.used_attempts(&turn_id).await
    }

    pub async fn note_model_attempt(&self) -> Result<u32, String> {
        let Some(turn_id) = self.turn() else {
            return Ok(0);
        };
        let used = self.used_attempts(&turn_id).await?;
        if used >= self.max_attempts {
            return Err("重试次数已用尽".to_string());
        }
        let next = used + 1;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            INSERT INTO native_model_attempt_budgets (
                session_record_id, turn_id, used, max_attempts, completed, updated_at
            ) VALUES ($1, $2, $3, $4, 0, $5)
            ON CONFLICT(session_record_id, turn_id) DO UPDATE SET
                used = excluded.used,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&self.session_id)
        .bind(&turn_id)
        .bind(i64::from(next))
        .bind(i64::from(self.max_attempts))
        .bind(now_sqlite())
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(next)
    }

    /// 这一次模型请求已经成功。下一次请求重新计算重试，不把成功请求累加进预算。
    pub async fn release_successful_attempt(&self) -> Result<(), String> {
        let Some(turn_id) = self.turn() else {
            return Ok(());
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            UPDATE native_model_attempt_budgets
            SET used = 0, updated_at = $1
            WHERE session_record_id = $2 AND turn_id = $3 AND completed = 0
            "#,
        )
        .bind(now_sqlite())
        .bind(&self.session_id)
        .bind(turn_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(())
    }

    pub async fn attempts_exhausted(&self) -> Result<bool, String> {
        let Some(turn_id) = self.turn() else {
            return Ok(false);
        };
        Ok(self.used_attempts(&turn_id).await? >= self.max_attempts)
    }

    pub async fn complete_turn(&self) -> Result<(), String> {
        let Some(turn_id) = self.turn() else {
            return Ok(());
        };
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(db_error)?;
        sqlx::query(
            r#"
            UPDATE native_model_attempt_budgets
            SET completed = 1, updated_at = $1
            WHERE session_record_id = $2 AND turn_id = $3
            "#,
        )
        .bind(now_sqlite())
        .bind(&self.session_id)
        .bind(&turn_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        self.missing_local_anchor.store(false, Ordering::SeqCst);
        *self.turn_id.lock().expect("turn") = None;
        Ok(())
    }

    async fn used_attempts(&self, turn_id: &str) -> Result<u32, String> {
        let used: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT used FROM native_model_attempt_budgets
            WHERE session_record_id = $1 AND turn_id = $2
            "#,
        )
        .bind(&self.session_id)
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(u32::try_from(used.unwrap_or(0)).unwrap_or(u32::MAX))
    }

    async fn latest_open_turn(&self) -> Result<Option<String>, String> {
        let turn: Option<String> = sqlx::query_scalar(
            r#"
            SELECT turn_id FROM native_tool_runs
            WHERE session_record_id = $1
              AND status != 'committed'
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(&self.session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        if turn.is_some() {
            return Ok(turn);
        }
        let attempt_turn: Option<String> = sqlx::query_scalar(
            r#"
            SELECT turn_id FROM native_model_attempt_budgets
            WHERE session_record_id = $1 AND completed = 0
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(&self.session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(attempt_turn)
    }

    async fn load_turn(&self, turn_id: &str) -> Result<Vec<ToolRun>, String> {
        let rows = sqlx::query(
            r#"
            SELECT turn_id, call_id, call_index, name, arguments, side_effect, status,
                   result_text, result_error
            FROM native_tool_runs
            WHERE session_record_id = $1 AND turn_id = $2
            ORDER BY call_index ASC
            "#,
        )
        .bind(&self.session_id)
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(rows
            .into_iter()
            .map(|row| ToolRun {
                turn_id: row.get("turn_id"),
                call_id: row.get("call_id"),
                call_index: row.get("call_index"),
                name: row.get("name"),
                arguments: row.get("arguments"),
                side_effect: row.get::<i64, _>("side_effect") == 1,
                status: ToolRunStatus::parse(&row.get::<String, _>("status")),
                result_text: row.get("result_text"),
                result_error: row.get::<i64, _>("result_error") == 1,
            })
            .collect())
    }
}

pub fn require_local_anchor(
    has_local_commit: bool,
    server_response_id: Option<&str>,
) -> Result<(), String> {
    if has_local_commit {
        Ok(())
    } else {
        let suffix = server_response_id
            .filter(|value| !value.is_empty())
            .map(|_| "，服务端续接状态不能代替本地提交锚点")
            .unwrap_or("");
        Err(format!("没有本地提交锚点{suffix}"))
    }
}

pub fn accept_runtime(current_instance: &str, request_instance: &str) -> Result<(), String> {
    if current_instance == request_instance {
        Ok(())
    } else {
        Err("旧运行实例的事件和审批已拒绝".to_string())
    }
}

pub fn step_needs_permission(step: &RecoveryStep) -> bool {
    matches!(step, RecoveryStep::Execute { .. })
}

fn db_error(error: impl std::fmt::Display) -> String {
    format!("恢复账本失败: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;

    fn call(id: &str, name: &str, side_effect: bool) -> PlannedCall {
        PlannedCall {
            call_id: id.to_string(),
            name: name.to_string(),
            arguments: "{}".to_string(),
            side_effect,
        }
    }

    async fn state() -> RecoveryState {
        let recovery = RecoveryState::new(setup_migrated_pool().await, "sess", 7);
        recovery.bind_turn("turn-1");
        recovery
    }

    #[tokio::test]
    async fn crash_points_reuse_committed_results_and_hold_unknown_side_effects() {
        let recovery = state().await;
        recovery
            .commit_plan(&[
                call("read", "Read", false),
                call("write", "Write", true),
                call("bash", "Bash", true),
            ])
            .await
            .unwrap();

        let before_start = recovery.settle_interrupted().await.unwrap();
        let assembly = RecoveryState::assemble(&[], &before_start);
        assert!(assembly
            .steps
            .iter()
            .all(|step| matches!(step, RecoveryStep::Execute { .. })));
        assert!(assembly.steps.iter().all(step_needs_permission));

        recovery.mark_started("write").await.unwrap();
        recovery.commit_result("bash", "ok", false).await.unwrap();
        let settled = recovery.settle_interrupted().await.unwrap();
        let assembly = RecoveryState::assemble(&[], &settled);
        assert!(matches!(assembly.steps[0], RecoveryStep::Execute { .. }));
        assert!(matches!(assembly.steps[1], RecoveryStep::Unknown { .. }));
        assert!(matches!(
            assembly.steps[2],
            RecoveryStep::Reuse { ref text, .. } if text == "ok"
        ));
        let again = recovery.settle_interrupted().await.unwrap();
        assert!(matches!(again[1].status, ToolRunStatus::Unknown));
        assert!(!again.iter().any(|run| {
            run.call_id == "write"
                && matches!(run.status, ToolRunStatus::Planned | ToolRunStatus::Started)
        }));
    }

    #[tokio::test]
    async fn parallel_commit_keeps_original_call_order() {
        let recovery = state().await;
        recovery
            .commit_plan(&[call("a", "Read", false), call("b", "Read", false)])
            .await
            .unwrap();
        recovery.mark_started("b").await.unwrap();
        recovery.commit_result("b", "second", false).await.unwrap();
        let runs = recovery.settle_interrupted().await.unwrap();
        let assembly = RecoveryState::assemble(&[], &runs);
        assert!(matches!(
            &assembly.steps[0],
            RecoveryStep::Execute { call_id, .. } if call_id == "a"
        ));
        assert!(matches!(
            &assembly.steps[1],
            RecoveryStep::Reuse { call_id, text, .. } if call_id == "b" && text == "second"
        ));
    }

    #[tokio::test]
    async fn restart_does_not_reset_attempt_budget() {
        let pool = setup_migrated_pool().await;
        let first = RecoveryState::new(pool.clone(), "sess", 3);
        first.bind_turn("turn-1");
        assert_eq!(first.note_model_attempt().await.unwrap(), 1);
        assert_eq!(first.note_model_attempt().await.unwrap(), 2);
        drop(first);

        let restarted = RecoveryState::new(pool, "sess", 3);
        restarted.bind_turn("turn-1");
        assert_eq!(restarted.note_model_attempt().await.unwrap(), 3);
        let error = restarted.note_model_attempt().await.unwrap_err();
        assert!(error.contains("重试次数已用尽"));
    }

    #[test]
    fn server_continuation_cannot_replace_a_missing_local_anchor() {
        assert!(require_local_anchor(false, Some("resp_123")).is_err());
        assert!(require_local_anchor(true, Some("resp_123")).is_ok());
        assert!(accept_runtime("new-instance", "old-instance").is_err());
        assert!(accept_runtime("same", "same").is_ok());
    }

    #[tokio::test]
    async fn successful_attempt_releases_budget_for_the_next_request() {
        let recovery = state().await;
        assert_eq!(recovery.note_model_attempt().await.unwrap(), 1);
        recovery.release_successful_attempt().await.unwrap();
        assert_eq!(recovery.used_now().await.unwrap(), 0);
        assert_eq!(recovery.note_model_attempt().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn model_retry_without_a_tool_plan_rejects_server_continuation() {
        let pool = setup_migrated_pool().await;
        let recovery = RecoveryState::new(pool.clone(), "sess", 7);
        recovery.bind_turn("turn-1");
        recovery.note_model_attempt().await.unwrap();
        drop(recovery);

        let restarted = RecoveryState::new(pool, "sess", 7);
        let runs = restarted.settle_interrupted().await.unwrap();
        assert!(runs.is_empty());
        assert!(restarted.missing_local_anchor());
        assert!(require_local_anchor(false, Some("resp_1")).is_err());
    }
}
