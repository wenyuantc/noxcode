#[allow(unused_imports)]
use super::*;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeBoundaryInput {
    pub session_record_id: String,
    pub message_id: String,
    pub edge: Option<String>,
    pub expected_revision: i64,
    pub request_id: String,
}

/// 从指定消息分叉或回退。`fork` 新建会话；`rewind` 保留当前会话并封存原分支。
#[tauri::command]
pub async fn apply_native_history_boundary(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    action: String,
    input: NativeBoundaryInput,
) -> Result<String, String> {
    let session_record_id = input.session_record_id.trim().to_string();
    let request_id = input.request_id.trim().to_string();
    if session_record_id.is_empty() || input.message_id.trim().is_empty() || request_id.is_empty() {
        return Err("会话、消息和请求标识不能为空".to_string());
    }
    let edge = crate::native::history::BoundaryEdge::parse(input.edge.as_deref())?;
    let rewind = match action.trim() {
        "fork" => false,
        "rewind" => true,
        _ => return Err("边界操作只能是 fork 或 rewind".to_string()),
    };
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    ensure_idle_for_boundary(&state, &session_record_id).await?;
    let pool = sqlite_pool(&app).await?;
    require_unarchived_session_with(&pool, &session_record_id).await?;
    if let Some(existing) = boundary_request_session(&pool, &request_id).await? {
        return Ok(existing);
    }
    let source = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_record_id}"))?;
    let source_branch = crate::native::history::active_branch_id(&pool, &session_record_id)
        .await?
        .ok_or_else(|| "该会话没有可操作的历史".to_string())?;
    let model = sqlx::query_scalar::<_, String>(
        "SELECT model FROM native_session_transcripts WHERE session_record_id = $1",
    )
    .bind(&session_record_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| format!("读取会话模型失败: {error}"))?
    .unwrap_or_default();
    let target_id = if rewind {
        session_record_id.clone()
    } else {
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
        new_id
    };
    let receipt = match crate::native::history::create_referencing_branch(
        &pool,
        crate::native::history::BranchReference {
            new_session_id: &target_id,
            source_branch_id: &source_branch,
            boundary_message_id: Some(input.message_id.trim()),
            profile_id: None,
            workspace_id: source.workspace_id.as_deref(),
            model: &model,
            turns: 0,
            edge,
            expected_revision: Some(input.expected_revision),
            request_id: Some(&request_id),
            seal_source: rewind,
        },
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            if !rewind {
                let _ = sqlx::query("DELETE FROM agent_sessions WHERE id = $1")
                    .bind(&target_id)
                    .execute(&pool)
                    .await;
            }
            return Err(error);
        }
    };
    if receipt.changed {
        record_branch_display(&pool, &target_id, &receipt.branch_id).await?;
    } else if !rewind {
        let _ = sqlx::query("DELETE FROM agent_sessions WHERE id = $1")
            .bind(&target_id)
            .execute(&pool)
            .await;
    }
    if rewind {
        state
            .lock()
            .await
            .invalidate_plan_authorization(&session_record_id);
    }
    Ok(target_id)
}

pub(super) async fn history_revision(
    pool: &sqlx::SqlitePool,
    branch_id: &str,
) -> Result<i64, String> {
    sqlx::query_scalar("SELECT revision FROM native_history_branches WHERE id = $1")
        .bind(branch_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取分支修订号失败: {error}"))?
        .ok_or_else(|| "来源分支不存在".to_string())
}

pub(super) async fn boundary_request_session(
    pool: &sqlx::SqlitePool,
    request_id: &str,
) -> Result<Option<String>, String> {
    let session_id = sqlx::query_scalar::<_, String>(
        r#"
        SELECT b.session_record_id
        FROM native_history_requests r
        JOIN native_history_branches b ON b.id = r.branch_id
        WHERE r.request_id = $1 AND b.deleted_at IS NULL
        "#,
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取边界请求失败: {error}"))?;
    Ok(session_id)
}

pub(super) async fn record_branch_display(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    branch_id: &str,
) -> Result<(), String> {
    let _ = insert_session_event(
        pool,
        session_record_id,
        "stdout",
        Some(&format!("[分支] {branch_id} 已按消息边界重建对话")),
    )
    .await;
    let Some(messages) = load_transcript(pool, session_record_id).await? else {
        return Ok(());
    };
    for message in messages {
        let line = match message.role {
            crate::native::model::types::Role::User => {
                Some(format!("[USER_INPUT] {}", message.content))
            }
            crate::native::model::types::Role::Assistant if !message.content.is_empty() => {
                Some(message.content)
            }
            crate::native::model::types::Role::Tool => {
                Some(format!("[工具结果] {}\n{}", message.name, message.content))
            }
            _ => None,
        };
        if let Some(line) = line {
            let _ = insert_session_event(pool, session_record_id, "stdout", Some(&line)).await;
        }
    }
    if let Some(goal) = crate::native::goals::current_goal(pool, session_record_id).await? {
        let _ = insert_session_event(pool, session_record_id, "stdout", Some(&goal.line())).await;
    }
    Ok(())
}

pub(super) async fn ensure_idle_for_boundary(
    manager_state: &Mutex<NativeAgentManager>,
    session_record_id: &str,
) -> Result<(), String> {
    let busy = {
        let manager = manager_state.lock().await;
        let Some(session) = manager.get_session(session_record_id) else {
            return Ok(());
        };
        session.input_queue.is_busy(&session.working)
            || session.followup_tx.capacity() < session.followup_tx.max_capacity()
    };
    if busy {
        return Err("会话正在工作，请等待结束或停止后再修改对话边界".to_string());
    }
    finish_live_input(manager_state, session_record_id).await
}
