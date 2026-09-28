#[allow(unused_imports)]
use super::*;

#[tauri::command]
pub async fn finish_native_input(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<(), String> {
    finish_live_input(state.inner(), &session_record_id).await
}

#[tauri::command]
pub async fn update_native_session_configuration(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: UpdateNativeSessionConfigurationInput,
) -> Result<NativeSessionConfigurationEvent, String> {
    crate::app::lifecycle::require_running(&app)?;
    let session_record_id = payload.session_record_id.trim().to_string();
    if session_record_id.is_empty() {
        return Err("会话不存在".to_string());
    }
    let ai_channel_id = payload.ai_channel_id.trim().to_string();
    if ai_channel_id.is_empty() {
        return Err("必须选择 AI 渠道".to_string());
    }
    let model = payload.model.trim().to_string();
    if model.is_empty() {
        return Err("必须选择模型".to_string());
    }
    let request_id = payload.request_id.trim();
    let request_id = if request_id.is_empty() {
        new_id()
    } else {
        request_id.to_string()
    };
    let reasoning_effort = payload
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);
    let (reply, done) = tokio::sync::oneshot::channel();
    let config_tx = {
        let manager = state.lock().await;
        manager.require_running()?;
        let session = manager
            .get_session(&session_record_id)
            .ok_or_else(|| "会话未在运行".to_string())?;
        if session.closing {
            return Err("内置 Agent 正在结束，请稍后重试".to_string());
        }
        session.config_tx.clone()
    };
    config_tx
        .send(NativeConfigurationRequest {
            request_id,
            ai_channel_id,
            model,
            reasoning_effort,
            reply,
        })
        .await
        .map_err(|_| "会话已结束".to_string())?;
    done.await.map_err(|_| "会话已结束".to_string())?
}

pub(super) async fn finish_live_input(
    manager_state: &Mutex<NativeAgentManager>,
    session_record_id: &str,
) -> Result<(), String> {
    let completion = {
        let mut manager = manager_state.lock().await;
        let Some(tx) = manager.begin_finish(session_record_id)? else {
            return Ok(());
        };
        if let Err(error) = tx.try_send(NativeFollowup::Finish) {
            if !tx.is_closed() {
                if let Some(session) = manager.get_session_mut(session_record_id) {
                    session.closing = false;
                }
                return Err(format!("无法结束会话，输入队列已满: {error}"));
            }
        }
        manager
            .get_session(session_record_id)
            .map(|session| session.join.abort_handle())
    };
    if let Some(completion) = completion {
        tokio::time::timeout(Duration::from_secs(40), async {
            while !completion.is_finished() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| "会话仍在保存或释放资源，请稍后重试".to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn restart_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
    crate::app::lifecycle::require_running(&app)?;
    let restart_id = payload
        .resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);
    let _operation = if let Some(session_id) = restart_id {
        state
            .lock()
            .await
            .invalidate_plan_authorization(&session_id);
        let guard = lock_agent_session_operation(&state, &session_id).await;
        require_unarchived_session_with(&sqlite_pool(&app).await?, &session_id).await?;
        let _ = stop_native_process(
            &app,
            state.inner(),
            &session_id,
            "restart_requested",
            "收到重启请求",
        )
        .await?;
        Some(guard)
    } else {
        None
    };
    start_native_session_locked(app, state.inner().clone(), payload, None).await
}

#[tauri::command]
pub async fn resume_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: StartNativeSessionInput,
    resume_session_id: Option<String>,
) -> Result<AgentSessionStarted, String> {
    let mut payload = payload;
    if let Some(resume) = resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        payload.resume_session_id = Some(resume.to_string());
    }
    if payload
        .resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .is_none()
    {
        return Err("续聊必须提供 resume_session_id".to_string());
    }
    start_native_with_manager(app, state.inner().clone(), payload).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SessionSubagentInfo {
    pub id: String,
    pub index: u32,
    pub kind: String,
    pub description: String,
    pub status: String,
    pub start_time_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub error_message: Option<String>,
}

pub(super) fn extract_event_line(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(line) = val.get("line").and_then(|l| l.as_str()) {
                return line.to_string();
            }
        }
    }
    trimmed.to_string()
}

pub fn extract_session_subagents(
    events: &[(String, Option<String>, String)],
    is_session_active: bool,
) -> Vec<SessionSubagentInfo> {
    use std::collections::BTreeMap;

    struct SubagentEntry {
        id: String,
        index: u32,
        kind: String,
        description: String,
        status: String,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
        error_message: Option<String>,
    }

    let mut map: BTreeMap<String, SubagentEntry> = BTreeMap::new();

    for (_event_id, raw_message, created_at) in events {
        let Some(raw_msg) = raw_message else {
            continue;
        };
        let line = extract_event_line(raw_msg);
        let Some((tag, index, kind, desc)) =
            crate::native::agent::subagent::parse_subagent_log_tag(&line)
        else {
            continue;
        };

        let time_ms = chrono::NaiveDateTime::parse_from_str(
            created_at,
            crate::app::shared::SQLITE_DATETIME_FORMAT,
        )
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis());

        let entry = map.entry(tag.clone()).or_insert_with(|| SubagentEntry {
            id: tag.clone(),
            index,
            kind: kind.clone(),
            description: desc.clone(),
            status: if is_session_active {
                "running".to_string()
            } else {
                "stopped".to_string()
            },
            start_time_ms: time_ms,
            end_time_ms: None,
            error_message: None,
        });

        if line.contains("结束 失败") {
            entry.status = "failed".to_string();
            entry.end_time_ms = time_ms;
            if entry.error_message.is_none() {
                entry.error_message = Some(line.clone());
            }
        } else if line.contains("结束 成功") {
            entry.status = "completed".to_string();
            entry.end_time_ms = time_ms;
        } else if line.contains("结束 停止") || line.contains("结束 已停止") {
            entry.status = "stopped".to_string();
            entry.end_time_ms = time_ms;
        }
    }

    let mut result: Vec<SessionSubagentInfo> = map
        .into_values()
        .map(|entry| {
            let duration_ms = match (entry.start_time_ms, entry.end_time_ms) {
                (Some(start), Some(end)) => Some((end - start).max(0)),
                _ => None,
            };
            SessionSubagentInfo {
                id: entry.id,
                index: entry.index,
                kind: entry.kind,
                description: entry.description,
                status: entry.status,
                start_time_ms: entry.start_time_ms,
                duration_ms,
                error_message: entry.error_message,
            }
        })
        .collect();

    result.sort_by(|a, b| {
        a.index
            .cmp(&b.index)
            .then_with(|| a.start_time_ms.cmp(&b.start_time_ms))
    });

    result
}

#[tauri::command]
pub async fn get_session_subagents(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_id: String,
) -> Result<Vec<SessionSubagentInfo>, String> {
    let is_active = state.lock().await.get_session(&session_id).is_some();
    let pool = sqlite_pool(&app).await?;
    let rows = sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT id, message, created_at FROM agent_session_events WHERE session_id = $1 AND event_type = 'stdout' ORDER BY created_at ASC, rowid ASC",
    )
    .bind(&session_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| format!("查询子 Agent 状态失败: {e}"))?;

    Ok(extract_session_subagents(&rows, is_active))
}
