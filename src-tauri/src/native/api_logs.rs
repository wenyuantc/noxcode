#![allow(dead_code)]

use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use tauri::{AppHandle, Runtime};

use crate::app::shared::{new_id, sqlite_pool};
use crate::db::models::{
    GetNativeUsageAnalyticsPayload, ListNativeApiCallLogsPayload, NativeApiCallLogDetail,
    NativeApiCallLogListItem, NativeApiCallLogPage, NativeApiCallLogStats, NativeUsageAnalytics,
    NativeUsageDailyBucket, NativeUsageModelBucket,
};
use crate::native::model::call_log::{redact_and_truncate_text, NativeApiCallLogInsert};

const LIST_API_CALL_LOGS_DEFAULT_LIMIT: i64 = 50;
const LIST_API_CALL_LOGS_MAX_LIMIT: i64 = 200;

const LIST_SELECT: &str = r#"
SELECT
    l.id, l.call_id, l.attempt, l.channel_id, l.channel_name, l.protocol, l.response_encoding,
    l.model, l.thinking_enabled, l.thinking_level, l.request_format, l.input_tokens,
    l.output_tokens, l.cached_tokens, l.total_tokens, l.first_token_ms, l.duration_ms,
    l.status, l.http_status, l.session_id, l.profile_id, l.workspace_id,
    NULL AS profile_name, w.name AS workspace_name, l.execution_target, l.call_kind, l.created_at
FROM native_api_call_logs l
LEFT JOIN workspaces w ON w.id = l.workspace_id
"#;

const DETAIL_SELECT: &str = r#"
SELECT
    l.id, l.call_id, l.attempt, l.channel_id, l.channel_name, l.protocol, l.response_encoding,
    l.model, l.thinking_enabled, l.thinking_level, l.request_format, l.request_body,
    l.request_truncated, l.response_body, l.response_truncated, l.input_tokens,
    l.output_tokens, l.cached_tokens, l.total_tokens, l.first_token_ms, l.duration_ms,
    l.status, l.http_status, l.error_message, l.session_id, l.profile_id, l.workspace_id,
    NULL AS profile_name, w.name AS workspace_name, l.subagent_id, l.call_kind,
    l.execution_target, l.created_at
FROM native_api_call_logs l
LEFT JOIN workspaces w ON w.id = l.workspace_id
"#;

const STATS_SELECT: &str = r#"
SELECT
    COUNT(*) AS total,
    COALESCE(SUM(CASE WHEN l.status = 'success' THEN 1 ELSE 0 END), 0) AS success,
    COALESCE(SUM(CASE WHEN l.status = 'failed' THEN 1 ELSE 0 END), 0) AS failed,
    COALESCE(SUM(CASE WHEN l.status = 'cancelled' THEN 1 ELSE 0 END), 0) AS cancelled,
    COALESCE(SUM(l.input_tokens), 0) AS input_tokens,
    COALESCE(SUM(l.output_tokens), 0) AS output_tokens,
    SUM(l.cached_tokens) AS cached_tokens_sum,
    SUM(l.total_tokens) AS total_tokens_sum,
    AVG(l.first_token_ms) AS avg_first_token_ms,
    AVG(l.duration_ms) AS avg_duration_ms
FROM native_api_call_logs l
LEFT JOIN workspaces w ON w.id = l.workspace_id
"#;

const DAILY_SELECT: &str = r#"
SELECT
    date(l.created_at) AS day,
    COUNT(*) AS calls,
    COALESCE(SUM(CASE WHEN l.status = 'success' THEN 1 ELSE 0 END), 0) AS success,
    COALESCE(SUM(CASE WHEN l.status = 'failed' THEN 1 ELSE 0 END), 0) AS failed,
    COALESCE(SUM(CASE WHEN l.status = 'cancelled' THEN 1 ELSE 0 END), 0) AS cancelled,
    COALESCE(SUM(l.input_tokens), 0) AS input_tokens,
    COALESCE(SUM(l.output_tokens), 0) AS output_tokens,
    COALESCE(SUM(l.cached_tokens), 0) AS cached_tokens,
    COALESCE(SUM(l.total_tokens), 0) AS total_tokens
