#[allow(unused_imports)]
use super::*;

/// 手动触发记忆整理（dream）：用指定渠道（优先其轻量模型）合并、去重工作区记忆。
#[tauri::command]
pub async fn dream_native_memory(
    app: AppHandle,
    workspace_id: String,
    channel_id: String,
    model: Option<String>,
) -> Result<String, String> {
    let pool = sqlite_pool(&app).await?;
    let run = load_native_client(
        &app,
        &pool,
        &channel_id,
        model.as_deref().unwrap_or(""),
        None,
    )
    .await?;
    let context = resolve_workspace_execution_context_with_pool(&pool, &workspace_id).await?;
    if context.execution_target != crate::app::shared::EXECUTION_TARGET_LOCAL {
        return Err("记忆只对本地工作区可用".to_string());
    }
    let root = context
        .working_dir
        .ok_or_else(|| "工作区缺少目录".to_string())?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
    let dir = crate::native::memory::memory_dir(&config_dir, &root);
    let client = run
        .client
        .with_call_log_context(CallLogContext::for_session(
            Some(run.channel_id.clone()),
            Some(run.channel_name.clone()),
            None,
            None,
            Some(workspace_id.clone()),
            CALL_KIND_ONE_SHOT,
            Some(context.execution_target.clone()),
        ));
    crate::native::memory::dream(&client, &run.model, run.lite_model.as_deref(), &dir).await
}

/// `/compact [指令]`：向运行中的会话投递压缩请求；等待输入时立即执行，工作中则在下一次模型调用前执行。
#[tauri::command]
pub async fn compact_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    instructions: Option<String>,
) -> Result<bool, String> {
    crate::app::lifecycle::require_running(&app)?;
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    require_unarchived_session_with(&sqlite_pool(&app).await?, &session_record_id).await?;
    let instructions = instructions
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let manager = state.lock().await;
    let Some(session) = manager.get_session(&session_record_id) else {
        return Ok(false);
    };
    if session.closing {
        return Err("会话正在结束，无法压缩".to_string());
    }
    session
        .followup_tx
        .try_send(NativeFollowup::Compact(NativeCompactionRequest::new(
            instructions,
            session.pending_compactions.clone(),
        )))
        .map_err(|error| format!("无法压缩，输入队列已满或会话已结束: {error}"))?;
    session.working.store(true, Ordering::SeqCst);
    Ok(true)
}

pub(super) fn resolve_plan_implementation_effort(
    current: Option<&str>,
    requested: Option<&str>,
) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| current.map(ToOwned::to_owned))
}

pub(super) fn plan_implementation_unchanged(
    current_channel: &str,
    current_model: &str,
    current_effort: Option<&str>,
    next_channel: &str,
    next_model: &str,
    next_effort: Option<&str>,
) -> bool {
    current_channel == next_channel && current_model == next_model && current_effort == next_effort
}

