#[allow(unused_imports)]
use super::*;

#[derive(Clone)]
pub(super) struct SteerSubmission {
    pub(super) session_record_id: String,
    pub(super) expected_turn_id: String,
    pub(super) input_id: String,
    pub(super) text: String,
    pub(super) image_paths: Vec<String>,
}

pub(super) fn load_steer_submission_images(
    root: &std::path::Path,
    paths: &[String],
) -> Result<crate::native::images::NativeImageLoad, String> {
    let canonical_root = if paths.is_empty() {
        root.to_path_buf()
    } else {
        root.canonicalize()
            .map_err(|e| format!("图片暂存目录不可用：{e}"))?
    };
    for path in paths {
        let path = std::path::Path::new(path);
        if !path.is_absolute()
            || !path
                .canonicalize()
                .map_err(|e| format!("图片不可用：{e}"))?
                .starts_with(&canonical_root)
        {
            return Err("请选择或粘贴图片到输入框后再提交".into());
        }
    }
    crate::native::images::load_steer_images(paths)
}

#[tauri::command]
pub async fn submit_native_steer(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    expected_turn_id: String,
    input_id: String,
    text: String,
    image_paths: Vec<String>,
) -> Result<crate::native::steer::SteerReceipt, crate::native::steer::SteerSubmissionError> {
    crate::app::lifecycle::require_running(&app)?;
    let pool = sqlite_pool(&app).await?;
    let root = crate::native::images::attachments_dir(
        &app.path()
            .app_config_dir()
            .map_err(|error| error.to_string())?,
    );
    let input = SteerSubmission {
        session_record_id,
        expected_turn_id,
        input_id,
        text,
        image_paths,
    };
    submit_native_steer_with(
        &pool,
        state.inner(),
        &input,
        |paths| async move {
            tokio::task::spawn_blocking(move || load_steer_submission_images(&root, &paths))
                .await
                .map_err(|error| error.to_string())?
        },
        |manager, resolved| {
            for (request_id, kind) in resolved {
                emit_request_resolved(&app, &input.session_record_id, &request_id, kind);
            }
            if let Some(pending) = manager
                .get_session(&input.session_record_id)
                .and_then(|session| session.pending_permission.front())
            {
                let _ = app.emit(
                    "native-permission-request",
                    permission_event(&input.session_record_id, &pending.request),
                );
            }
        },
    )
    .await
}