FROM native_api_call_logs l
LEFT JOIN workspaces w ON w.id = l.workspace_id
"#;

const MODEL_SELECT: &str = r#"
SELECT
    COALESCE(NULLIF(trim(l.model), ''), '') AS model,
    COUNT(*) AS calls,
    COALESCE(SUM(l.input_tokens), 0) AS input_tokens,
    COALESCE(SUM(l.output_tokens), 0) AS output_tokens,
    COALESCE(SUM(l.cached_tokens), 0) AS cached_tokens,
    COALESCE(SUM(l.total_tokens), 0) AS total_tokens
FROM native_api_call_logs l
LEFT JOIN workspaces w ON w.id = l.workspace_id
"#;

pub fn spawn_insert_native_api_call_log(pool: SqlitePool, record: NativeApiCallLogInsert) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = insert_native_api_call_log(&pool, &record).await {
            eprintln!("[native] 写入 API 调用记录失败: {error}");
        }
    });
}

pub async fn insert_native_api_call_log(
    pool: &SqlitePool,
    record: &NativeApiCallLogInsert,
) -> Result<String, String> {
    let id = if record.id.trim().is_empty() {
        new_id()
    } else {
        record.id.clone()
    };
    let request = record.request_body.as_deref().map(redact_and_truncate_text);
    let response = record
        .response_body
        .as_deref()
        .map(redact_and_truncate_text);
    let error_message = record.error_message.as_deref().map(|text| {
        let redacted = redact_and_truncate_text(text);
        redacted.text
    });
    let request_truncated =
        record.request_truncated || request.as_ref().is_some_and(|item| item.truncated);
    let response_truncated =
        record.response_truncated || response.as_ref().is_some_and(|item| item.truncated);

    sqlx::query(
        r#"
        INSERT INTO native_api_call_logs (
            id, call_id, attempt, channel_id, channel_name, protocol, response_encoding,
            model, thinking_enabled, thinking_level, request_format, request_body,
            request_truncated, response_body, response_truncated, input_tokens,
            output_tokens, cached_tokens, total_tokens, first_token_ms, duration_ms,
            status, http_status, error_message, session_id, profile_id, workspace_id,
            subagent_id, call_kind, execution_target, operation, model_role
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            $8, $9, $10, $11, $12,
            $13, $14, $15, $16,
            $17, $18, $19, $20, $21,
            $22, $23, $24, $25, $26, $27,
            $28, $29, $30, $31, $32
        )
        "#,
    )
    .bind(&id)
    .bind(&record.call_id)
    .bind(record.attempt)
    .bind(record.channel_id.as_deref())
    .bind(record.channel_name.as_deref())
    .bind(&record.protocol)
    .bind(record.response_encoding.as_deref())
    .bind(record.model.as_deref())
    .bind(if record.thinking_enabled { 1 } else { 0 })
    .bind(record.thinking_level.as_deref())
    .bind(&record.request_format)
    .bind(request.as_ref().map(|item| item.text.as_str()))
    .bind(if request_truncated { 1 } else { 0 })
    .bind(response.as_ref().map(|item| item.text.as_str()))
    .bind(if response_truncated { 1 } else { 0 })
    .bind(record.input_tokens)
    .bind(record.output_tokens)
    .bind(record.cached_tokens)
    .bind(record.total_tokens)
    .bind(record.first_token_ms)
    .bind(record.duration_ms)
    .bind(&record.status)
    .bind(record.http_status)
    .bind(error_message.as_deref())
    .bind(record.session_id.as_deref())
    .bind(record.profile_id.as_deref())
    .bind(record.workspace_id.as_deref())
    .bind(record.subagent_id.as_deref())
    .bind(record.call_kind.as_deref())
    .bind(record.execution_target.as_deref())
    .bind(&record.operation)
    .bind(&record.model_role)
    .execute(pool)
    .await
    .map_err(|error| format!("Failed to insert native API call log: {error}"))?;

    Ok(id)
}

