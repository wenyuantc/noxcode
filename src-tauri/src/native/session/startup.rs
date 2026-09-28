#[allow(unused_imports)]
use super::*;

#[tauri::command]
pub async fn start_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
    start_native_with_manager(app, state.inner().clone(), payload).await
}

pub(crate) async fn start_native_with_manager(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
    crate::app::lifecycle::require_running(&app)?;
    manager_state.lock().await.require_running()?;
    let _operation = match payload.resume_session_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => {
            let guard = lock_agent_session_operation(&manager_state, id).await;
            require_unarchived_session_with(&sqlite_pool(&app).await?, id).await?;
            Some(guard)
        }
        _ => None,
    };
    start_native_session_locked(app, manager_state, payload, None).await
}

pub(super) async fn start_native_session_locked(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    payload: StartNativeSessionInput,
    approved_request: Option<(&str, &PlanAuthorization)>,
) -> Result<AgentSessionStarted, String> {
    crate::app::lifecycle::require_running(&app)?;
    manager_state.lock().await.require_running()?;
    let plan_mode = payload.plan_mode.unwrap_or(false);
    let kind = session_kind(plan_mode);
    let workspace_id = payload.workspace_id.trim().to_string();
    if workspace_id.is_empty() {
        return Err("必须选择工作区".to_string());
    }
    let resume_id = payload
        .resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);
    if let Some(resume_id) = resume_id.as_deref() {
        let pool = sqlite_pool(&app).await?;
        if crate::native::file_rollback::has_unsettled(&pool, resume_id).await? {
            let files = live_files_for_session(&app, &manager_state, &pool, resume_id).await?;
            crate::native::file_rollback::block_until_rollback_settled(&pool, resume_id, &files)
                .await?;
        }
        let runtime = {
            let manager = manager_state.lock().await;
            if let Some(session) = manager.get_session(resume_id) {
                if session.info.workspace_id.as_deref() != Some(&workspace_id) {
                    return Err("会话不属于当前工作区".to_string());
                }
                let runtime = session.runtime_snapshot();
                if let Some(runtime) = &runtime {
                    validate_live_configuration(runtime, &payload)?;
                }
                runtime
            } else {
                None
            }
        };
        let attachment_root = app
            .path()
            .app_config_dir()
            .map_err(|error| format!("无法读取应用配置目录: {error}"))?;
        if let Some((info, queue)) = enqueue_live_input(
            manager_state.as_ref(),
            resume_id,
            &payload.prompt,
            payload.image_paths.as_deref(),
            Some(&pool),
            Some(&attachment_root),
        )
        .await?
        {
            let started = AgentSessionStarted {
                runtime,
                input_queue_id: Some(queue.queue_id),
                profile_id: info.profile_id,
                workspace_id: info.workspace_id.unwrap_or_else(|| workspace_id.clone()),
                session_kind: info.session_kind,
                session_record_id: info.session_record_id,
            };
            let _ = app.emit("native-session", &started);
            return Ok(started);
        }
    }

    let pool = sqlite_pool(&app).await?;
    if let Some(resume_id) = resume_id.as_deref() {
        if let Some((_, authorization)) = approved_request {
            authorization.check()?;
        }
        if plans::validate_resume(
            &pool,
            resume_id,
            plan_mode,
            approved_request.map(|(request, _)| request),
        )
        .await?
        {
            manager_state
                .lock()
                .await
                .invalidate_plan_authorization(resume_id);
            plans::invalidate_persisted(&pool, resume_id, true).await?;
        }
    }
    let execution_context =
        resolve_workspace_execution_context_with_pool(&pool, &workspace_id).await?;
    let workspace_root = execution_context
        .working_dir
        .clone()
        .ok_or_else(|| format!("{ENGINE_LABEL} 工作区缺少工作目录"))?;
    let mut run_cwd = workspace_root.clone();
    // 在创建/重新激活会话及发起模型请求前失败，避免所有远程工具反复触发同一门槛。
    let ssh_config = load_session_ssh_config(&pool, &execution_context).await?;

    let channel_id = payload.ai_channel_id.trim().to_string();
    if channel_id.is_empty() {
        return Err("必须选择 AI 渠道".to_string());
    }
    let model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("");
    let effort = payload
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty());
    let mut run = load_native_client(&app, &pool, &channel_id, model, effort).await?;
    run.profile_system_prompt = payload
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);

    let prompt = if plan_mode {
        payload.prompt.clone()
    } else if let Some(def) = run.bound_subagent.as_ref() {
        crate::native::prompt::wrap_prompt_for_required_subagent(&payload.prompt, &def.name)
    } else {
        payload.prompt.clone()
    };

    let session_record_id = resume_id.clone().unwrap_or_else(new_id);
    let _new_operation = if resume_id.is_none() {
        Some(lock_agent_session_operation(&manager_state, &session_record_id).await)
    } else {
        None
    };
    let native_settings = crate::native::settings::load_native_settings(&app).ok();
    let isolate = payload.isolate_worktree.unwrap_or(false);
    let existing_working_dir = if let Some(resume_id) = resume_id.as_deref() {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT working_dir FROM agent_sessions WHERE id = $1",
        )
        .bind(resume_id)
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .flatten()
    } else {
        None
    };
    let (session_worktree, startup_notices) = maybe_isolate_session_worktree(
        &app,
        &workspace_id,
        &session_record_id,
        &mut run_cwd,
        isolate,
        resume_id.is_some(),
        existing_working_dir.as_deref(),
    )
    .await;
    if let Some((_, authorization)) = approved_request {
        authorization.check()?;
        let approved = plans::load_approved(&pool, &session_record_id)
            .await?
            .ok_or_else(|| "缺少已保存计划".to_string())?;
        let implementation_cwd = if let Some(config) = ssh_config.as_ref() {
            plans::resolve_ssh_cwd(&SshToolRuntime {
                app: app.clone(),
                config: config.clone(),
                root: run_cwd.clone(),
                authorized_paths: Vec::new(),
            })
            .await?
        } else {
            run_cwd.clone()
        };
        if approved.cwd != implementation_cwd {
            return Err("实施工作目录与计划保存目录不一致".into());
        }
    }
    let sandbox_active = native_settings
        .as_ref()
        .is_some_and(|settings| settings.bash_sandbox_enabled)
        && ssh_config.is_none();
    let session_record_id = if let Some(resume_id) = resume_id.as_deref() {
        reactivate_agent_session(
            &pool,
            resume_id,
            &workspace_id,
            &channel_id,
            &run_cwd,
            &execution_context.execution_target,
            execution_context.ssh_config_id.as_deref(),
            execution_context.target_host_label.as_deref(),
            &kind,
        )
        .await?
    } else {
        insert_agent_session(
            &pool,
            &session_record_id,
            &channel_id,
            &workspace_id,
            &run_cwd,
            &execution_context.execution_target,
            execution_context.ssh_config_id.as_deref(),
            execution_context.target_host_label.as_deref(),
            &kind,
            None,
            &payload.prompt,
        )
        .await?
    };

    if resume_id.is_none() {
        let _ = insert_session_event(
            &pool,
            &session_record_id,
            "session_requested",
            Some("内置 Agent 会话已创建"),
        )
        .await;
        crate::native::ai_features::spawn_session_title_generation(
            app.clone(),
            session_record_id.clone(),
            workspace_id.clone(),
            payload.prompt.clone(),
            payload.locale.clone(),
        );
    }

    run.client = run
        .client
        .with_call_log_context(CallLogContext::for_session(
            Some(run.channel_id.clone()),
            Some(run.channel_name.clone()),
            Some(session_record_id.clone()),
            None,
            Some(workspace_id.clone()),
            if plan_mode {
                CALL_KIND_PLAN
            } else {
                CALL_KIND_CHAT
            },
            Some(execution_context.execution_target.clone()),
        ))
        // OpenAI / Responses 按会话打 prompt_cache_key，Anthropic 走 cache_control。
        .with_prompt_cache_key(session_record_id.clone());

    let ssh = ssh_config.map(|config| SshToolRuntime {
        app: app.clone(),
        config,
        root: run_cwd.clone(),
        authorized_paths: Vec::new(),
    });

    if let Ok(target) = crate::git::resolve_git_target_at(&app, &workspace_id, &run_cwd).await {
        if let Err(error) = create_checkpoint(
            &pool,
            &target,
            &workspace_id,
            &session_record_id,
            Some("session_start"),
            Some("session_start"),
        )
        .await
        {
            eprintln!("[native] session_start 打点失败: {error}");
        }
    }

    let permission_mode = payload
        .permission_mode
        .as_deref()
        .map(|mode| crate::native::settings::normalize_permission_mode(Some(mode)))
        .unwrap_or_else(|| crate::native::settings::effective_permission_mode(&app));
    let runtime = NativeSessionRuntime {
        ai_channel_id: run.channel_id.clone(),
        model: run.model.clone(),
        reasoning_effort: run.effort.clone(),
        permission_mode: permission_mode.clone(),
        plan_mode,
        worktree_path: session_worktree.clone(),
        sandbox_active,
    };
    let input_queue = Arc::new(NativeInputQueue::new(&session_record_id));
    let steer_pool = pool.clone();
    let steer_app = app.clone();
    input_queue.steer.configure(
        Arc::new(move |receipt| {
            let pool = steer_pool.clone();
            Box::pin(async move { persist_steer_receipt(&pool, &receipt).await })
        }),
        Arc::new(move |snapshot| {
            let _ = steer_app.emit("native-steer", snapshot);
        }),
    );
    let queue_app = app.clone();
    input_queue.set_on_change(Arc::new(move |snapshot| {
        let _ = queue_app.emit("native-input-queue", snapshot);
    }));
    let started = AgentSessionStarted {
        runtime: Some(runtime.clone()),
        input_queue_id: Some(input_queue.id.clone()),
        profile_id: String::new(),
        workspace_id: workspace_id.clone(),
        session_kind: kind.clone(),
        session_record_id: session_record_id.clone(),
    };
    let (followup_tx, followup_rx) = mpsc::channel(8);
    let (config_tx, config_rx) = mpsc::channel(8);
    let cancel = crate::native::tools::CancelFlag::new();
    let allow_all_high_risk = Arc::new(AtomicBool::new(
        crate::native::settings::permission_mode_is_yolo(&permission_mode),
    ));
    let permission_storage_root =
        if execution_context.execution_target == crate::app::shared::EXECUTION_TARGET_LOCAL {
            Some(PathBuf::from(&workspace_root))
        } else {
            app.path()
                .app_config_dir()
                .ok()
                .map(|dir| crate::native::permission_rules::ssh_rules_root(&dir, &workspace_id))
        };
    let permission_rules =
        crate::native::permission_rules::shared_rules(match app.path().app_config_dir() {
            Ok(dir) => crate::native::permission_rules::load_effective_rules(
                &dir,
                permission_storage_root.as_deref(),
            ),
            Err(_) => Default::default(),
        });
    let working = Arc::new(AtomicBool::new(true));
    let (loop_ready_tx, loop_ready_rx) = tokio::sync::oneshot::channel();
    let manager_spawn = manager_state.clone();
    let app_spawn = app.clone();
    let cancel_run = cancel.clone();
    let allow_all_run = allow_all_high_risk.clone();
    let working_run = working.clone();
    let session_spawn = session_record_id.clone();
    let profile_spawn = String::new();
    let workspace_spawn = workspace_id.clone();
    let kind_spawn = kind.clone();
    let image_paths = payload.image_paths.clone();
    let resume_run = payload.resume_session_id.clone();
    let rules_run = permission_rules.clone();
    let queue_run = input_queue.clone();
    let context_token_limit =
        crate::native::settings::session_context_window_tokens(&app, run.context_tokens) as usize;
    let transcript_model = Arc::new(Mutex::new(run.model.clone()));
    let live_model = Arc::new(std::sync::Mutex::new(live_snapshot_from_run(
        &run,
        context_token_limit,
        Some(execution_context.execution_target.clone()),
        None,
    )));
    let live_model_run = live_model.clone();
    let transcript_model_run = transcript_model.clone();
    let join = tokio::spawn(async move {
        if loop_ready_rx.await.is_err() {
            return;
        }
        run_native_loop(
            app_spawn,
            manager_spawn,
            run,
            prompt,
            run_cwd,
            ssh,
            cancel_run,
            allow_all_run,
            working_run,
            rules_run,
            followup_rx,
            config_rx,
            queue_run,
            session_spawn,
            profile_spawn,
            workspace_spawn,
            kind_spawn,
            image_paths,
            plan_mode,
            resume_run,
            permission_mode,
            live_model_run,
            transcript_model_run,
            workspace_root,
            session_worktree,
            startup_notices,
        )
        .await;
    });

    let registered = manager_state.lock().await.add_session(NativeLiveSession {
        runtime: Some(runtime),
        plan_mode: Arc::new(AtomicBool::new(plan_mode)),
        background: None,
        processes: None,
        closing: false,
        info: NativeSessionInfo {
            profile_id: String::new(),
            channel_id: channel_id.clone(),
            workspace_id: Some(workspace_id.clone()),
            session_kind: kind,
            session_record_id: session_record_id.clone(),
        },
        cancel,
        followup_tx,
        config_tx,
        input_queue,
        join,
        allow_all_high_risk,
        allow_session_commands: Arc::default(),
        working,
        pending_compactions: Arc::default(),
        permission_rules,
        workspace_root: permission_storage_root,
        pending_permission: std::collections::VecDeque::new(),
        pending_question: std::collections::VecDeque::new(),
        pending_plan_approval: std::collections::VecDeque::new(),
        live_model: Some(live_model.clone()),
        transcript_model: Some(transcript_model.clone()),
    });
    if !registered {
        let _ = update_agent_session_status(
            &pool,
            &session_record_id,
            "exited",
            Some(0),
            Some(&now_sqlite()),
        )
        .await;
        return Err("应用正在退出，会话未启动".into());
    }
    if let Some((_, authorization)) = approved_request {
        if let Err(error) = authorization.commit_implementation(|| {
            let _ = loop_ready_tx.send(());
        }) {
            manager_state
                .lock()
                .await
                .remove_session(&session_record_id);
            update_agent_session_status(
                &pool,
                &session_record_id,
                "exited",
                Some(0),
                Some(&now_sqlite()),
            )
            .await?;
            return Err(error);
        }
    } else {
        let _ = loop_ready_tx.send(());
    }
    let _ = app.emit("native-session", &started);
    Ok(started)
}

