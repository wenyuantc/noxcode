//! 会话目标（Goal）：一条持久化的「当前目标 + 进度清单」，模型用 `Goal` 工具维护，
//! `GoalRead` 读取；事件流写 `[GOAL] {json}` 供前端展示。
//!
//! 另含 `ReadSessionContext`：读取同工作区其它会话的标题与最近对话摘录，用于跨会话接续。

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, Row, SqlitePool};

use crate::app::shared::{new_id, now_sqlite};
use crate::native::model::types::{Message, Role};

pub const GOAL_LINE_PREFIX: &str = "[GOAL] ";
pub const GOAL_CONTINUE_LIMIT: i64 = 3;
const MAX_CHECKLIST_ITEMS: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalChecklistItem {
    pub item: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalCriterion {
    pub id: String,
    pub kind: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalVerificationView {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedToolResult {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub status: String,
    pub result_text: String,
    pub result_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSnapshot {
    pub path: String,
    pub text: Option<String>,
}

pub type GoalReviewHook = std::sync::Arc<
    dyn Fn(
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = GoalReviewDecision> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalReviewDecision {
    Pass,
    Fail(String),
    TimedOut,
    Cancelled,
    Unavailable(String),
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
    pub bound_branch_id: Option<String>,
    pub criteria_json: String,
    pub criteria_version: i64,
    pub continue_count: i64,
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
    /// 完成状态所属的历史分支。分支变化后，旧完成记录不再作为当前证据。
    #[serde(default)]
    pub bound_branch_id: Option<String>,
    #[serde(default)]
    pub criteria: Vec<GoalCriterion>,
    #[serde(default)]
    pub criteria_version: i64,
    #[serde(default)]
    pub continue_count: i64,
    #[serde(default)]
    pub continue_limit: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<GoalVerificationView>,
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
            bound_branch_id: record.bound_branch_id,
            criteria: serde_json::from_str(&record.criteria_json).unwrap_or_default(),
            criteria_version: record.criteria_version,
            continue_count: record.continue_count,
            continue_limit: GOAL_CONTINUE_LIMIT,
            verification: None,
        }
    }

    pub fn counts_as_complete(&self, active_branch: Option<&str>) -> bool {
        if self.status != "completed" {
            return false;
        }
        match (self.bound_branch_id.as_deref(), active_branch) {
            (Some(bound), Some(active)) => bound == active,
            (None, _) => true,
            _ => false,
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
        if let Some(verification) = &self.verification {
            lines.push(format!("核验：{}", verification.status));
            if let Some(reason) = verification
                .failure_reason
                .as_deref()
                .filter(|reason| !reason.is_empty())
            {
                lines.push(reason.to_string());
            }
            for item in &verification.evidence {
                lines.push(format!("- {item}"));
            }
            let remaining = GOAL_CONTINUE_LIMIT.saturating_sub(self.continue_count);
            lines.push(format!("剩余自动继续：{remaining}"));
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
    let Some(record) = record else {
        return Ok(None);
    };
    let mut goal = NativeGoal::from_record(record);
    let active = crate::native::history::active_branch_id(pool, session_record_id).await?;
    goal.verification =
        latest_verification_view(pool, &goal.id, goal.criteria_version, active.as_deref()).await?;
    if goal.status == "completed" && !goal.counts_as_complete(active.as_deref()) {
        goal.status = "active".to_string();
        let notice = "来源分支的完成记录不能作为当前分支的完成证据";
        goal.note = Some(match goal.note.take() {
            Some(note) if note.contains(notice) => note,
            Some(note) => format!("{note}\n{notice}"),
            None => notice.to_string(),
        });
    }
    Ok(Some(goal))
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
#[allow(clippy::too_many_arguments)]
pub async fn apply_goal_action(
    pool: &SqlitePool,
    session_record_id: &str,
    workspace_id: Option<&str>,
    action: &str,
    title: Option<&str>,
    checklist: Option<Vec<GoalChecklistItem>>,
    note: Option<&str>,
    criteria: Option<Vec<GoalCriterion>>,
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
            let criteria = normalize_criteria(criteria.unwrap_or_default());
            let branch_id =
                crate::native::history::active_branch_id(pool, session_record_id).await?;
            sqlx::query(
                "INSERT INTO native_goals (id, session_record_id, workspace_id, title, status, progress_json, note, created_at, updated_at, bound_branch_id, criteria_json, criteria_version, continue_count) VALUES ($1, $2, $3, $4, 'active', $5, $6, $7, $8, $9, $10, 1, 0)",
            )
            .bind(&id)
            .bind(session_record_id)
            .bind(workspace_id)
            .bind(title)
            .bind(serde_json::to_string(&checklist).unwrap_or_else(|_| "[]".to_string()))
            .bind(note.map(str::trim).filter(|item| !item.is_empty()))
            .bind(&now)
            .bind(&now)
            .bind(branch_id)
            .bind(serde_json::to_string(&criteria).unwrap_or_else(|_| "[]".to_string()))
            .execute(pool)
            .await
            .map_err(|error| format!("创建目标失败: {error}"))?;
            current_goal(pool, session_record_id).await
        }
        "complete" => {
            verify_goal(
                pool,
                session_record_id,
                &[],
                &[],
                GoalReviewDecision::Unavailable("未提供可读取的证据".to_string()),
                false,
            )
            .await
        }
        "update" => {
            let Some(existing) = existing else {
                return Err("当前没有目标，先用 action=set 设置".to_string());
            };
            let title = title
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or(existing.title.clone());
            let checklist = match checklist {
                Some(items) => normalize_checklist(items),
                None => existing.checklist.clone(),
            };
            let (criteria_json, criteria_version, continue_count) = match criteria {
                Some(items) => {
                    let normalized = normalize_criteria(items);
                    let json =
                        serde_json::to_string(&normalized).unwrap_or_else(|_| "[]".to_string());
                    let stored = serde_json::to_string(&existing.criteria)
                        .unwrap_or_else(|_| "[]".to_string());
                    if json == stored {
                        (json, existing.criteria_version, existing.continue_count)
                    } else {
                        (json, existing.criteria_version.saturating_add(1), 0)
                    }
                }
                None => (
                    serde_json::to_string(&existing.criteria).unwrap_or_else(|_| "[]".to_string()),
                    existing.criteria_version,
                    existing.continue_count,
                ),
            };
            let status = "active";
            let note = match note {
                Some(value) => Some(value.trim().to_string()).filter(|item| !item.is_empty()),
                None => existing.note.clone(),
            };
            let branch_id =
                crate::native::history::active_branch_id(pool, session_record_id).await?;
            sqlx::query(
                "UPDATE native_goals SET title = $1, status = $2, progress_json = $3, note = $4, updated_at = $5, bound_branch_id = COALESCE($7, bound_branch_id), criteria_json = $8, criteria_version = $9, continue_count = $10 WHERE id = $6",
            )
            .bind(&title)
            .bind(status)
            .bind(serde_json::to_string(&checklist).unwrap_or_else(|_| "[]".to_string()))
            .bind(&note)
            .bind(&now)
            .bind(&existing.id)
            .bind(branch_id)
            .bind(&criteria_json)
            .bind(criteria_version)
            .bind(continue_count)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEvidence {
    criterion_id: String,
    kind: String,
    passed: bool,
    detail: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
}

pub async fn verify_goal(
    pool: &SqlitePool,
    session_record_id: &str,
    tools: &[SubmittedToolResult],
    artifacts: &[ArtifactSnapshot],
    review: GoalReviewDecision,
    cancelled: bool,
) -> Result<Option<NativeGoal>, String> {
    let Some(existing) = load_goal_record(pool, session_record_id).await? else {
        return Err("当前没有目标，先用 action=set 设置".to_string());
    };
    let goal = NativeGoal::from_record(existing);
    if cancelled || matches!(review, GoalReviewDecision::Cancelled) {
        save_verification(
            pool,
            &goal,
            "cancelled",
            "核验已取消",
            &[],
            goal.continue_count,
        )
        .await?;
        return current_goal(pool, session_record_id).await;
    }
    let timed_out = matches!(review, GoalReviewDecision::TimedOut);
    let mut evidence = Vec::new();
    if goal.criteria.is_empty() {
        evidence.push(StoredEvidence {
            criterion_id: String::new(),
            kind: "none".to_string(),
            passed: false,
            detail: "没有验收条件，不能标为完成".to_string(),
            path: None,
            fingerprint: None,
            call_id: None,
        });
    }
    for criterion in &goal.criteria {
        evidence.push(judge_criterion(criterion, tools, artifacts, &review));
    }
    let all_passed =
        !goal.criteria.is_empty() && !timed_out && evidence.iter().all(|item| item.passed);
    let exhausted = goal.continue_count >= GOAL_CONTINUE_LIMIT;
    let mut continue_count = goal.continue_count;
    let (verification_status, goal_status) = if timed_out {
        if continue_count < GOAL_CONTINUE_LIMIT {
            continue_count += 1;
        }
        let status = if continue_count >= GOAL_CONTINUE_LIMIT {
            "budget_exhausted"
        } else {
            "timed_out"
        };
        (status, "active")
    } else if all_passed && !exhausted {
        continue_count = 0;
        ("passed", "completed")
    } else {
        if continue_count < GOAL_CONTINUE_LIMIT {
            continue_count += 1;
        }
        let status = if continue_count >= GOAL_CONTINUE_LIMIT {
            "budget_exhausted"
        } else {
            "failed"
        };
        (status, "active")
    };
    let failure_reason = if goal_status == "completed" {
        String::new()
    } else if verification_status == "budget_exhausted" {
        "自动继续次数已用完，核验暂停".to_string()
    } else if verification_status == "timed_out" {
        "核验超时，目标保持未完成".to_string()
    } else {
        evidence
            .iter()
            .filter(|item| !item.passed)
            .map(|item| item.detail.clone())
            .collect::<Vec<_>>()
            .join("；")
    };
    let branch_id = crate::native::history::active_branch_id(pool, session_record_id).await?;
    let mut checklist = goal.checklist.clone();
    if goal_status == "completed" {
        for item in &mut checklist {
            item.done = true;
        }
    }
    let now = now_sqlite();
    sqlx::query(
        "UPDATE native_goals SET status = $1, progress_json = $2, continue_count = $3, bound_branch_id = COALESCE($4, bound_branch_id), updated_at = $5 WHERE id = $6",
    )
    .bind(goal_status)
    .bind(serde_json::to_string(&checklist).unwrap_or_else(|_| "[]".to_string()))
    .bind(continue_count)
    .bind(&branch_id)
    .bind(&now)
    .bind(&goal.id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新目标核验失败: {error}"))?;
    save_verification(
        pool,
        &goal,
        verification_status,
        &failure_reason,
        &evidence,
        continue_count,
    )
    .await?;
    current_goal(pool, session_record_id).await
}

fn judge_criterion(
    criterion: &GoalCriterion,
    tools: &[SubmittedToolResult],
    artifacts: &[ArtifactSnapshot],
    review: &GoalReviewDecision,
) -> StoredEvidence {
    match criterion.kind.as_str() {
        "test" => judge_test(criterion, tools),
        "artifact" => judge_artifact(criterion, artifacts),
        "subjective" => judge_subjective(criterion, artifacts, review),
        other => StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: other.to_string(),
            passed: false,
            detail: format!("{}：未知验收条件", criterion.description),
            path: criterion.path.clone(),
            fingerprint: None,
            call_id: None,
        },
    }
}

fn judge_test(criterion: &GoalCriterion, tools: &[SubmittedToolResult]) -> StoredEvidence {
    let command = criterion.command.as_deref().unwrap_or("").trim();
    let matched = tools.iter().find(|tool| {
        tool.name == "Bash"
            && tool.status == "committed"
            && !tool.result_error
            && !command.is_empty()
            && tool.arguments.contains(command)
    });
    match matched {
        Some(tool) => StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: "test".to_string(),
            passed: true,
            detail: format!(
                "{}：已读取提交结果 {}",
                criterion.description,
                clip(&tool.result_text, 120)
            ),
            path: None,
            fingerprint: None,
            call_id: Some(tool.call_id.clone()),
        },
        None => StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: "test".to_string(),
            passed: false,
            detail: format!("{}：没有可读取的测试结果", criterion.description),
            path: None,
            fingerprint: None,
            call_id: None,
        },
    }
}

fn judge_artifact(criterion: &GoalCriterion, artifacts: &[ArtifactSnapshot]) -> StoredEvidence {
    let path = criterion.path.as_deref().unwrap_or("").trim();
    let snapshot = artifacts.iter().find(|item| item.path == path);
    match snapshot.and_then(|item| item.text.as_ref()) {
        Some(text) if !path.is_empty() => StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: "artifact".to_string(),
            passed: true,
            detail: format!("{}：已读取产物 {path}", criterion.description),
            path: Some(path.to_string()),
            fingerprint: Some(content_sha(text)),
            call_id: None,
        },
        _ => StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: "artifact".to_string(),
            passed: false,
            detail: format!("{}：产物不存在", criterion.description),
            path: Some(path.to_string()),
            fingerprint: None,
            call_id: None,
        },
    }
}

fn reviewed_artifact<'a>(
    criterion: &GoalCriterion,
    artifacts: &'a [ArtifactSnapshot],
) -> Option<&'a ArtifactSnapshot> {
    if let Some(path) = criterion
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return artifacts.iter().find(|item| {
            item.path == path && item.text.as_ref().is_some_and(|text| !text.is_empty())
        });
    }
    artifacts
        .iter()
        .find(|item| item.text.as_ref().is_some_and(|text| !text.is_empty()))
}

fn judge_subjective(
    criterion: &GoalCriterion,
    artifacts: &[ArtifactSnapshot],
    review: &GoalReviewDecision,
) -> StoredEvidence {
    let Some(snapshot) = reviewed_artifact(criterion, artifacts) else {
        return StoredEvidence {
            criterion_id: criterion.id.clone(),
            kind: "subjective".to_string(),
            passed: false,
            detail: format!("{}：没有可复核的产物", criterion.description),
            path: criterion.path.clone(),
            fingerprint: None,
            call_id: None,
        };
    };
    let (passed, detail) = match review {
        GoalReviewDecision::Pass => (true, format!("{}：轻量模型复核通过", criterion.description)),
        GoalReviewDecision::Fail(reason) => (
            false,
            format!("{}：复核未通过 {reason}", criterion.description),
        ),
        GoalReviewDecision::TimedOut => (false, format!("{}：复核超时", criterion.description)),
        GoalReviewDecision::Cancelled => (false, format!("{}：复核已取消", criterion.description)),
        GoalReviewDecision::Unavailable(reason) => {
            (false, format!("{}：{reason}", criterion.description))
        }
    };
    StoredEvidence {
        criterion_id: criterion.id.clone(),
        kind: "subjective".to_string(),
        passed,
        detail,
        path: Some(snapshot.path.clone()),
        fingerprint: snapshot.text.as_deref().map(content_sha),
        call_id: None,
    }
}

pub async fn invalidate_changed_artifacts(
    pool: &SqlitePool,
    session_record_id: &str,
    artifacts: &[ArtifactSnapshot],
) -> Result<Option<NativeGoal>, String> {
    let Some(goal) = current_goal(pool, session_record_id).await? else {
        return Ok(None);
    };
    if goal.status != "completed" {
        return Ok(Some(goal));
    }
    let active = crate::native::history::active_branch_id(pool, session_record_id).await?;
    let Some(raw) =
        latest_verification_raw(pool, &goal.id, goal.criteria_version, active.as_deref()).await?
    else {
        return Ok(Some(goal));
    };
    if raw.status != "passed" {
        return Ok(Some(goal));
    }
    let stored: Vec<StoredEvidence> = serde_json::from_str(&raw.evidence_json).unwrap_or_default();
    let stale = stored.iter().any(|item| {
        let Some(fingerprint) = item.fingerprint.as_deref() else {
            return false;
        };
        let Some(path) = item.path.as_deref() else {
            return false;
        };
        match artifacts.iter().find(|artifact| artifact.path == path) {
            Some(artifact) => {
                artifact.text.as_deref().map(content_sha).as_deref() != Some(fingerprint)
            }
            None => false,
        }
    });
    if !stale {
        return Ok(Some(goal));
    }
    sqlx::query("UPDATE native_goals SET status = 'active', updated_at = $1 WHERE id = $2")
        .bind(now_sqlite())
        .bind(&goal.id)
        .execute(pool)
        .await
        .map_err(|error| format!("更新目标失败: {error}"))?;
    save_verification(
        pool,
        &goal,
        "failed",
        "产物已变化，旧证据失效",
        &[],
        goal.continue_count,
    )
    .await?;
    current_goal(pool, session_record_id).await
}

pub async fn verification_paused(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<bool, String> {
    let Some(goal) = load_goal_record(pool, session_record_id).await? else {
        return Ok(false);
    };
    let active = crate::native::history::active_branch_id(pool, session_record_id).await?;
    let Some(raw) =
        latest_verification_raw(pool, &goal.id, goal.criteria_version, active.as_deref()).await?
    else {
        return Ok(false);
    };
    Ok(raw.status == "budget_exhausted")
}

pub fn read_local_artifacts(root: &Path, criteria: &[GoalCriterion]) -> Vec<ArtifactSnapshot> {
    criteria
        .iter()
        .filter_map(|criterion| criterion.path.clone())
        .map(|path| {
            let text = read_under_root(root, &path).ok().flatten();
            ArtifactSnapshot { path, text }
        })
        .collect()
}

pub async fn load_committed_tools(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Vec<SubmittedToolResult>, String> {
    let rows = sqlx::query(
        r#"
        SELECT call_id, name, arguments, status, result_text, result_error
        FROM native_tool_runs
        WHERE session_record_id = $1 AND status = 'committed'
        ORDER BY updated_at ASC
        "#,
    )
    .bind(session_record_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取工具结果失败: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| SubmittedToolResult {
            call_id: row.get("call_id"),
            name: row.get("name"),
            arguments: row.get("arguments"),
            status: row.get("status"),
            result_text: row
                .get::<Option<String>, _>("result_text")
                .unwrap_or_default(),
            result_error: row.get::<i64, _>("result_error") != 0,
        })
        .collect())
}

pub async fn list_verifications(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Vec<GoalVerificationView>, String> {
    let Some(goal) = load_goal_record(pool, session_record_id).await? else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query(
        r#"
        SELECT status, failure_reason, evidence_json
        FROM native_goal_verifications
        WHERE goal_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(&goal.id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取核验记录失败: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| GoalVerificationView {
            status: row.get("status"),
            failure_reason: row.get("failure_reason"),
            evidence: evidence_lines(&row.get::<String, _>("evidence_json")),
        })
        .collect())
}

#[tauri::command]
pub async fn list_native_goal_verifications(
    app: tauri::AppHandle,
    session_record_id: String,
) -> Result<Vec<GoalVerificationView>, String> {
    let pool = crate::app::shared::sqlite_pool(&app).await?;
    list_verifications(&pool, session_record_id.trim()).await
}

async fn load_goal_record(
    pool: &SqlitePool,
    session_record_id: &str,
) -> Result<Option<NativeGoalRecord>, String> {
    sqlx::query_as::<_, NativeGoalRecord>(
        "SELECT * FROM native_goals WHERE session_record_id = $1 AND status != 'cleared' ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取目标失败: {error}"))
}

async fn save_verification(
    pool: &SqlitePool,
    goal: &NativeGoal,
    status: &str,
    failure_reason: &str,
    evidence: &[StoredEvidence],
    continue_count: i64,
) -> Result<(), String> {
    let branch_id = crate::native::history::active_branch_id(pool, &goal.session_record_id).await?;
    let reason = if failure_reason.is_empty() {
        None
    } else {
        Some(failure_reason)
    };
    sqlx::query(
        r#"
        INSERT INTO native_goal_verifications (
            id, goal_id, session_record_id, criteria_version, branch_id, status,
            evidence_json, failure_reason, continue_count, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(new_id())
    .bind(&goal.id)
    .bind(&goal.session_record_id)
    .bind(goal.criteria_version)
    .bind(branch_id)
    .bind(status)
    .bind(serde_json::to_string(evidence).unwrap_or_else(|_| "[]".to_string()))
    .bind(reason)
    .bind(continue_count)
    .bind(now_sqlite())
    .execute(pool)
    .await
    .map_err(|error| format!("保存核验记录失败: {error}"))?;
    Ok(())
}

struct VerificationRaw {
    status: String,
    evidence_json: String,
    failure_reason: Option<String>,
}

async fn latest_verification_raw(
    pool: &SqlitePool,
    goal_id: &str,
    criteria_version: i64,
    branch_id: Option<&str>,
) -> Result<Option<VerificationRaw>, String> {
    let row = sqlx::query(
        r#"
        SELECT status, evidence_json, failure_reason
        FROM native_goal_verifications
        WHERE goal_id = $1
          AND criteria_version = $2
          AND ($3 IS NULL OR branch_id IS NULL OR branch_id = $3)
        ORDER BY created_at DESC, rowid DESC
        LIMIT 1
        "#,
    )
    .bind(goal_id)
    .bind(criteria_version)
    .bind(branch_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取核验记录失败: {error}"))?;
    Ok(row.map(|row| VerificationRaw {
        status: row.get("status"),
        evidence_json: row.get("evidence_json"),
        failure_reason: row.get("failure_reason"),
    }))
}

async fn latest_verification_view(
    pool: &SqlitePool,
    goal_id: &str,
    criteria_version: i64,
    branch_id: Option<&str>,
) -> Result<Option<GoalVerificationView>, String> {
    let Some(raw) = latest_verification_raw(pool, goal_id, criteria_version, branch_id).await?
    else {
        return Ok(None);
    };
    Ok(Some(GoalVerificationView {
        status: raw.status,
        failure_reason: raw.failure_reason,
        evidence: evidence_lines(&raw.evidence_json),
    }))
}

fn evidence_lines(json: &str) -> Vec<String> {
    let items: Vec<StoredEvidence> = serde_json::from_str(json).unwrap_or_default();
    items.into_iter().map(|item| item.detail).collect()
}

fn content_sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let prefix: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{prefix}…")
    }
}

fn read_under_root(root: &Path, relative: &str) -> Result<Option<String>, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() || relative.contains('\0') {
        return Err(format!("拒绝绝对路径 {relative}"));
    }
    let mut path = root.to_path_buf();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return Err(format!("路径越界 {relative}")),
        }
    }
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("读取 {relative} 失败: {error}"))
}