pub fn sqlite_call_log_sink(pool: SqlitePool) -> crate::native::model::client::CallLogSink {
    std::sync::Arc::new(move |record: NativeApiCallLogInsert| {
        spawn_insert_native_api_call_log(pool.clone(), record);
    })
}

fn empty_stats() -> NativeApiCallLogStats {
    NativeApiCallLogStats::default()
}

fn empty_page() -> NativeApiCallLogPage {
    NativeApiCallLogPage {
        items: Vec::new(),
        total: 0,
        stats: empty_stats(),
    }
}

fn empty_analytics() -> NativeUsageAnalytics {
    NativeUsageAnalytics {
        stats: empty_stats(),
        daily: Vec::new(),
        models: Vec::new(),
    }
}

fn analytics_list_payload(
    payload: &GetNativeUsageAnalyticsPayload,
) -> ListNativeApiCallLogsPayload {
    ListNativeApiCallLogsPayload {
        start_date: payload.start_date.clone(),
        end_date: payload.end_date.clone(),
        ..ListNativeApiCallLogsPayload::default()
    }
}

fn escape_sql_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn optional_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|item| !item.is_empty())
}

fn parse_activity_date_bound(date: &str, end_of_day: bool) -> Option<i64> {
    let trimmed = date.trim();
    if trimmed.is_empty() {
        return None;
    }
    let date_part = trimmed.split('T').next().unwrap_or(trimmed).trim();
    let naive_date = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()?;
    let naive_dt = if end_of_day {
        naive_date.and_hms_milli_opt(23, 59, 59, 999)?
    } else {
        naive_date.and_hms_opt(0, 0, 0)?
    };
    Some(naive_dt.and_utc().timestamp())
}

fn date_range_invalid(payload: &ListNativeApiCallLogsPayload) -> bool {
    let start = payload
        .start_date
        .as_deref()
        .and_then(|date| parse_activity_date_bound(date, false));
    let end = payload
        .end_date
        .as_deref()
        .and_then(|date| parse_activity_date_bound(date, true));
    matches!((start, end), (Some(start), Some(end)) if start > end)
}

fn push_and(builder: &mut QueryBuilder<'_, Sqlite>, has_where: &mut bool) {
    if *has_where {
        builder.push(" AND ");
    } else {
        builder.push(" WHERE ");
        *has_where = true;
    }
}

fn push_filters(builder: &mut QueryBuilder<'_, Sqlite>, payload: &ListNativeApiCallLogsPayload) {
    let mut has_where = false;
    if let Some(workspace_id) = optional_trimmed(payload.workspace_id.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("l.workspace_id = ");
        builder.push_bind(workspace_id.to_string());
    }
    if let Some(profile_id) = optional_trimmed(payload.profile_id.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("l.profile_id = ");
        builder.push_bind(profile_id.to_string());
    }
    if let Some(execution_target) = optional_trimmed(payload.execution_target.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("LOWER(COALESCE(l.execution_target, 'local')) = ");
        builder.push_bind(execution_target.to_ascii_lowercase());
    }
    if let Some(session_id) = optional_trimmed(payload.session_id.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("l.session_id = ");
        builder.push_bind(session_id.to_string());
    }
    if let Some(channel_name) = optional_trimmed(payload.channel_name.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("LOWER(COALESCE(l.channel_name, '')) LIKE ");
        builder.push_bind(format!(
            "%{}%",
            escape_sql_like(&channel_name.to_ascii_lowercase())
        ));
        builder.push(" ESCAPE '\\'");
    }
    if let Some(model) = optional_trimmed(payload.model.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("LOWER(COALESCE(l.model, '')) LIKE ");
        builder.push_bind(format!(
            "%{}%",
            escape_sql_like(&model.to_ascii_lowercase())
        ));
        builder.push(" ESCAPE '\\'");
    }
    if let Some(status) = optional_trimmed(payload.status.as_deref()) {
        push_and(builder, &mut has_where);
        builder.push("l.status = ");
        builder.push_bind(status.to_string());
    }
    if let Some(start) = payload
        .start_date
        .as_deref()
        .and_then(|date| parse_activity_date_bound(date, false))
    {
        push_and(builder, &mut has_where);
        builder.push("CAST(strftime('%s', l.created_at) AS INTEGER) >= ");
        builder.push_bind(start);
    }
    if let Some(end) = payload
        .end_date
        .as_deref()
        .and_then(|date| parse_activity_date_bound(date, true))
    {
        push_and(builder, &mut has_where);
        builder.push("CAST(strftime('%s', l.created_at) AS INTEGER) <= ");
        builder.push_bind(end);
    }
}

