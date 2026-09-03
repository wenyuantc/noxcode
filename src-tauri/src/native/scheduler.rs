//! Cron 自动化：按计划在工作区启动内置 Agent 会话。
//!
//! 五段 cron（分 时 日 月 周，支持 `*`、`a,b`、`a-b`、`*/n`、`a-b/n` 与 `@hourly` 等别名），
//! 本地时区计算 `next_run_at`。调度器每 30 秒扫描一次到期的自动化；同一工作区已有会话在
//! 工作中时推迟 1 分钟，避免并发改同一份代码。

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tauri::{AppHandle, Runtime};
use tokio::sync::Mutex;

use crate::app::shared::{new_id, now_sqlite, sqlite_pool, SQLITE_DATETIME_FORMAT};
use crate::db::models::StartNativeSessionInput;
use crate::native::manager::NativeAgentManager;

const SCAN_INTERVAL: Duration = Duration::from_secs(30);
const DEFER_WHEN_BUSY_SECS: i64 = 60;
const MAX_AUTOMATIONS_PER_WORKSPACE: usize = 50;
const SEARCH_HORIZON_DAYS: i64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NativeAutomation {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub timezone: Option<String>,
    pub enabled: i64,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub last_session_id: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNativeAutomation {
    pub workspace_id: String,
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateNativeAutomation {
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub cron: Option<String>,
    pub channel_id: Option<String>,
    pub model: Option<String>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// cron 解析
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
    /// 日与周都被限定时按 POSIX 语义取「或」。
    day_restricted: bool,
    weekday_restricted: bool,
}

fn expand_alias(expr: &str) -> &str {
    match expr.trim() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        other => other,
    }
}

fn parse_field(field: &str, min: u32, max: u32, names: &[&str]) -> Result<Vec<u32>, String> {
    let mut values = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("cron 字段为空：{field}"));
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (
                range,
                step.parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| format!("cron 步长无效：{part}"))?,
            ),
            None => (part, 1),
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                parse_value(a, min, max, names)?,
                parse_value(b, min, max, names)?,
            )
        } else {
            let value = parse_value(range, min, max, names)?;
            if step > 1 {
                (value, max)
            } else {
                (value, value)
            }
        };
        if start > end {
            return Err(format!("cron 区间起点大于终点：{part}"));
        }
        let mut current = start;
        while current <= end {
            values.push(current);
            current += step;
        }
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

fn parse_value(text: &str, min: u32, max: u32, names: &[&str]) -> Result<u32, String> {
    let lower = text.trim().to_ascii_lowercase();
    if let Some(index) = names
        .iter()
        .position(|name| lower.starts_with(&name.to_ascii_lowercase()))
    {
        return Ok(index as u32 + min);
    }
    let value = lower
        .parse::<u32>()
        .map_err(|_| format!("cron 值无效：{text}"))?;
    // 周日允许写 7。
    let value = if max == 6 && value == 7 { 0 } else { value };
    if value < min || value > max {
        return Err(format!("cron 值超出范围 {min}-{max}：{text}"));
    }
    Ok(value)
}

const MONTH_NAMES: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAY_NAMES: &[&str] = &["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

pub fn parse_cron(expr: &str) -> Result<CronSchedule, String> {
    let expanded = expand_alias(expr);
    let fields: Vec<&str> = expanded.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "cron 需要 5 段（分 时 日 月 周），收到 {} 段：{expr}",
            fields.len()
        ));
    }
    Ok(CronSchedule {
        minutes: parse_field(fields[0], 0, 59, &[])?,
        hours: parse_field(fields[1], 0, 23, &[])?,
        days: parse_field(fields[2], 1, 31, &[])?,
        months: parse_field(fields[3], 1, 12, MONTH_NAMES)?,
        weekdays: parse_field(fields[4], 0, 6, WEEKDAY_NAMES)?,
        day_restricted: fields[2] != "*",
        weekday_restricted: fields[4] != "*",
    })
}

impl CronSchedule {
    fn day_matches(&self, time: &DateTime<Local>) -> bool {
        let day_ok = self.days.contains(&time.day());
        let weekday_ok = self
            .weekdays
            .contains(&(time.weekday().num_days_from_sunday()));
        match (self.day_restricted, self.weekday_restricted) {
            (true, true) => day_ok || weekday_ok,
            (true, false) => day_ok,
            (false, true) => weekday_ok,
            (false, false) => true,
        }
    }