fn normalize_criteria(items: Vec<GoalCriterion>) -> Vec<GoalCriterion> {
    items
        .into_iter()
        .filter(|item| !item.description.trim().is_empty())
        .map(|item| {
            let description = item
                .description
                .trim()
                .chars()
                .take(200)
                .collect::<String>();
            let command = item
                .command
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let path = item
                .path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let kind = match item.kind.as_str() {
                "test" | "artifact" | "subjective" => item.kind,
                _ => "subjective".to_string(),
            };
            GoalCriterion {
                id: criterion_id(&kind, &description, command.as_deref(), path.as_deref()),
                kind,
                description,
                command,
                path,
            }
        })
        .take(MAX_CHECKLIST_ITEMS)
        .collect()
}

fn criterion_id(
    kind: &str,
    description: &str,
    command: Option<&str>,
    path: Option<&str>,
) -> String {
    content_sha(&format!(
        "{kind}|{description}|{}|{}",
        command.unwrap_or(""),
        path.unwrap_or("")
    ))
}

pub fn parse_criteria(value: Option<&Value>) -> Option<Vec<GoalCriterion>> {
    let items = value?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| {
                let map = item.as_object()?;
                let description = map.get("description").and_then(Value::as_str)?.to_string();
                let kind = map
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("subjective")
                    .to_string();
                Some(GoalCriterion {
                    id: String::new(),
                    kind,
                    description,
                    command: map
                        .get("command")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    path: map
                        .get("path")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                })
            })
            .collect(),
    )
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