pub(crate) async fn list_native_api_call_logs_with_pool(
    pool: &SqlitePool,
    payload: &ListNativeApiCallLogsPayload,
) -> Result<NativeApiCallLogPage, String> {
    if date_range_invalid(payload) {
        return Ok(empty_page());
    }

    let limit = payload
        .limit
        .filter(|value| *value > 0)
        .unwrap_or(LIST_API_CALL_LOGS_DEFAULT_LIMIT)
        .clamp(1, LIST_API_CALL_LOGS_MAX_LIMIT);
    let offset = payload.offset.unwrap_or(0).max(0);
    let include_total = payload.include_total.unwrap_or(true);

    let mut items_q = QueryBuilder::<Sqlite>::new(LIST_SELECT);
    push_filters(&mut items_q, payload);
    items_q.push(" ORDER BY l.created_at DESC, l.id DESC LIMIT ");
    items_q.push_bind(limit);
    items_q.push(" OFFSET ");
    items_q.push_bind(offset);
    let items: Vec<NativeApiCallLogListItem> = items_q
        .build_query_as::<NativeApiCallLogListItem>()
        .fetch_all(pool)
        .await
        .map_err(|error| format!("获取 API 调用记录失败: {error}"))?;

    let mut stats_q = QueryBuilder::<Sqlite>::new(STATS_SELECT);
    push_filters(&mut stats_q, payload);
    let stats: NativeApiCallLogStats = stats_q
        .build_query_as::<NativeApiCallLogStats>()
        .fetch_one(pool)
        .await
        .map_err(|error| format!("统计 API 调用记录失败: {error}"))?;
    let total = if include_total { stats.total } else { 0 };

    Ok(NativeApiCallLogPage {
        items,
        total,
        stats,
    })
}

pub(crate) async fn get_native_api_call_log_with_pool(
    pool: &SqlitePool,
    id: &str,
) -> Result<NativeApiCallLogDetail, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("API 调用记录 ID 不能为空".to_string());
    }
    let mut query = QueryBuilder::<Sqlite>::new(DETAIL_SELECT);
    query.push(" WHERE l.id = ");
    query.push_bind(id.to_string());
    query
        .build_query_as::<NativeApiCallLogDetail>()
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取 API 调用记录失败: {error}"))?
        .ok_or_else(|| "API 调用记录不存在".to_string())
}