    /// 严格晚于 `after` 的下一次触发时间。
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut cursor = after
            .with_second(0)?
            .with_nanosecond(0)?
            .checked_add_signed(chrono::Duration::minutes(1))?;
        let horizon = after.checked_add_signed(chrono::Duration::days(SEARCH_HORIZON_DAYS))?;
        while cursor <= horizon {
            if !self.months.contains(&cursor.month()) || !self.day_matches(&cursor) {
                // 跳到第二天零点。
                let next_day = cursor.date_naive().succ_opt()?.and_hms_opt(0, 0, 0)?;
                cursor = Local.from_local_datetime(&next_day).earliest()?;
                continue;
            }
            if !self.hours.contains(&cursor.hour()) {
                cursor = cursor
                    .with_minute(0)?
                    .checked_add_signed(chrono::Duration::hours(1))?;
                continue;
            }
            if !self.minutes.contains(&cursor.minute()) {
                cursor = cursor.checked_add_signed(chrono::Duration::minutes(1))?;
                continue;
            }
            return Some(cursor);
        }
        None
    }
}

fn format_local(time: &DateTime<Local>) -> String {
    time.format(SQLITE_DATETIME_FORMAT).to_string()
}

fn parse_local(text: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(text.trim(), SQLITE_DATETIME_FORMAT).ok()?;
    Local.from_local_datetime(&naive).earliest()
}

pub fn compute_next_run(cron: &str, after: DateTime<Local>) -> Result<Option<String>, String> {
    let schedule = parse_cron(cron)?;
    Ok(schedule.next_after(after).map(|time| format_local(&time)))
}

// ---------------------------------------------------------------------------
// 持久化
// ---------------------------------------------------------------------------

pub async fn list_automations(
    pool: &SqlitePool,
    workspace_id: Option<&str>,
) -> Result<Vec<NativeAutomation>, String> {
    let rows =
        match workspace_id {
            Some(workspace_id) => sqlx::query_as::<_, NativeAutomation>(
                "SELECT * FROM native_automations WHERE workspace_id = $1 ORDER BY created_at ASC",
            )
            .bind(workspace_id)
            .fetch_all(pool)
            .await,
            None => {
                sqlx::query_as::<_, NativeAutomation>(
                    "SELECT * FROM native_automations ORDER BY created_at ASC",
                )
                .fetch_all(pool)
                .await
            }
        };
    rows.map_err(|error| format!("读取自动化失败: {error}"))
}

pub async fn get_automation(pool: &SqlitePool, id: &str) -> Result<NativeAutomation, String> {
    sqlx::query_as::<_, NativeAutomation>("SELECT * FROM native_automations WHERE id = $1 LIMIT 1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取自动化失败: {error}"))?
        .ok_or_else(|| format!("自动化不存在: {id}"))
}

pub async fn create_automation(
    pool: &SqlitePool,
    payload: CreateNativeAutomation,
) -> Result<NativeAutomation, String> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err("自动化名称不能为空".to_string());
    }
    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err("自动化提示词不能为空".to_string());
    }
    let cron = payload.cron.trim();
    parse_cron(cron)?;
    let existing = list_automations(pool, Some(&payload.workspace_id)).await?;
    if existing.len() >= MAX_AUTOMATIONS_PER_WORKSPACE {
        return Err(format!(
            "每个工作区最多 {MAX_AUTOMATIONS_PER_WORKSPACE} 条自动化"
        ));
    }
    let enabled = payload.enabled.unwrap_or(true);
    let next_run = if enabled {
        compute_next_run(cron, Local::now())?
    } else {
        None
    };
    let id = new_id();
    let now = now_sqlite();
    sqlx::query(
        "INSERT INTO native_automations (id, workspace_id, name, prompt, cron, enabled, channel_id, model, next_run_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&id)
    .bind(&payload.workspace_id)
    .bind(name)
    .bind(prompt)
    .bind(cron)
    .bind(i64::from(enabled))
    .bind(payload.channel_id.as_deref().map(str::trim).filter(|item| !item.is_empty()))
    .bind(payload.model.as_deref().map(str::trim).filter(|item| !item.is_empty()))
    .bind(&next_run)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|error| format!("创建自动化失败: {error}"))?;
    get_automation(pool, &id).await
}