pub(super) async fn apply_plan_implementation_model(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    session_record_id: &str,
    mut next: NativeRunSettings,
    request_id: &str,
    authorization: &PlanAuthorization,
) -> Result<(), String> {
    let snapshot = {
        let manager = manager_state.lock().await;
        let session = manager
            .get_session(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let runtime = session.runtime_snapshot();
        let current_channel = runtime
            .as_ref()
            .map(|item| item.ai_channel_id.as_str())
            .unwrap_or("");
        let current_model = runtime
            .as_ref()
            .map(|item| item.model.as_str())
            .unwrap_or("");
        let current_effort = runtime
            .as_ref()
            .and_then(|item| item.reasoning_effort.clone());
        if plan_implementation_unchanged(
            current_channel,
            current_model,
            current_effort.as_deref(),
            &next.channel_id,
            &next.model,
            next.effort.as_deref(),
        ) {
            return Ok(());
        }
        let workspace_id = session.info.workspace_id.clone().unwrap_or_default();
        let profile_id = session.info.profile_id.clone();
        let session_kind = session.info.session_kind.clone();
        let input_queue_id = session.input_queue.id.clone();
        let execution_target = session
            .live_model
            .as_ref()
            .and_then(|slot| {
                slot.lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .execution_target
                    .clone()
            })
            .or_else(|| Some(crate::app::shared::EXECUTION_TARGET_LOCAL.to_string()));
        let recovery = session.live_model.as_ref().and_then(|slot| {
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .client
                .recovery_state()
        });
        (
            workspace_id,
            profile_id,
            session_kind,
            input_queue_id,
            execution_target,
            recovery,
        )
    };
    let (workspace_id, profile_id, session_kind, input_queue_id, execution_target, recovery) =
        snapshot;
    if let Some(recovery) = recovery {
        next.client = next.client.with_recovery(recovery);
    }
    let pool = sqlite_pool(app).await?;
    let next = bind_run_to_session(
        next,
        session_record_id,
        &workspace_id,
        false,
        execution_target.clone(),
    );
    authorization.check()?;
    let context_token_limit =
        crate::native::settings::session_context_window_tokens(app, next.context_tokens) as usize;
    let hook_agent = hook_agent_handler(&next);
    let runtime = {
        let mut manager = manager_state.lock().await;
        manager.require_plan_approval(session_record_id, request_id)?;
        authorization.with_current(|| -> Result<_, String> {
            let session = manager
                .get_session_mut(session_record_id)
                .ok_or_else(|| "会话已结束".to_string())?;
            if let Some(slot) = &session.live_model {
                write_live_model(
                    slot,
                    live_snapshot_from_run(
                        &next,
                        context_token_limit,
                        execution_target,
                        Some(hook_agent),
                    ),
                );
            }
            session.info.channel_id = next.channel_id.clone();
            let permission_mode = session
                .runtime
                .as_ref()
                .map(|item| item.permission_mode.clone())
                .unwrap_or_else(|| crate::native::settings::PERMISSION_MODE_DEFAULT.to_string());
            let plan_mode = session.plan_mode.load(Ordering::SeqCst);
            let previous = session.runtime.clone();
            let runtime = NativeSessionRuntime {
                ai_channel_id: next.channel_id.clone(),
                model: next.model.clone(),
                reasoning_effort: next.effort.clone(),
                permission_mode,
                plan_mode,
                worktree_path: previous
                    .as_ref()
                    .and_then(|item| item.worktree_path.clone()),
                sandbox_active: previous.as_ref().is_some_and(|item| item.sandbox_active),
            };
            session.runtime = Some(runtime.clone());
            let transcript = session
                .transcript_model
                .clone()
                .unwrap_or_else(|| Arc::new(Mutex::new(next.model.clone())));
            Ok((runtime, transcript))
        })??
    };
    update_agent_session_channel(&pool, session_record_id, &next.channel_id).await?;
    authorization.check()?;
    let (runtime, transcript_model) = runtime;
    {
        let mut transcript = transcript_model.lock().await;
        authorization.with_current(|| *transcript = next.model.clone())?;
    }
    let started = AgentSessionStarted {
        runtime: Some(runtime),
        input_queue_id: Some(input_queue_id),
        profile_id: profile_id.clone(),
        workspace_id: workspace_id.clone(),
        session_kind: session_kind.clone(),
        session_record_id: session_record_id.to_string(),
    };
    let _ = app.emit("native-session", &started);
    emit_native_line(
        app,
        session_record_id,
        &profile_id,
        Some(&workspace_id),
        &session_kind,
        format!(
            "[内置 Agent] 实施改用 渠道={} 协议={} model={}{}",
            next.channel_name,
            next.protocol,
            next.model,
            next.effort
                .as_deref()
                .map(|effort| format!(" 思考={effort}"))
                .unwrap_or_default()
        ),
    )
    .await;
    Ok(())
}

/// Invalidate immediately, then serialize durable cleanup with approval/save/start.
/// Stop retains pending approval; steer passes `clear_pending = true`.
/// Do not call this while holding the session operation lock. Call the manager's
/// synchronous invalidation first and `plans::invalidate_persisted` under your lock instead.
pub(crate) async fn invalidate_native_plan_authorization(
    app: &AppHandle,
    manager: &Arc<Mutex<NativeAgentManager>>,
    session: &str,
    clear_pending: bool,
) -> Result<(), String> {
    manager.lock().await.invalidate_plan_authorization(session);
    let _operation = lock_agent_session_operation(manager, session).await;
    plans::invalidate_persisted(&sqlite_pool(app).await?, session, clear_pending).await
}

pub(super) async fn clear_pending_plan_request(
    pool: &sqlx::SqlitePool,
    session: &str,
    request: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE agent_sessions SET pending_plan_json = NULL WHERE id = $1 AND json_extract(pending_plan_json, '$.request_id') = $2")
        .bind(session).bind(request).execute(pool).await.map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn resolve_native_plan_approval(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    request_id: String,
    approved: bool,
    feedback: Option<String>,
    ai_channel_id: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
) -> Result<Option<AgentSessionStarted>, String> {
    crate::app::lifecycle::require_running(&app)?;
    // Capture stop/steer invalidation without mutating any existing authorization.
    let authorization_revision = state
        .lock()
        .await
        .plan_authorization_revision(&session_record_id);
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    let pool = sqlite_pool(&app).await?;
    require_unarchived_session_with(&pool, &session_record_id).await?;
    let record =
        sqlx::query_as::<_, AgentSessionRecord>("SELECT * FROM agent_sessions WHERE id = $1")
            .bind(&session_record_id)
            .fetch_one(&pool)
            .await
            .map_err(|error| error.to_string())?;
    let pending = plans::load_pending(&pool, &session_record_id).await?;
    if pending.request_id != request_id {
        return Err("计划批准请求已过期".into());
    }
    let (live, runtime, live_cwd) = {
        let manager = state.lock().await;
        let live = manager.get_session(&session_record_id);
        if live.is_some() {
            manager.require_plan_approval(&session_record_id, &request_id)?;
        }
        (
            live.is_some(),
            live.and_then(|session| session.runtime_snapshot()),
            manager.effective_plan_cwd(&session_record_id),
        )
    };
    // The operation lock and authoritative pending identity must precede replacement.
    // A stale request cannot cancel a save or an approved reply awaiting consumption.
    let authorization = state.lock().await.begin_plan_authorization(
        &session_record_id,
        &request_id,
        authorization_revision,
    )?;
    let workspace_id = record
        .workspace_id
        .clone()
        .ok_or_else(|| "会话缺少工作区".to_string())?;
    let old = plans::load_approved(&pool, &session_record_id).await?;
    if !approved && live {
        authorization.check()?;
        plans::invalidate_persisted(&pool, &session_record_id, false).await?;
        let next = state.lock().await.resolve_plan_approval(
            &session_record_id,
            &request_id,
            PlanApprovalAnswer {
                approved: false,
                feedback: feedback.unwrap_or_default(),
                ..Default::default()
            },
        )?;
        clear_pending_plan_request(&pool, &session_record_id, &request_id).await?;
        emit_request_resolved(&app, &session_record_id, &request_id, "plan_approval");
        if let Some(next) = next {
            save_pending_plan_with(&pool, &session_record_id, &next.request_id, &next.plan).await?;
            let manager = state.lock().await;
            if manager
                .require_plan_approval(&session_record_id, &next.request_id)
                .is_ok()
            {
                let _ = app.emit(
                    "native-plan-approval-request",
                    plan_approval_event(&session_record_id, &next),
                );
            }
        }
        return Ok(None);
    }
    let retry = approved
        .then(|| {
            old.as_ref().filter(|plan| {
                plan.retry_matches(&pending)
                    && feedback.as_deref().unwrap_or_default().trim() == plan.feedback
                    && ai_channel_id
                        .as_deref()
                        .is_none_or(|value| value.trim() == plan.ai_channel_id)
                    && model
                        .as_deref()
                        .is_none_or(|value| value.trim() == plan.model)
                    && reasoning_effort
                        .as_deref()
                        .is_none_or(|value| Some(value.trim()) == plan.reasoning_effort.as_deref())
            })
        })
        .flatten();
    // A retry reuses the exact approved model and feedback, never a changed UI selection.
    let channel = retry
        .map(|plan| plan.ai_channel_id.clone())
        .or_else(|| ai_channel_id.filter(|value| !value.trim().is_empty()))
        .or_else(|| {
            runtime
                .as_ref()
                .map(|runtime| runtime.ai_channel_id.clone())
        })
        .or(record.ai_channel_id.clone())
        .ok_or_else(|| "必须选择实施渠道".to_string())?;
    let selected_model = retry
        .map(|plan| plan.model.clone())
        .or_else(|| model.filter(|value| !value.trim().is_empty()))
        .or_else(|| runtime.as_ref().map(|runtime| runtime.model.clone()));
    let effort = retry
        .map(|plan| plan.reasoning_effort.clone())
        .unwrap_or_else(|| {
            resolve_plan_implementation_effort(
                runtime
                    .as_ref()
                    .and_then(|runtime| runtime.reasoning_effort.as_deref()),
                reasoning_effort.as_deref(),
            )
        });
    let feedback = retry
        .map(|plan| plan.feedback.clone())
        .unwrap_or_else(|| feedback.unwrap_or_default().trim().to_string());
    let selected_model = match selected_model {
        Some(model) => model,
        None => sqlx::query_scalar::<_, String>("SELECT model FROM native_session_transcripts WHERE session_record_id = $1 AND deleted_at IS NULL")
            .bind(&session_record_id).fetch_optional(&pool).await.map_err(|error| error.to_string())?.unwrap_or_default(),
    };
    // Validate before persisting authorization or writing any file.
    let validated = load_native_client(
        &app,
        &pool,
        channel.trim(),
        selected_model.trim(),
        effort.as_deref(),
    )
    .await?;
    authorization.check()?;
    if !approved {
        plans::invalidate_persisted(&pool, &session_record_id, false).await?;
        let started = start_native_session_locked(
            app.clone(),
            state.inner().clone(),
            StartNativeSessionInput {
                ai_channel_id: validated.channel_id,
                workspace_id,
                prompt: format!(
                    "请根据反馈修改下面的计划，重新提交审批：\n\n{}\n\n反馈：{}",
                    pending.plan, feedback
                ),
                model: Some(validated.model),
                reasoning_effort: validated.effort,
                resume_session_id: Some(session_record_id.clone()),
                plan_mode: Some(true),
                permission_mode: runtime.map(|runtime| runtime.permission_mode),
                system_prompt: None,
                image_paths: None,
                isolate_worktree: Some(false),
                locale: None,
            },
            None,
        )
        .await?;
        emit_request_resolved(&app, &session_record_id, &request_id, "plan_approval");
        return Ok(Some(started));
    }
    let cwd = live_cwd
        .or(record.working_dir.clone())
        .ok_or_else(|| "会话缺少工作目录".to_string())?;
    let remote = record.execution_target == crate::app::shared::EXECUTION_TARGET_SSH;
    let relative = plans::relative_path(&session_record_id)?;
    // This is only a display target until the remote host resolves the cwd.
    let path = if remote {
        format!("{}/{}", cwd.trim_end_matches('/'), relative)
    } else {
        Path::new(&cwd)
            .join(&relative)
            .to_string_lossy()
            .into_owned()
    };
    // Keep an already-known canonical target across connection failures on retry.
    let known_target = retry.filter(|plan| remote && plan.cwd_resolved);
    let mut snapshot = ApprovedPlanSnapshot {
        authorization_id: new_id(),
        request_id: request_id.clone(),
        body: pending.plan,
        feedback,
        cwd: known_target
            .map(|plan| plan.cwd.clone())
            .unwrap_or_else(|| cwd.clone()),
        path: known_target
            .map(|plan| plan.path.clone())
            .unwrap_or_else(|| path.clone()),
        cwd_resolved: !remote || known_target.is_some(),
        content_hash: String::new(),
        saved_hash: old.as_ref().and_then(|plan| plan.saved_hash.clone()),
        saved_path: old
            .as_ref()
            .and_then(|plan| plan.last_saved_path().map(ToOwned::to_owned)),
        status: PlanSaveStatus::Saving,
        ai_channel_id: validated.channel_id.clone(),
        model: validated.model.clone(),
        reasoning_effort: validated.effort.clone(),
        error: None,
    };
    let content = snapshot.content();
    snapshot.content_hash = plans::hash(content.as_bytes());
    let outcome: Result<Option<AgentSessionStarted>, String> = async {
        let plan_ssh = if remote {
            Some(
                plans::resolve_authorized_ssh_target(
                    &pool,
                    &session_record_id,
                    &mut snapshot,
                    &authorization,
                    retry,
                    || async {
                        let config_id = record
                            .ssh_config_id
                            .as_deref()
                            .ok_or_else(|| "SSH 会话缺少配置".to_string())?;
                        let config = fetch_ssh_config_record_by_id(&pool, config_id).await?;
                        let mut ssh = SshToolRuntime {
                            app: app.clone(),
                            config,
                            root: cwd.clone(),
                            authorized_paths: Vec::new(),
                        };
                        ssh.root = plans::resolve_ssh_cwd(&ssh).await?;
                        Ok((ssh.root.clone(), ssh))
                    },
                )
                .await?,
            )
        } else {
            plans::store_approved(&pool, &session_record_id, &snapshot).await?;
            if retry.is_some_and(|plan| plan.cwd != cwd || plan.path != path) {
                authorization.cancel();
                return Err("计划工作目录已改变，请重新确认计划".into());
            }
            None
        };
        authorization.check()?;
        if let Some(ssh) = plan_ssh.as_ref() {
            plans::write_ssh(
                ssh,
                &session_record_id,
                &content,
                snapshot.expected_hash(),
                &authorization,
            )
            .await?;
        } else {
            let cwd = cwd.clone();
            let session = session_record_id.clone();
            let content = content.clone();
            let expected = snapshot.expected_hash().map(ToOwned::to_owned);
            let token = authorization.clone();
            tokio::task::spawn_blocking(move || {
                plans::write_local(
                    Path::new(&cwd),
                    &session,
                    &content,
                    expected.as_deref(),
                    &token,
                )
            })
            .await
            .map_err(|error| error.to_string())??;
        }
        // Even cancellation after an actual write retains the hash for a safe later retry.
        snapshot.saved_hash = Some(snapshot.content_hash.clone());
        snapshot.saved_path = Some(snapshot.path.clone());
        snapshot.status = PlanSaveStatus::Saved;
        plans::store_approved(&pool, &session_record_id, &snapshot).await?;
        authorization.check()?;
        if live {
            state
                .lock()
                .await
                .require_plan_approval(&session_record_id, &request_id)?;
            apply_plan_implementation_model(
                &app,
                state.inner(),
                &session_record_id,
                validated,
                &request_id,
                &authorization,
            )
            .await?;
            authorization.check()?;
            state.lock().await.resolve_plan_approval(
                &session_record_id,
                &request_id,
                PlanApprovalAnswer {
                    approved: true,
                    feedback: snapshot.feedback.clone(),
                    ai_channel_id: Some(snapshot.ai_channel_id.clone()),
                    model: Some(snapshot.model.clone()),
                    plan_path: Some(snapshot.path.clone()),
                    authorization: Some(authorization.clone()),
                },
            )?;
            authorization.wait_for_implementation().await?;
            Ok(None)
        } else {
            let started = start_native_session_locked(
                app.clone(),
                state.inner().clone(),
                StartNativeSessionInput {
                    ai_channel_id: snapshot.ai_channel_id.clone(),
                    workspace_id,
                    prompt: format!(
                        "已批准并保存计划，请开始实施。\n计划文件：{}\n\n{}",
                        snapshot.path, content
                    ),
                    model: Some(snapshot.model.clone()),
                    reasoning_effort: snapshot.reasoning_effort.clone(),
                    resume_session_id: Some(session_record_id.clone()),
                    plan_mode: Some(false),
                    permission_mode: runtime.map(|runtime| runtime.permission_mode),
                    system_prompt: None,
                    image_paths: None,
                    isolate_worktree: Some(false),
                    locale: None,
                },
                Some((&request_id, &authorization)),
            )
            .await?;
            Ok(Some(started))
        }
    }
    .await;
    match outcome {
        Ok(started) => {
            clear_pending_plan_request(&pool, &session_record_id, &request_id).await?;
            emit_request_resolved(&app, &session_record_id, &request_id, "plan_approval");
            let next = state
                .lock()
                .await
                .get_session(&session_record_id)
                .and_then(|session| {
                    session
                        .pending_plan_approval
                        .front()
                        .map(|pending| pending.request.clone())
                });
            if let Some(next) = next {
                save_pending_plan_with(&pool, &session_record_id, &next.request_id, &next.plan)
                    .await?;
                let manager = state.lock().await;
                if manager
                    .require_plan_approval(&session_record_id, &next.request_id)
                    .is_ok()
                {
                    let _ = app.emit(
                        "native-plan-approval-request",
                        plan_approval_event(&session_record_id, &next),
                    );
                }
            }
            Ok(started)
        }
        Err(error) => {
            plans::record_failure(
                &pool,
                &session_record_id,
                &mut snapshot,
                &authorization,
                &error,
            )
            .await?;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn answer_native_plan_question(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    request_id: String,
    skipped: bool,
    answers: Vec<String>,
) -> Result<(), String> {
    let mut manager = state.lock().await;
    let next = manager.resolve_question(
        &session_record_id,
        &request_id,
        PlanQuestionAnswer { skipped, answers },
    )?;
    emit_request_resolved(&app, &session_record_id, &request_id, "question");
    if let Some(request) = next {
        let _ = app.emit(
            "native-plan-question",
            question_event(&session_record_id, &request),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<(), String> {
    state
        .lock()
        .await
        .invalidate_plan_authorization(&session_record_id);
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    if !stop_native_process(
        &app,
        state.inner(),
        &session_record_id,
        "stopping_requested",
        "收到停止请求",
    )
    .await?
    {
        return Err(format!("未找到内置 Agent 会话 {session_record_id}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_native(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    profile_id: String,
) -> Result<(), String> {
    let processes = state.lock().await.get_profile_processes(&profile_id);
    for process in processes {
        state
            .lock()
            .await
            .invalidate_plan_authorization(&process.session_record_id);
        let _operation = lock_agent_session_operation(&state, &process.session_record_id).await;
        let _ = stop_native_process(
            &app,
            state.inner(),
            &process.session_record_id,
            "stopping_requested",
            "收到停止请求",
        )
        .await?;
    }
    Ok(())
}
