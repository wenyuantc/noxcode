#[allow(unused_imports)]
use super::*;

pub(super) async fn background_registry(
    state: &Mutex<NativeAgentManager>,
    session_record_id: &str,
) -> Result<Arc<crate::native::agent::background::BackgroundTaskRegistry>, String> {
    let manager = state.lock().await;
    let session = manager
        .get_session(session_record_id)
        .ok_or_else(|| "会话已结束".to_string())?;
    if session.closing {
        return Err("会话正在结束".to_string());
    }
    session
        .background
        .clone()
        .ok_or_else(|| "会话尚未完成初始化".to_string())
}

#[tauri::command]
pub async fn list_native_background_processes(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<Vec<crate::native::tools::processes::ProcessSnapshot>, String> {
    Ok(state
        .lock()
        .await
        .get_session(&session_record_id)
        .and_then(|session| session.processes.as_ref())
        .map(|registry| registry.snapshots())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn stop_native_background_process(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    process_id: String,
) -> Result<crate::native::tools::processes::ProcessSnapshot, String> {
    let manager = state.lock().await;
    let session = manager
        .get_session(&session_record_id)
        .ok_or_else(|| "会话已结束".to_string())?;
    if session.closing {
        return Err("会话正在结束".to_string());
    }
    session
        .processes
        .as_ref()
        .ok_or_else(|| "会话尚未完成初始化".to_string())?
        .stop(&process_id)
}

#[tauri::command]
pub async fn list_native_background_tasks(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<Vec<crate::native::agent::background::BackgroundTaskSnapshot>, String> {
    Ok(state
        .lock()
        .await
        .get_session(&session_record_id)
        .and_then(|session| session.background.as_ref())
        .map(|registry| registry.snapshots())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn send_native_background_message(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    task_id: String,
    message: String,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("消息不能为空".to_string());
    }
    background_registry(state.inner(), &session_record_id)
        .await?
        .send_message(&task_id, message.trim())
        .await
}

#[tauri::command]
pub async fn stop_native_background_task(
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    task_id: String,
) -> Result<bool, String> {
    background_registry(state.inner(), &session_record_id)
        .await?
        .stop(&task_id)
        .ok_or_else(|| "后台任务不存在".to_string())
}