/// The IPC's complete admission path. Loader/publisher dependencies keep the same
/// persistence, identity and locking path testable with real SQLite and staged files.
pub(super) async fn submit_native_steer_with<F, Fut, P>(
    pool: &sqlx::SqlitePool,
    state: &Arc<Mutex<NativeAgentManager>>,
    input: &SteerSubmission,
    load_images: F,
    publish_resolved: P,
) -> Result<crate::native::steer::SteerReceipt, crate::native::steer::SteerSubmissionError>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<crate::native::images::NativeImageLoad, String>>,
    P: FnOnce(&NativeAgentManager, Vec<(String, &'static str)>),
{
    use crate::native::steer::{SteerMailbox, SteerSubmissionError};
    let SteerSubmission {
        session_record_id,
        expected_turn_id,
        input_id,
        text,
        image_paths,
    } = input;
    SteerMailbox::validate_payload(input_id, text, image_paths)
        .map_err(SteerSubmissionError::rejected)?;
    let submission_lock = state
        .lock()
        .await
        .steer_submission_lock(session_record_id, input_id);
    let _submission = submission_lock.lock_owned().await;
    // The UUID lock covers lookup, image loading, acceptance and staged cleanup.
    // A concurrent retry can only observe the completed result, even if its turn ended.
    let manager = state.lock().await;
    let live_instance = manager
        .get_session(session_record_id)
        .map(|session| session.input_queue.id.as_str());
    if let Some(receipt) = find_steer_receipt(
        pool,
        session_record_id,
        input_id,
        expected_turn_id,
        text,
        image_paths,
        live_instance,
    )
    .await?
    {
        return Ok(receipt);
    }
    let session = manager
        .get_session(session_record_id)
        .ok_or("会话已结束，请保留草稿")?;
    if !session.accepts_origin(None) {
        return Err("会话已停止".into());
    }
    let mailbox = session.input_queue.steer.clone();
    drop(manager);
    if let Some(receipt) = mailbox
        .prior(expected_turn_id, input_id, text, image_paths)
        .await?
    {
        return Ok(receipt);
    }
    let loaded = load_images(image_paths.clone())
        .await
        .map_err(SteerSubmissionError::rejected)?;
    {
        let mut manager = state.lock().await;
        let session = manager.get_session(session_record_id).ok_or("会话已结束")?;
        if !session.accepts_origin(None) || session.input_queue.id != mailbox.instance_id {
            return Err("当前回合已结束或已切换".into());
        }
        // Refuse stale/full/oversized admission before cancelling a saving authorization.
        if let Some(receipt) = mailbox
            .check_admission(
                expected_turn_id,
                input_id,
                text,
                image_paths,
                &loaded.images,
            )
            .await
            .map_err(SteerSubmissionError::rejected)?
        {
            return Ok(receipt);
        }
        manager.invalidate_plan_authorization(session_record_id);
    }
    let _operation = lock_agent_session_operation(state, session_record_id).await;
    let mut manager = state.lock().await;
    let session = manager.get_session(session_record_id).ok_or("会话已结束")?;
    if !session.accepts_origin(None) || session.input_queue.id != mailbox.instance_id {
        return Err("会话已停止或重启".into());
    }
    // Admission revalidates under the final-seal gate. Its persistence callback
    // commits plan invalidation and acceptance together, or rolls both back.
    let receipt = mailbox
        .accept(
            expected_turn_id,
            input_id,
            text,
            image_paths,
            loaded.images.clone(),
        )
        .await
        .map_err(SteerSubmissionError::rejected)?;
    manager.invalidate_plan_authorization(session_record_id);
    let resolved = manager.supersede_main_requests(session_record_id);
    publish_resolved(&manager, resolved);
    drop(manager);
    crate::native::images::cleanup_staged_loaded_images(&loaded);
    Ok(receipt)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn find_steer_receipt(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    input_id: &str,
    turn_id: &str,
    text: &str,
    paths: &[String],
    live_instance: Option<&str>,
) -> Result<Option<crate::native::steer::SteerReceipt>, String> {
    use crate::native::steer::{SteerMailbox, SteerReceipt, SteerStatus};
    let row = sqlx::query_scalar::<_, String>("SELECT message FROM agent_session_events WHERE session_id = $1 AND event_type = 'native_steer' AND json_extract(message, '$.input_id') = $2 ORDER BY rowid DESC LIMIT 1")
        .bind(session_id).bind(input_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut receipt: SteerReceipt = serde_json::from_str(&row).map_err(|e| e.to_string())?;
    if receipt.turn_id != turn_id || receipt.payload_hash != SteerMailbox::payload_hash(text, paths)
    {
        return Err("相同输入标识不能用于不同内容".into());
    }
    if receipt.status == SteerStatus::Accepted
        && live_instance != Some(receipt.instance_id.as_str())
    {
        receipt.status = SteerStatus::Cancelled;
        receipt.error =
            Some("会话已中断；未自动重放此指令。可恢复文字草稿，图片需要重新选择。".into());
        let message = serde_json::to_string(&receipt).map_err(|e| e.to_string())?;
        insert_session_event(pool, session_id, "native_steer", Some(&message)).await?;
    }
    Ok(Some(receipt))
}

pub(super) async fn load_steer_receipts(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    live_instance: Option<&str>,
) -> Result<Vec<crate::native::steer::SteerReceipt>, String> {
    use crate::native::steer::{SteerReceipt, SteerStatus};
    let rows = sqlx::query_scalar::<_, String>("SELECT message FROM agent_session_events WHERE session_id = $1 AND event_type = 'native_steer' ORDER BY rowid DESC LIMIT 1024")
        .bind(session_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let mut receipts = Vec::new();
    for row in rows {
        let Ok(mut receipt) = serde_json::from_str::<SteerReceipt>(&row) else {
            continue;
        };
        if !seen.insert(receipt.input_id.clone()) {
            continue;
        }
        if receipt.status == SteerStatus::Accepted
            && live_instance != Some(receipt.instance_id.as_str())
        {
            receipt.status = SteerStatus::Cancelled;
            receipt.error =
                Some("会话已中断；未自动重放此指令。可恢复文字草稿，图片需要重新选择。".into());
            let message = serde_json::to_string(&receipt).map_err(|e| e.to_string())?;
            insert_session_event(pool, session_id, "native_steer", Some(&message)).await?;
        }
        receipts.push(receipt);
        if receipts.len() >= 256 {
            break;
        }
    }
    receipts.reverse();
    Ok(receipts)
}

#[tauri::command]
pub async fn get_native_steer_snapshot(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<crate::native::steer::SteerSnapshot, String> {
    let manager = state.lock().await;
    let mailbox = manager
        .get_session(&session_record_id)
        .map(|s| s.input_queue.steer.clone());
    // Keep instance selection and recovery consistent with a concurrent restart.
    let pool = sqlite_pool(&app).await?;
    let receipts = load_steer_receipts(
        &pool,
        &session_record_id,
        mailbox.as_ref().map(|m| m.instance_id.as_str()),
    )
    .await?;
    if let Some(mailbox) = mailbox {
        let mut snapshot = mailbox.snapshot().await;
        let current: std::collections::HashSet<_> = snapshot
            .receipts
            .iter()
            .map(|r| r.input_id.clone())
            .collect();
        let mut history: Vec<_> = receipts
            .into_iter()
            .filter(|r| !current.contains(&r.input_id))
            .collect();
        history.append(&mut snapshot.receipts);
        snapshot.receipts = history;
        return Ok(snapshot);
    }
    Ok(crate::native::steer::SteerSnapshot {
        lifecycle: None,
        session_record_id,
        instance_id: receipts
            .last()
            .map(|r| r.instance_id.clone())
            .unwrap_or_default(),
        turn_id: None,
        revision: 0,
        receipts,
    })
}

#[tauri::command]
pub async fn send_native_input(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    input: String,
    image_paths: Option<Vec<String>>,
) -> Result<NativeInputQueueSnapshot, String> {
    crate::app::lifecycle::require_running(&app)?;
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    let pool = sqlite_pool(&app).await?;
    require_unarchived_session_with(&pool, &session_record_id).await?;
    let attachment_root = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    enqueue_live_input(
        state.inner().as_ref(),
        &session_record_id,
        &input,
        image_paths.as_deref(),
        Some(&pool),
        Some(&attachment_root),
    )
    .await?
    .map(|(_, snapshot)| snapshot)
    .ok_or_else(|| format!("会话 {session_record_id} 当前没有运行中的内置 Agent"))
}

#[tauri::command]
pub async fn list_native_queued_inputs(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<NativeInputQueueSnapshot, String> {
    let manager = state.lock().await;
    let session = manager
        .get_session(&session_record_id)
        .ok_or_else(|| "会话已结束".to_string())?;
    Ok(session.input_queue.snapshot())
}

#[tauri::command]
pub async fn update_native_queued_input(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    input_id: String,
    input: Option<String>,
    editing: bool,
) -> Result<NativeInputQueueSnapshot, String> {
    let manager = state.lock().await;
    let session = manager
        .get_session(&session_record_id)
        .ok_or_else(|| "会话已结束".to_string())?;
    if session.closing {
        return Err("会话正在结束".to_string());
    }
    session
        .input_queue
        .update(&input_id, input.as_deref(), editing)
}

#[tauri::command]
pub async fn remove_native_queued_input(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    input_id: String,
) -> Result<NativeInputQueueSnapshot, String> {
    let manager = state.lock().await;
    let session = manager
        .get_session(&session_record_id)
        .ok_or_else(|| "会话已结束".to_string())?;
    session.input_queue.remove(&input_id)
}

pub(super) fn emit_request_resolved(
    app: &AppHandle,
    session_record_id: &str,
    request_id: &str,
    kind: &str,
) {
    let _ = app.emit(
        "native-request-resolved",
        serde_json::json!({
            "session_record_id": session_record_id, "request_id": request_id, "kind": kind,
        }),
    );
}
