#[allow(unused_imports)]
use super::*;

#[tauri::command]
pub async fn resolve_native_tool_permission(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    request_id: String,
    decision: NativePermissionDecision,
    file_access: Option<Vec<crate::native::tools::file_access::FileAccessSelection>>,
    scope: Option<crate::native::tools::permission::RuleScope>,
) -> Result<(), String> {
    let mut manager = state.lock().await;
    // Keep the request queued until all selected rules have been saved atomically.
    if decision == NativePermissionDecision::AllowAlways {
        let config_dir = app
            .path()
            .app_config_dir()
            .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
        manager.save_permission_rules(
            &config_dir,
            &session_record_id,
            &request_id,
            file_access.as_deref(),
            scope,
        )?;
    }
    let (resolved, next) = if decision == NativePermissionDecision::AllowSessionCommands {
        manager.resolve_session_commands(&session_record_id, &request_id)?
    } else {
        (
            vec![request_id.clone()],
            manager.resolve_permission(&session_record_id, &request_id, decision)?,
        )
    };
    for request_id in resolved {
        emit_request_resolved(&app, &session_record_id, &request_id, "permission");
    }
    if let Some(request) = next {
        let _ = app.emit(
            "native-permission-request",
            permission_event(&session_record_id, &request),
        );
    }
    Ok(())
}

/// `/fork`：从最新已提交边界复制对话到一条新的已结束会话。文件回滚不在这里执行。
#[tauri::command]
pub async fn fork_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    checkpoint_id: Option<String>,
) -> Result<String, String> {
    if checkpoint_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|item| !item.is_empty())
    {
        return Err("文件回滚需要预览后再应用，/fork 只分叉当前对话".to_string());
    }
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    ensure_idle_for_boundary(&state, &session_record_id).await?;
    let pool = sqlite_pool(&app).await?;
    let source = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_record_id}"))?;
    let messages = load_transcript(&pool, &session_record_id)
        .await?
        .ok_or_else(|| "该会话没有可分叉的上下文".to_string())?;
    let new_id = new_id();
    let now = now_sqlite();
    let title = source
        .title
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| format!("{item}（分叉）"))
        .unwrap_or_else(|| "分叉会话".to_string());
    sqlx::query(
        r#"
        INSERT INTO agent_sessions (
            id, ai_channel_id, workspace_id, working_dir, execution_target,
            ssh_config_id, target_host_label, session_kind, status,
            started_at, ended_at, exit_code, resume_session_id, created_at, title
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'exited', $9, $9, 0, $10, $9, $11)
        "#,
    )
    .bind(&new_id)
    .bind(&source.ai_channel_id)
    .bind(&source.workspace_id)
    .bind(&source.working_dir)
    .bind(&source.execution_target)
    .bind(&source.ssh_config_id)
    .bind(&source.target_host_label)
    .bind(&source.session_kind)
    .bind(&now)
    .bind(&session_record_id)
    .bind(&title)
    .execute(&pool)
    .await
    .map_err(|error| format!("创建分叉会话失败: {error}"))?;
    let model = sqlx::query_scalar::<_, String>(
        "SELECT model FROM native_session_transcripts WHERE session_record_id = $1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话模型失败: {error}"))?
    .unwrap_or_default();
    let turns = messages
        .iter()
        .filter(|message| message.role == crate::native::model::types::Role::User)
        .count() as u32;
    let source_branch = crate::native::history::active_branch_id(&pool, &session_record_id)
        .await?
        .ok_or_else(|| "该会话没有可分叉的历史".to_string())?;
    let revision = history_revision(&pool, &source_branch).await?;
    let receipt = crate::native::history::create_referencing_branch(
        &pool,
        crate::native::history::BranchReference {
            new_session_id: &new_id,
            source_branch_id: &source_branch,
            boundary_message_id: None,
            profile_id: None,
            workspace_id: source.workspace_id.as_deref(),
            model: &model,
            turns,
            edge: crate::native::history::BoundaryEdge::After,
            expected_revision: Some(revision),
            request_id: None,
            seal_source: false,
        },
    )
    .await?;
    record_branch_display(&pool, &new_id, &receipt.branch_id).await?;
    let _ = insert_session_event(
        &pool,
        &new_id,
        "stdout",
        Some(&format!(
            "[续聊] 从会话 {session_record_id} 分叉，共引用 {} 条消息",
            messages.len()
        )),
    )
    .await;
    Ok(new_id)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeFileRollbackPreviewInput {
    pub session_record_id: String,
    pub message_id: String,
    pub edge: Option<String>,
    pub mode: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeFileRollbackInput {
    pub session_record_id: String,
    pub message_id: String,
    pub edge: Option<String>,
    pub mode: String,
    pub expected_revision: i64,
    pub token: String,
    pub request_id: String,
}

pub(super) async fn live_files_for_session(
    app: &AppHandle,
    manager_state: &Mutex<NativeAgentManager>,
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
) -> Result<crate::native::file_rollback::LiveFiles, String> {
    let record = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(session_record_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_record_id}"))?;
    let stored = record
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);
    let live_root = manager_state
        .lock()
        .await
        .effective_plan_cwd(session_record_id);
    let root = live_root
        .or(stored)
        .ok_or_else(|| "会话没有绑定工作目录，不能回滚文件".to_string())?;
    if record.execution_target == crate::app::shared::EXECUTION_TARGET_SSH {
        let config_id = record
            .ssh_config_id
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .ok_or_else(|| "SSH 会话缺少配置".to_string())?;
        let config = fetch_ssh_config_record_by_id(pool, config_id).await?;
        return Ok(crate::native::file_rollback::LiveFiles::ssh(
            SshToolRuntime {
                app: app.clone(),
                config,
                root,
                authorized_paths: Vec::new(),
            },
        ));
    }
    if record.execution_target != crate::app::shared::EXECUTION_TARGET_LOCAL {
        return Err(format!("未知的执行目标 {}", record.execution_target));
    }
    Ok(crate::native::file_rollback::LiveFiles::disk(
        crate::app::shared::EXECUTION_TARGET_LOCAL,
        root,
    ))
}