pub(crate) async fn get_native_usage_analytics_with_pool(
    pool: &SqlitePool,
    payload: &GetNativeUsageAnalyticsPayload,
) -> Result<NativeUsageAnalytics, String> {
    let filters = analytics_list_payload(payload);
    if date_range_invalid(&filters) {
        return Ok(empty_analytics());
    }

    let mut stats_q = QueryBuilder::<Sqlite>::new(STATS_SELECT);
    push_filters(&mut stats_q, &filters);
    let stats: NativeApiCallLogStats = stats_q
        .build_query_as::<NativeApiCallLogStats>()
        .fetch_one(pool)
        .await
        .map_err(|error| format!("统计使用数据失败: {error}"))?;

    let mut daily_q = QueryBuilder::<Sqlite>::new(DAILY_SELECT);
    push_filters(&mut daily_q, &filters);
    daily_q.push(" GROUP BY date(l.created_at) ORDER BY date(l.created_at) ASC");
    let daily: Vec<NativeUsageDailyBucket> = daily_q
        .build_query_as::<NativeUsageDailyBucket>()
        .fetch_all(pool)
        .await
        .map_err(|error| format!("统计每日用量失败: {error}"))?;

    let mut models_q = QueryBuilder::<Sqlite>::new(MODEL_SELECT);
    push_filters(&mut models_q, &filters);
    models_q.push(
        " GROUP BY COALESCE(NULLIF(trim(l.model), ''), '') ORDER BY total_tokens DESC, calls DESC, model ASC",
    );
    let models: Vec<NativeUsageModelBucket> = models_q
        .build_query_as::<NativeUsageModelBucket>()
        .fetch_all(pool)
        .await
        .map_err(|error| format!("统计模型用量失败: {error}"))?;

    Ok(NativeUsageAnalytics {
        stats,
        daily,
        models,
    })
}

#[tauri::command]
pub async fn list_native_api_call_logs<R: Runtime>(
    app: AppHandle<R>,
    payload: Option<ListNativeApiCallLogsPayload>,
) -> Result<NativeApiCallLogPage, String> {
    let pool = sqlite_pool(&app).await?;
    let payload = payload.unwrap_or_default();
    list_native_api_call_logs_with_pool(&pool, &payload).await
}

#[tauri::command]
pub async fn get_native_api_call_log<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<NativeApiCallLogDetail, String> {
    let pool = sqlite_pool(&app).await?;
    get_native_api_call_log_with_pool(&pool, &id).await
}