pub async fn update_automation(
    pool: &SqlitePool,
    id: &str,
    updates: UpdateNativeAutomation,
) -> Result<NativeAutomation, String> {
    let current = get_automation(pool, id).await?;
    let name = updates
        .name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(current.name);
    let prompt = updates
        .prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(current.prompt);
    let cron = updates
        .cron
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(current.cron);
    parse_cron(&cron)?;
    let enabled = updates.enabled.unwrap_or(current.enabled != 0);
    let channel_id = match updates.channel_id {
        Some(value) => Some(value).filter(|item| !item.trim().is_empty()),
        None => current.channel_id,
    };
    let model = match updates.model {
        Some(value) => Some(value).filter(|item| !item.trim().is_empty()),
        None => current.model,
    };
    let next_run = if enabled {
        compute_next_run(&cron, Local::now())?
    } else {
        None
    };
    sqlx::query(
        "UPDATE native_automations SET name = $1, prompt = $2, cron = $3, enabled = $4, channel_id = $5, model = $6, next_run_at = $7, updated_at = $8 WHERE id = $9",
    )
    .bind(&name)
    .bind(&prompt)
    .bind(&cron)
    .bind(i64::from(enabled))
    .bind(&channel_id)
    .bind(&model)
    .bind(&next_run)
    .bind(now_sqlite())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新自动化失败: {error}"))?;
    get_automation(pool, id).await
}

pub async fn delete_automation(pool: &SqlitePool, id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM native_automations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| format!("删除自动化失败: {error}"))?;
    Ok(result.rows_affected() > 0)
}

async fn mark_run(
    pool: &SqlitePool,
    automation: &NativeAutomation,
    session_id: Option<&str>,
    error: Option<&str>,
    next_run: Option<String>,
) {
    let _ = sqlx::query(
        "UPDATE native_automations SET last_run_at = $1, next_run_at = $2, last_session_id = COALESCE($3, last_session_id), last_error = $4, updated_at = $5 WHERE id = $6",
    )
    .bind(now_sqlite())
    .bind(&next_run)
    .bind(session_id)
    .bind(error)
    .bind(now_sqlite())
    .bind(&automation.id)
    .execute(pool)
    .await;
}

async fn defer(pool: &SqlitePool, automation: &NativeAutomation) {
    let next = Local::now() + chrono::Duration::seconds(DEFER_WHEN_BUSY_SECS);
    let _ = sqlx::query("UPDATE native_automations SET next_run_at = $1 WHERE id = $2")
        .bind(format_local(&next))
        .bind(&automation.id)
        .execute(pool)
        .await;
}

/// 用自动化配置启动一次会话（调度到期或用户点击「立即运行」）。
pub async fn run_automation_now(
    app: &AppHandle,
    manager: &Arc<Mutex<NativeAgentManager>>,
    automation: &NativeAutomation,
) -> Result<String, String> {
    let pool = sqlite_pool(app).await?;
    let channel_id = match automation.channel_id.as_deref() {
        Some(id) if !id.trim().is_empty() => id.to_string(),
        _ => {
            let record = sqlx::query_scalar::<_, String>(
                "SELECT id FROM ai_channels WHERE enabled = 1 ORDER BY updated_at DESC LIMIT 1",
            )
            .fetch_optional(&pool)
            .await
            .map_err(|error| format!("读取渠道失败: {error}"))?;
            record.ok_or_else(|| "没有可用的 AI 渠道".to_string())?
        }
    };
    let prompt = format!(
        "[自动化 {}]\n{}",
        automation.name.trim(),
        automation.prompt.trim()
    );
    let started = crate::native::session::start_native_with_manager(
        app.clone(),
        manager.clone(),
        StartNativeSessionInput {
            ai_channel_id: channel_id,
            workspace_id: automation.workspace_id.clone(),
            prompt,
            model: automation.model.clone(),
            reasoning_effort: None,
            system_prompt: None,
            resume_session_id: None,
            image_paths: None,
            plan_mode: Some(false),
        },
    )
    .await?;
    Ok(started.session_record_id)
}