pub(super) fn validate_live_configuration(
    runtime: &NativeSessionRuntime,
    payload: &StartNativeSessionInput,
) -> Result<(), String> {
    let differs = payload.ai_channel_id.trim() != runtime.ai_channel_id
        || payload
            .model
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .is_some_and(|value| value.trim() != runtime.model)
        || payload
            .reasoning_effort
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .is_some_and(|value| Some(value.trim()) != runtime.reasoning_effort.as_deref())
        || payload
            .plan_mode
            .is_some_and(|value| value != runtime.plan_mode)
        || payload.permission_mode.as_deref().is_some_and(|value| {
            crate::native::settings::normalize_permission_mode(Some(value))
                != runtime.permission_mode
        });
    if differs {
        Err("运行配置已变更，请先正常结束空闲会话，再使用新配置继续".to_string())
    } else {
        Ok(())
    }
}

pub(crate) enum NativeIdleWait {
    Followup(Option<NativeFollowup>),
    Configuration(Option<NativeConfigurationRequest>),
    Input(Option<crate::native::input_queue::NativeQueuedInput>),
}

pub(crate) async fn recv_idle_wait(
    followup_rx: &mut mpsc::Receiver<NativeFollowup>,
    config_rx: &mut mpsc::Receiver<NativeConfigurationRequest>,
    input_queue: &NativeInputQueue,
    cancel: &crate::native::tools::CancelFlag,
    working: &AtomicBool,
) -> NativeIdleWait {
    tokio::select! {
        biased;
        followup = followup_rx.recv() => NativeIdleWait::Followup(followup),
        config = config_rx.recv() => NativeIdleWait::Configuration(config),
        input = input_queue.recv(cancel, working) => NativeIdleWait::Input(input),
    }
}