#[tauri::command]
pub async fn get_native_usage_analytics<R: Runtime>(
    app: AppHandle<R>,
    payload: Option<GetNativeUsageAnalyticsPayload>,
) -> Result<NativeUsageAnalytics, String> {
    let pool = sqlite_pool(&app).await?;
    let payload = payload.unwrap_or_default();
    get_native_usage_analytics_with_pool(&pool, &payload).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::setup_migrated_pool;
    use crate::native::model::call_log::{CALL_KIND_CHAT, CALL_STATUS_FAILED, CALL_STATUS_SUCCESS};

    fn sample_record() -> NativeApiCallLogInsert {
        NativeApiCallLogInsert {
            id: "log-1".to_string(),
            call_id: "call-1".to_string(),
            attempt: 1,
            channel_id: Some("ch-1".to_string()),
            channel_name: Some("OpenAI".to_string()),
            protocol: "openai".to_string(),
            response_encoding: Some("sse".to_string()),
            model: Some("gpt-4o".to_string()),
            thinking_enabled: false,
            thinking_level: None,
            request_format: "openai".to_string(),
            request_body: Some(r#"{"model":"gpt-4o"}"#.to_string()),
            request_truncated: false,
            response_body: Some(r#"{"choices":[{"delta":{"content":"ok"}}]}"#.to_string()),
            response_truncated: false,
            input_tokens: Some(10),
            output_tokens: Some(4),
            cached_tokens: Some(3),
            total_tokens: Some(14),
            first_token_ms: Some(12),
            duration_ms: Some(40),
            status: CALL_STATUS_SUCCESS.to_string(),
            http_status: Some(200),
            error_message: None,
            session_id: Some("sess-1".to_string()),
            profile_id: Some("prof-1".to_string()),
            workspace_id: Some("ws-1".to_string()),
            subagent_id: None,
            call_kind: Some(CALL_KIND_CHAT.to_string()),
            execution_target: Some("local".to_string()),
            operation: "agent_step".to_string(),
            model_role: "main".to_string(),
        }
    }

    #[tokio::test]
    async fn insert_stores_nullable_tokens_and_cached_usage() {
        let pool = setup_migrated_pool().await;
        insert_native_api_call_log(&pool, &sample_record())
            .await
            .expect("insert");
        let row = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<i64>, String)>(
            "SELECT input_tokens, output_tokens, cached_tokens, total_tokens, status FROM native_api_call_logs WHERE id = 'log-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("read row");
        assert_eq!(row.0, Some(10));
        assert_eq!(row.1, Some(4));
        assert_eq!(row.2, Some(3));
        assert_eq!(row.3, Some(14));
        assert_eq!(row.4, CALL_STATUS_SUCCESS);
    }

    #[tokio::test]
    async fn list_filters_by_profile_and_workspace() {
        let pool = setup_migrated_pool().await;
        insert_native_api_call_log(&pool, &sample_record())
            .await
            .expect("insert");
        let mut other = sample_record();
        other.id = "log-2".to_string();
        other.profile_id = Some("prof-2".to_string());
        other.workspace_id = Some("ws-2".to_string());
        insert_native_api_call_log(&pool, &other)
            .await
            .expect("insert other");

        let page = list_native_api_call_logs_with_pool(
            &pool,
            &ListNativeApiCallLogsPayload {
                profile_id: Some("prof-1".to_string()),
                workspace_id: Some("ws-1".to_string()),
                ..ListNativeApiCallLogsPayload::default()
            },
        )
        .await
        .expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "log-1");
        assert_eq!(page.stats.success, 1);
        assert_eq!(page.stats.cached_tokens_sum, Some(3));
        assert_eq!(page.stats.total_tokens_sum, Some(14));
        assert_eq!(page.stats.avg_first_token_ms, Some(12.0));
        assert_eq!(page.stats.avg_duration_ms, Some(40.0));
        let detail = get_native_api_call_log_with_pool(&pool, "log-1")
            .await
            .expect("detail");
        assert_eq!(detail.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn parse_activity_date_bound_covers_day() {
        let start = parse_activity_date_bound("2026-09-02", false).expect("start");
        let end = parse_activity_date_bound("2026-09-02", true).expect("end");
        assert!(end > start);
        assert!(parse_activity_date_bound("not-a-date", false).is_none());
    }

    async fn set_created_at(pool: &SqlitePool, id: &str, created_at: &str) {
        sqlx::query("UPDATE native_api_call_logs SET created_at = $1 WHERE id = $2")
            .bind(created_at)
            .bind(id)
            .execute(pool)
            .await
            .expect("update created_at");
    }

    async fn insert_dated(
        pool: &SqlitePool,
        record: NativeApiCallLogInsert,
        created_at: &str,
    ) -> String {
        let id = insert_native_api_call_log(pool, &record)
            .await
            .expect("insert");
        set_created_at(pool, &id, created_at).await;
        id
    }

    #[tokio::test]
    async fn usage_analytics_groups_daily_and_models() {
        let pool = setup_migrated_pool().await;

        let mut day1_main = sample_record();
        day1_main.id = "log-d1-main".to_string();
        day1_main.call_id = "call-d1-main".to_string();
        insert_dated(&pool, day1_main, "2026-09-01 10:00:00").await;

        let mut day1_failed = sample_record();
        day1_failed.id = "log-d1-fail".to_string();
        day1_failed.call_id = "call-d1-fail".to_string();
        day1_failed.model = Some("claude-4".to_string());
        day1_failed.status = CALL_STATUS_FAILED.to_string();
        day1_failed.input_tokens = Some(20);
        day1_failed.output_tokens = Some(0);
        day1_failed.cached_tokens = None;
        day1_failed.total_tokens = Some(20);
        insert_dated(&pool, day1_failed, "2026-09-01 18:00:00").await;

        let mut day2 = sample_record();
        day2.id = "log-d2".to_string();
        day2.call_id = "call-d2".to_string();
        day2.input_tokens = Some(100);
        day2.output_tokens = Some(40);
        day2.cached_tokens = Some(25);
        day2.total_tokens = Some(140);
        insert_dated(&pool, day2, "2026-09-02 09:00:00").await;

        let mut outside = sample_record();
        outside.id = "log-out".to_string();
        outside.call_id = "call-out".to_string();
        outside.model = Some("outside".to_string());
        insert_dated(&pool, outside, "2026-08-01 09:00:00").await;

        let mut unnamed = sample_record();
        unnamed.id = "log-blank".to_string();
        unnamed.call_id = "call-blank".to_string();
        unnamed.model = Some("   ".to_string());
        unnamed.input_tokens = Some(5);
        unnamed.output_tokens = Some(1);
        unnamed.cached_tokens = Some(0);
        unnamed.total_tokens = Some(6);
        insert_dated(&pool, unnamed, "2026-09-02 22:00:00").await;

        let analytics = get_native_usage_analytics_with_pool(
            &pool,
            &GetNativeUsageAnalyticsPayload {
                start_date: Some("2026-09-01".to_string()),
                end_date: Some("2026-09-02".to_string()),
            },
        )
        .await
        .expect("analytics");

        assert_eq!(analytics.stats.total, 4);
        assert_eq!(analytics.stats.success, 3);
        assert_eq!(analytics.stats.failed, 1);
        assert_eq!(analytics.stats.input_tokens, 135);
        assert_eq!(analytics.stats.output_tokens, 45);
        assert_eq!(analytics.stats.cached_tokens_sum, Some(28));
        assert_eq!(analytics.stats.total_tokens_sum, Some(180));

        assert_eq!(analytics.daily.len(), 2);
        assert_eq!(analytics.daily[0].day, "2026-09-01");
        assert_eq!(analytics.daily[0].calls, 2);
        assert_eq!(analytics.daily[0].success, 1);
        assert_eq!(analytics.daily[0].failed, 1);
        assert_eq!(analytics.daily[0].input_tokens, 30);
        assert_eq!(analytics.daily[0].output_tokens, 4);
        assert_eq!(analytics.daily[0].cached_tokens, 3);
        assert_eq!(analytics.daily[0].total_tokens, 34);
        assert_eq!(analytics.daily[1].day, "2026-09-02");
        assert_eq!(analytics.daily[1].calls, 2);
        assert_eq!(analytics.daily[1].total_tokens, 146);

        assert_eq!(analytics.models.len(), 3);
        assert_eq!(analytics.models[0].model, "gpt-4o");
        assert_eq!(analytics.models[0].calls, 2);
        assert_eq!(analytics.models[0].total_tokens, 154);
        assert_eq!(analytics.models[1].model, "claude-4");
        assert_eq!(analytics.models[1].calls, 1);
        assert_eq!(analytics.models[1].total_tokens, 20);
        assert_eq!(analytics.models[2].model, "");
        assert_eq!(analytics.models[2].calls, 1);
        assert_eq!(analytics.models[2].total_tokens, 6);
    }

    #[tokio::test]
    async fn usage_analytics_empty_when_no_rows_in_range() {
        let pool = setup_migrated_pool().await;
        insert_dated(&pool, sample_record(), "2026-08-01 10:00:00").await;

        let analytics = get_native_usage_analytics_with_pool(
            &pool,
            &GetNativeUsageAnalyticsPayload {
                start_date: Some("2026-09-01".to_string()),
                end_date: Some("2026-09-02".to_string()),
            },
        )
        .await
        .expect("analytics");

        assert_eq!(analytics.stats.total, 0);
        assert!(analytics.daily.is_empty());
        assert!(analytics.models.is_empty());
    }

    #[tokio::test]
    async fn usage_analytics_returns_empty_for_inverted_range() {
        let pool = setup_migrated_pool().await;
        insert_dated(&pool, sample_record(), "2026-09-01 10:00:00").await;

        let analytics = get_native_usage_analytics_with_pool(
            &pool,
            &GetNativeUsageAnalyticsPayload {
                start_date: Some("2026-09-03".to_string()),
                end_date: Some("2026-09-01".to_string()),
            },
        )
        .await
        .expect("analytics");

        assert_eq!(analytics.stats.total, 0);
        assert!(analytics.daily.is_empty());
        assert!(analytics.models.is_empty());
    }
}