pub fn interpret_review(text: &str) -> GoalReviewDecision {
    if text.contains("结论: 不通过") || text.contains("结论：不通过") {
        GoalReviewDecision::Fail(text.trim().to_string())
    } else if text.contains("结论: 通过") || text.contains("结论：通过") {
        GoalReviewDecision::Pass
    } else {
        GoalReviewDecision::Fail("复核没有给出明确结论".to_string())
    }
}

pub async fn review_goal_with_client(
    client: &crate::native::model::client::ModelClient,
    main_model: &str,
    lite_model: Option<&str>,
    material: &str,
) -> GoalReviewDecision {
    use crate::native::model::client::ChatRequest;
    use crate::native::model::response::ModelErrorKind;
    let model = lite_model
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or(main_model);
    let messages = vec![
        Message::system(
            "你只复核给定产物是否满足验收条件。不要提议或执行命令。只输出一行：结论: 通过 或 结论: 不通过",
        ),
        Message::user(material),
    ];
    match client
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model,
            effort: None,
            max_output_tokens: Some(128),
            thinking_enabled: false,
        })
        .await
    {
        Ok(response) => match response.complete_message() {
            Ok(message) => interpret_review(&message.content),
            Err(_) => GoalReviewDecision::Unavailable("复核没有返回文本".to_string()),
        },
        Err(error) => match error.kind {
            ModelErrorKind::Cancelled => GoalReviewDecision::Cancelled,
            ModelErrorKind::FirstByteTimeout
            | ModelErrorKind::StreamIdle
            | ModelErrorKind::RequestTimeout => GoalReviewDecision::TimedOut,
            _ => GoalReviewDecision::Unavailable(error.message),
        },
    }
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

    #[test]
    fn review_text_needs_an_explicit_conclusion() {
        assert_eq!(interpret_review("结论: 通过"), GoalReviewDecision::Pass);
        assert_eq!(interpret_review("结论：通过"), GoalReviewDecision::Pass);
        match interpret_review("结论: 不通过\n缺标题") {
            GoalReviewDecision::Fail(reason) => assert!(reason.contains("不通过")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            interpret_review("看起来还行"),
            GoalReviewDecision::Fail(_)
        ));
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
        let err = apply_goal_action(&pool, "s-1", Some("ws-1"), "update", None, None, None, None)
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
            None,
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
            None,
        )
        .await
        .unwrap()
        .expect("goal");
        assert_eq!(updated.id, goal.id);
        assert!(updated.checklist[0].done);
        assert_eq!(updated.note.as_deref(), Some("先看日志"));
        let completed = apply_goal_action(&pool, "s-1", None, "complete", None, None, None, None)
            .await
            .unwrap()
            .expect("goal");
        assert_eq!(completed.status, "active");
        assert!(completed.describe().contains("没有验收条件"));
        assert!(
            apply_goal_action(&pool, "s-1", None, "clear", None, None, None, None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(current_goal(&pool, "s-1").await.unwrap().is_none());
        assert!(
            apply_goal_action(&pool, "s-1", None, "bogus", None, None, None, None)
                .await
                .is_err()
        );
    }

    fn sample_criteria() -> Vec<GoalCriterion> {
        vec![
            GoalCriterion {
                id: String::new(),
                kind: "test".to_string(),
                description: "单元测试".to_string(),
                command: Some("cargo test".to_string()),
                path: None,
            },
            GoalCriterion {
                id: String::new(),
                kind: "artifact".to_string(),
                description: "产物".to_string(),
                command: None,
                path: Some("out.txt".to_string()),
            },
            GoalCriterion {
                id: String::new(),
                kind: "subjective".to_string(),
                description: "读起来正确".to_string(),
                command: None,
                path: Some("out.txt".to_string()),
            },
        ]
    }

    async fn insert_bash(
        pool: &SqlitePool,
        session: &str,
        call_id: &str,
        command: &str,
        output: &str,
        result_error: bool,
    ) {
        sqlx::query(
            r#"
            INSERT INTO native_tool_runs (
                id, session_record_id, turn_id, call_id, call_index, name, arguments,
                side_effect, status, result_text, result_error, updated_at
            ) VALUES ($1, $2, 'turn', $3, 0, 'Bash', $4, 1, 'committed', $5, $6, $7)
            "#,
        )
        .bind(new_id())
        .bind(session)
        .bind(call_id)
        .bind(format!(r#"{{"command":"{command}"}}"#))
        .bind(output)
        .bind(i64::from(result_error))
        .bind(now_sqlite())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn verification_requires_backend_evidence_and_stops_after_three_failures() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .unwrap();
        seed_session(&pool, "s-v", "核验").await;
        apply_goal_action(
            &pool,
            "s-v",
            Some("ws-1"),
            "set",
            Some("做出产物"),
            None,
            Some("测试通过"),
            Some(sample_criteria()),
        )
        .await
        .unwrap();
        let claimed = verify_goal(&pool, "s-v", &[], &[], GoalReviewDecision::Pass, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, "active");
        assert!(claimed.describe().contains("没有可读取的测试结果"));
        assert_eq!(claimed.continue_count, 1);

        let timed = verify_goal(&pool, "s-v", &[], &[], GoalReviewDecision::TimedOut, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(timed.status, "active");
        assert_eq!(timed.verification.as_ref().unwrap().status, "timed_out");
        assert_eq!(timed.continue_count, 2);

        let cancelled = verify_goal(&pool, "s-v", &[], &[], GoalReviewDecision::Cancelled, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.status, "active");
        assert_eq!(cancelled.verification.as_ref().unwrap().status, "cancelled");
        assert_eq!(cancelled.continue_count, 2);

        let failed = verify_goal(
            &pool,
            "s-v",
            &[],
            &[],
            GoalReviewDecision::Fail("不对".into()),
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(failed.continue_count, 3);
        assert_eq!(
            failed.verification.as_ref().unwrap().status,
            "budget_exhausted"
        );
        assert!(verification_paused(&pool, "s-v").await.unwrap());

        insert_bash(&pool, "s-v", "ok", "cargo test", "1 passed", false).await;
        let artifacts = vec![ArtifactSnapshot {
            path: "out.txt".into(),
            text: Some("abc".into()),
        }];
        let blocked = verify_goal(
            &pool,
            "s-v",
            &load_committed_tools(&pool, "s-v").await.unwrap(),
            &artifacts,
            GoalReviewDecision::Pass,
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(blocked.status, "active");
        assert_ne!(blocked.verification.as_ref().unwrap().status, "passed");

        apply_goal_action(
            &pool,
            "s-v",
            None,
            "update",
            None,
            None,
            None,
            Some(vec![GoalCriterion {
                id: String::new(),
                kind: "test".into(),
                description: "只测客观".into(),
                command: Some("cargo test".into()),
                path: None,
            }]),
        )
        .await
        .unwrap();
        assert!(!verification_paused(&pool, "s-v").await.unwrap());
        let reset = current_goal(&pool, "s-v").await.unwrap().unwrap();
        assert_eq!(reset.continue_count, 0);
        assert!(reset.verification.is_none());
        let passed = verify_goal(
            &pool,
            "s-v",
            &load_committed_tools(&pool, "s-v").await.unwrap(),
            &[],
            GoalReviewDecision::Unavailable("不需要".into()),
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(passed.status, "completed");
        assert_eq!(passed.verification.as_ref().unwrap().status, "passed");
        assert!(passed.verification.as_ref().unwrap().evidence[0].contains("1 passed"));

        apply_goal_action(
            &pool,
            "s-v",
            None,
            "update",
            None,
            None,
            None,
            Some(sample_criteria()),
        )
        .await
        .unwrap();
        let subjective = verify_goal(
            &pool,
            "s-v",
            &load_committed_tools(&pool, "s-v").await.unwrap(),
            &artifacts,
            GoalReviewDecision::Fail("语气不对".into()),
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(subjective.status, "active");
        assert!(subjective.describe().contains("复核未通过"));
        let reviewed = verify_goal(
            &pool,
            "s-v",
            &load_committed_tools(&pool, "s-v").await.unwrap(),
            &artifacts,
            GoalReviewDecision::Pass,
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reviewed.status, "completed");
        let stale = invalidate_changed_artifacts(
            &pool,
            "s-v",
            &[ArtifactSnapshot {
                path: "out.txt".into(),
                text: Some("changed".into()),
            }],
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(stale.status, "active");
        assert!(stale.describe().contains("旧证据失效"));

        let again = verify_goal(
            &pool,
            "s-v",
            &load_committed_tools(&pool, "s-v").await.unwrap(),
            &[ArtifactSnapshot {
                path: "out.txt".into(),
                text: Some("changed".into()),
            }],
            GoalReviewDecision::Pass,
            false,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(again.status, "completed");
        sqlx::query("UPDATE native_goals SET bound_branch_id = 'other-branch' WHERE id = $1")
            .bind(&again.id)
            .execute(&pool)
            .await
            .unwrap();
        let moved = current_goal(&pool, "s-v").await.unwrap().unwrap();
        assert_eq!(moved.status, "active");
        assert!(moved.note.unwrap().contains("不能作为当前分支的完成证据"));
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