/// 单次扫描：启动到期的自动化。返回启动的会话数。
pub async fn scan_once(app: &AppHandle, manager: &Arc<Mutex<NativeAgentManager>>) -> usize {
    let Ok(pool) = sqlite_pool(app).await else {
        return 0;
    };
    let Ok(items) = list_automations(&pool, None).await else {
        return 0;
    };
    let now = Local::now();
    let mut started = 0;
    for automation in items.into_iter().filter(|item| item.enabled != 0) {
        let due = match automation.next_run_at.as_deref().and_then(parse_local) {
            Some(next) => next <= now,
            None => {
                // 缺 next_run_at 的旧行：补算后下次再跑。
                if let Ok(next) = compute_next_run(&automation.cron, now) {
                    let _ =
                        sqlx::query("UPDATE native_automations SET next_run_at = $1 WHERE id = $2")
                            .bind(&next)
                            .bind(&automation.id)
                            .execute(&pool)
                            .await;
                }
                false
            }
        };
        if !due {
            continue;
        }
        let busy = manager
            .lock()
            .await
            .has_working_workspace_processes(&automation.workspace_id);
        if busy {
            defer(&pool, &automation).await;
            continue;
        }
        let next_run = compute_next_run(&automation.cron, now).ok().flatten();
        match run_automation_now(app, manager, &automation).await {
            Ok(session_id) => {
                started += 1;
                mark_run(&pool, &automation, Some(&session_id), None, next_run).await;
            }
            Err(error) => {
                eprintln!("[scheduler] 自动化 {} 启动失败: {error}", automation.name);
                mark_run(&pool, &automation, None, Some(&error), next_run).await;
            }
        }
    }
    started
}

/// 后台调度循环；在应用 setup 时启动。
pub fn spawn_scheduler(app: AppHandle, manager: Arc<Mutex<NativeAgentManager>>) {
    tauri::async_runtime::spawn(async move {
        // 给数据库插件与主窗口一点启动时间。
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            let _ = scan_once(&app, &manager).await;
            tokio::time::sleep(SCAN_INTERVAL).await;
        }
    });
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_native_automations<R: Runtime>(
    app: AppHandle<R>,
    workspace_id: Option<String>,
) -> Result<Vec<NativeAutomation>, String> {
    let pool = sqlite_pool(&app).await?;
    list_automations(&pool, workspace_id.as_deref()).await
}

#[tauri::command]
pub async fn create_native_automation<R: Runtime>(
    app: AppHandle<R>,
    payload: CreateNativeAutomation,
) -> Result<NativeAutomation, String> {
    let pool = sqlite_pool(&app).await?;
    create_automation(&pool, payload).await
}

#[tauri::command]
pub async fn update_native_automation<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    updates: UpdateNativeAutomation,
) -> Result<NativeAutomation, String> {
    let pool = sqlite_pool(&app).await?;
    update_automation(&pool, &id, updates).await
}

#[tauri::command]
pub async fn delete_native_automation<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<bool, String> {
    let pool = sqlite_pool(&app).await?;
    delete_automation(&pool, &id).await
}

#[tauri::command]
pub async fn run_native_automation_now(
    app: AppHandle,
    state: tauri::State<'_, Arc<Mutex<NativeAgentManager>>>,
    id: String,
) -> Result<String, String> {
    let pool = sqlite_pool(&app).await?;
    let automation = get_automation(&pool, &id).await?;
    let manager = state.inner().clone();
    let session_id = run_automation_now(&app, &manager, &automation).await?;
    let next_run = compute_next_run(&automation.cron, Local::now())
        .ok()
        .flatten();
    mark_run(&pool, &automation, Some(&session_id), None, next_run).await;
    Ok(session_id)
}