pub(super) fn apply_run_settings_to_runner(runner: &mut AgentRunner, run: &NativeRunSettings) {
    runner.lite_model = run.lite_model.clone();
    runner.ctx.hook_agent = Some(hook_agent_handler(run));
    if let Some(scope) = runner.ctx.session_scope.as_mut() {
        scope.channel_id = run.channel_id.clone();
        scope.model = run.model.clone();
    }
}

pub(super) async fn save_pending_plan(
    app: &AppHandle,
    session_record_id: &str,
    request_id: &str,
    plan: &str,
) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    save_pending_plan_with(&pool, session_record_id, request_id, plan).await
}

pub(super) async fn save_pending_plan_with(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    request_id: &str,
    plan: &str,
) -> Result<(), String> {
    let snapshot = PendingPlanSnapshot {
        request_id: request_id.to_string(),
        plan: plan.to_string(),
        created_at: now_sqlite(),
    };
    let payload = serde_json::to_string(&snapshot)
        .map_err(|error| format!("序列化待批准计划失败: {error}"))?;
    sqlx::query("UPDATE agent_sessions SET pending_plan_json = $1, approved_plan_json = CASE WHEN json_extract(approved_plan_json, '$.status') IN ('saving', 'failed') THEN json_set(approved_plan_json, '$.status', 'cancelled') ELSE approved_plan_json END WHERE id = $2")
        .bind(payload)
        .bind(session_record_id)
        .execute(pool)
        .await
        .map_err(|error| format!("保存待批准计划失败: {error}"))?;
    Ok(())
}