pub(super) fn files_rollback_enabled(app: &AppHandle) -> bool {
    crate::native::settings::load_native_settings(app)
        .map(|settings| settings.auto_checkpoint_after_tool_call)
        .unwrap_or(true)
}

/// 按消息边界预览可回滚的文件。不写文件，也不改对话分支。
#[tauri::command]
pub async fn preview_native_file_rollback(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    input: NativeFileRollbackPreviewInput,
) -> Result<crate::native::file_rollback::FileRollbackPreview, String> {
    let session_record_id = input.session_record_id.trim().to_string();
    let message_id = input.message_id.trim().to_string();
    if session_record_id.is_empty() || message_id.is_empty() {
        return Err("会话和消息不能为空".to_string());
    }
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    ensure_idle_for_boundary(&state, &session_record_id).await?;
    let pool = sqlite_pool(&app).await?;
    require_unarchived_session_with(&pool, &session_record_id).await?;
    let files = live_files_for_session(&app, &state, &pool, &session_record_id).await?;
    let edge = crate::native::history::BoundaryEdge::parse(input.edge.as_deref())?;
    crate::native::file_rollback::preview(
        &pool,
        &session_record_id,
        &message_id,
        edge,
        &input.mode,
        files_rollback_enabled(&app),
        &files,
    )
    .await
}

/// 用预览凭据应用文件回滚。对话切换只在文件成功之后发生。
#[tauri::command]
pub async fn apply_native_file_rollback(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    input: NativeFileRollbackInput,
) -> Result<String, String> {
    let session_record_id = input.session_record_id.trim().to_string();
    let message_id = input.message_id.trim().to_string();
    let request_id = input.request_id.trim().to_string();
    if session_record_id.is_empty() || message_id.is_empty() || request_id.is_empty() {
        return Err("会话、消息和请求标识不能为空".to_string());
    }
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    ensure_idle_for_boundary(&state, &session_record_id).await?;
    let pool = sqlite_pool(&app).await?;
    require_unarchived_session_with(&pool, &session_record_id).await?;
    let files = live_files_for_session(&app, &state, &pool, &session_record_id).await?;
    let edge = crate::native::history::BoundaryEdge::parse(input.edge.as_deref())?;
    let source = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_record_id}"))?;
    let model = sqlx::query_scalar::<_, String>(
        "SELECT model FROM native_session_transcripts WHERE session_record_id = $1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话模型失败: {error}"))?
    .unwrap_or_default();
    let receipt = crate::native::file_rollback::commit_rollback(
        &pool,
        crate::native::file_rollback::RollbackCommit {
            session_record_id: &session_record_id,
            message_id: &message_id,
            edge,
            mode: &input.mode,
            expected_revision: input.expected_revision,
            token: &input.token,
            request_id: &request_id,
            files_enabled: files_rollback_enabled(&app),
            workspace_id: source.workspace_id.as_deref(),
            model: &model,
        },
        &files,
    )
    .await?;
    if receipt.changed_conversation {
        if let Some(branch_id) = receipt.branch_id.as_deref() {
            record_branch_display(&pool, &session_record_id, branch_id).await?;
        }
        state
            .lock()
            .await
            .invalidate_plan_authorization(&session_record_id);
    }
    Ok(session_record_id)
}