/// 工具 CronList 的文本表示。
pub fn format_automation_list(items: &[NativeAutomation]) -> String {
    if items.is_empty() {
        return "当前工作区没有自动化。".to_string();
    }
    items
        .iter()
        .map(|item| {
            format!(
                "- {} [{}] cron=`{}` {} 下次：{} 上次：{}{}",
                item.id,
                item.name,
                item.cron,
                if item.enabled != 0 {
                    "启用"
                } else {
                    "停用"
                },
                item.next_run_at.as_deref().unwrap_or("-"),
                item.last_run_at.as_deref().unwrap_or("-"),
                item.last_error
                    .as_deref()
                    .map(|error| format!(" 错误：{error}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(y, m, d)
                    .unwrap()
                    .and_hms_opt(h, min, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
    }

    #[test]
    fn parses_fields_steps_ranges_and_aliases() {
        let schedule = parse_cron("*/15 9-17 * * mon-fri").expect("parse");
        assert_eq!(schedule.minutes, vec![0, 15, 30, 45]);
        assert_eq!(schedule.hours, (9..=17).collect::<Vec<_>>());
        assert_eq!(schedule.weekdays, vec![1, 2, 3, 4, 5]);
        assert!(!schedule.day_restricted && schedule.weekday_restricted);
        let daily = parse_cron("@daily").expect("alias");
        assert_eq!(daily.minutes, vec![0]);
        assert_eq!(daily.hours, vec![0]);
        assert!(parse_cron("* * *").is_err());
        assert!(parse_cron("61 * * * *").is_err());
        assert!(parse_cron("*/0 * * * *").is_err());
        let sunday = parse_cron("0 0 * * 7").expect("sunday");
        assert_eq!(sunday.weekdays, vec![0]);
    }

    #[test]
    fn next_after_walks_forward_correctly() {
        // 2026-09-03 是周四。
        let after = local(2026, 9, 3, 10, 7);
        let every_15 = parse_cron("*/15 * * * *").unwrap();
        assert_eq!(every_15.next_after(after), Some(local(2026, 9, 3, 10, 15)));
        let weekday_morning = parse_cron("30 9 * * mon-fri").unwrap();
        assert_eq!(
            weekday_morning.next_after(after),
            Some(local(2026, 9, 4, 9, 30))
        );
        // 周五 09:30 之后 → 下周一。
        assert_eq!(
            weekday_morning.next_after(local(2026, 9, 4, 9, 30)),
            Some(local(2026, 9, 7, 9, 30))
        );
        let monthly = parse_cron("0 0 1 * *").unwrap();
        assert_eq!(monthly.next_after(after), Some(local(2026, 10, 1, 0, 0)));
        let text = compute_next_run("0 12 * * *", after).unwrap().unwrap();
        assert_eq!(text, "2026-09-03 12:00:00");
        assert!(parse_local(&text).is_some());
    }

    #[tokio::test]
    async fn automation_crud_round_trip() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-1', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("workspace");
        let created = create_automation(
            &pool,
            CreateNativeAutomation {
                workspace_id: "ws-1".to_string(),
                name: " 每日体检 ".to_string(),
                prompt: "跑一遍测试并汇报".to_string(),
                cron: "0 9 * * *".to_string(),
                channel_id: None,
                model: None,
                enabled: None,
            },
        )
        .await
        .expect("create");
        assert_eq!(created.name, "每日体检");
        assert!(created.next_run_at.is_some());
        assert_eq!(created.enabled, 1);
        let bad = create_automation(
            &pool,
            CreateNativeAutomation {
                workspace_id: "ws-1".to_string(),
                name: "x".to_string(),
                prompt: "y".to_string(),
                cron: "nope".to_string(),
                channel_id: None,
                model: None,
                enabled: None,
            },
        )
        .await;
        assert!(bad.is_err());
        let updated = update_automation(
            &pool,
            &created.id,
            UpdateNativeAutomation {
                enabled: Some(false),
                ..UpdateNativeAutomation::default()
            },
        )
        .await
        .expect("update");
        assert_eq!(updated.enabled, 0);
        assert!(updated.next_run_at.is_none());
        let listed = list_automations(&pool, Some("ws-1")).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert!(format_automation_list(&listed).contains("每日体检"));
        assert!(delete_automation(&pool, &created.id).await.expect("delete"));
        assert!(!delete_automation(&pool, &created.id)
            .await
            .expect("delete again"));
        assert_eq!(format_automation_list(&[]), "当前工作区没有自动化。");
    }
}