pub(super) async fn clear_pending_plan(
    app: &AppHandle,
    session_record_id: &str,
) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    clear_pending_plan_with(&pool, session_record_id).await
}

pub(super) async fn clear_pending_plan_with(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE agent_sessions SET pending_plan_json = NULL WHERE id = $1 AND pending_plan_json IS NOT NULL",
    )
    .bind(session_record_id)
    .execute(pool)
    .await
    .map_err(|error| format!("清除待批准计划失败: {error}"))?;
    Ok(())
}

pub(super) async fn update_agent_session_channel(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    ai_channel_id: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE agent_sessions SET ai_channel_id = $1 WHERE id = $2")
        .bind(ai_channel_id)
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(|error| format!("更新会话渠道失败: {error}"))?;
    Ok(())
}

pub(super) fn bind_run_to_session(
    mut run: NativeRunSettings,
    session_record_id: &str,
    workspace_id: &str,
    plan_mode: bool,
    execution_target: Option<String>,
) -> NativeRunSettings {
    run.client = run
        .client
        .with_call_log_context(CallLogContext::for_session(
            Some(run.channel_id.clone()),
            Some(run.channel_name.clone()),
            Some(session_record_id.to_string()),
            None,
            Some(workspace_id.to_string()),
            if plan_mode {
                CALL_KIND_PLAN
            } else {
                CALL_KIND_CHAT
            },
            execution_target,
        ))
        .with_prompt_cache_key(session_record_id.to_string());
    run
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_session_configuration(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    run: &mut NativeRunSettings,
    runner: &mut AgentRunner,
    request: &NativeConfigurationRequest,
    session_record_id: &str,
    workspace_id: &str,
    transcript_model: &Arc<Mutex<String>>,
    last_transcript_fingerprint: &Arc<Mutex<Option<u64>>>,
    profile_id: &str,
) -> Result<(NativeSessionRuntime, bool), String> {
    let pool = sqlite_pool(app).await?;
    let mut next = load_native_client(
        app,
        &pool,
        request.ai_channel_id.trim(),
        request.model.trim(),
        request.reasoning_effort.as_deref(),
    )
    .await?;
    next.profile_system_prompt = run.profile_system_prompt.clone();
    next.bound_subagent = run.bound_subagent.clone();
    next = bind_run_to_session(
        next,
        session_record_id,
        workspace_id,
        runner.is_plan_mode(),
        if runner.ctx.ssh.is_some() {
            Some(crate::app::shared::EXECUTION_TARGET_SSH.to_string())
        } else {
            Some(crate::app::shared::EXECUTION_TARGET_LOCAL.to_string())
        },
    );
    update_agent_session_channel(&pool, session_record_id, &next.channel_id).await?;
    if let Some(recovery) = run.client.recovery_state() {
        next.client = next.client.with_recovery(recovery);
    }
    let runtime = {
        let mut manager = manager_state.lock().await;
        let session = manager
            .get_session_mut(session_record_id)
            .ok_or_else(|| "会话已结束".to_string())?;
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
        runtime
    };
    apply_run_settings_to_runner(runner, &next);
    configure_runner_limits(app, runner, next.context_tokens);
    *transcript_model.lock().await = next.model.clone();
    let compacted = if runner.context_window.should_compact(&runner.messages) {
        runner
            .compact_now_with(&next.client, CompactTrigger::Downshift, None)
            .await?
            .is_some()
    } else {
        false
    };
    persist_runner_transcript(
        app,
        session_record_id,
        profile_id,
        workspace_id,
        &next.model,
        &runner.messages,
        last_transcript_fingerprint.as_ref(),
    )
    .await;
    *run = next;
    Ok((runtime, compacted))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_session_configuration(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    run: &mut NativeRunSettings,
    runner: &mut AgentRunner,
    request: NativeConfigurationRequest,
    session_record_id: &str,
    workspace_id: &str,
    transcript_model: &Arc<Mutex<String>>,
    last_transcript_fingerprint: &Arc<Mutex<Option<u64>>>,
    profile_id: &str,
    input_queue_id: &str,
    config_revision: &mut u64,
    live_model: &SharedLiveModel,
) {
    let request_id = request.request_id.clone();
    let result = apply_session_configuration(
        app,
        manager_state,
        run,
        runner,
        &request,
        session_record_id,
        workspace_id,
        transcript_model,
        last_transcript_fingerprint,
        profile_id,
    )
    .await;
    let event = match result {
        Ok((runtime, compacted)) => {
            let execution_target = if runner.ctx.ssh.is_some() {
                Some(crate::app::shared::EXECUTION_TARGET_SSH.to_string())
            } else {
                Some(crate::app::shared::EXECUTION_TARGET_LOCAL.to_string())
            };
            publish_live_run(live_model, runner, run, app, execution_target);
            *config_revision = config_revision.saturating_add(1);
            NativeSessionConfigurationEvent {
                session_record_id: session_record_id.to_string(),
                request_id,
                revision: *config_revision,
                input_queue_id: Some(input_queue_id.to_string()),
                runtime: Some(runtime),
                compacted,
                error: None,
            }
        }
        Err(error) => NativeSessionConfigurationEvent {
            session_record_id: session_record_id.to_string(),
            request_id,
            revision: 0,
            input_queue_id: Some(input_queue_id.to_string()),
            runtime: None,
            compacted: false,
            error: Some(error),
        },
    };
    let _ = app.emit("native-session-configuration", &event);
    if let Some(error) = event.error.clone() {
        let _ = request.reply.send(Err(error));
    } else {
        let _ = request.reply.send(Ok(event));
    }
}
