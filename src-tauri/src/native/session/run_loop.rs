#[allow(unused_imports)]
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_native_loop(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    mut run: NativeRunSettings,
    first_prompt: String,
    run_cwd: String,
    ssh: Option<SshToolRuntime>,
    cancel: crate::native::tools::CancelFlag,
    allow_all_high_risk: Arc<AtomicBool>,
    working: Arc<AtomicBool>,
    permission_rules: crate::native::permission_rules::SharedPermissionRules,
    followup_rx: mpsc::Receiver<NativeFollowup>,
    mut config_rx: mpsc::Receiver<NativeConfigurationRequest>,
    input_queue: Arc<NativeInputQueue>,
    session_record_id: String,
    profile_id: String,
    workspace_id: String,
    kind: String,
    image_paths: Option<Vec<String>>,
    plan_mode: bool,
    resume_session_id: Option<String>,
    permission_mode: String,
    live_model: SharedLiveModel,
    transcript_model: Arc<Mutex<String>>,
    workspace_root: String,
    session_worktree: Option<String>,
    startup_notices: Vec<String>,
) {
    let followup_rx = Arc::new(Mutex::new(followup_rx));
    let mut config_revision = 0_u64;
    let mut runner = AgentRunner::new(LocalWorkspace::new(PathBuf::from(&run_cwd)));
    runner.ctx.ssh = ssh;
    let web_settings_app = app.clone();
    runner.ctx.web_settings_provider =
        Some(Arc::new(move || load_network_settings(&web_settings_app)));
    runner.ctx.extra_env = load_network_settings(&app)
        .map(|settings| proxy_env_vars(&settings))
        .unwrap_or_default();
    runner.ctx.original_root = PathBuf::from(&workspace_root);
    if let Ok(mut root) = runner.ctx.active_root.write() {
        *root = PathBuf::from(&run_cwd);
    }
    if let Ok(mut stored) = runner.ctx.worktree_path.write() {
        *stored = session_worktree;
    }
    configure_local_tool_runtime(&app, &mut runner, &session_record_id, &workspace_id).await;
    runner.ctx.cancel = cancel.clone();
    runner.ctx.allow_all_high_risk = allow_all_high_risk;
    runner.ctx.permission_rules = permission_rules;
    let background_app = app.clone();
    let background_session = session_record_id.clone();
    runner.background.set_on_change(Some(Arc::new(move |tasks| {
        let _ = background_app.emit(
            "native-background-tasks",
            serde_json::json!({
                "session_record_id": background_session,
                "tasks": tasks,
            }),
        );
    })));
    {
        let mut manager = manager_state.lock().await;
        if let Some(session) = manager.get_session_mut(&session_record_id) {
            session.background = Some(runner.background.clone());
            session.processes = runner.ctx.processes.clone();
            runner.ctx.plan_mode = session.plan_mode.clone();
            runner.ctx.allow_session_commands = session.allow_session_commands.clone();
        }
        manager.attach_workspace_roots(
            &session_record_id,
            crate::native::manager::SessionWorkspaceRoots {
                active_root: runner.ctx.active_root.clone(),
                worktree_path: runner.ctx.worktree_path.clone(),
            },
        );
    }
    runner.steer_rx = Some(followup_rx.clone());
    runner.ctx.user_steer = Some(input_queue.steer.clone());
    runner.ctx.main_origin = Some(input_queue.steer.begin_turn().await);
    emit_turn_state(
        &app,
        &input_queue.steer,
        &working,
        "working",
        runner.on_event.as_ref(),
    )
    .await;
    if plan_mode {
        runner.set_read_only(true);
        runner.set_plan_mode(true);
    }
    let plan_mode_app = app.clone();
    let plan_mode_session = session_record_id.clone();
    let plan_mode_queue = input_queue.id.clone();
    runner.ctx.on_plan_mode_change = Some(Arc::new(move |value| {
        emit_plan_mode(&plan_mode_app, &plan_mode_session, &plan_mode_queue, value);
    }));
    // The initial event also covers callers that start a session without the
    // Composer (for example, scheduled runs); the frontend has a session_kind
    // fallback when this event races listener registration.
    emit_plan_mode(
        &app,
        &session_record_id,
        &input_queue.id,
        runner.is_plan_mode(),
    );
    attach_mutation_checkpoint(
        &app,
        &mut runner,
        workspace_id.clone(),
        session_record_id.clone(),
    );
    let announce_startup = should_announce_session_startup(resume_session_id.as_deref());
    runner.ctx.auto_approve_overwrite =
        crate::native::settings::permission_mode_auto_approves_edits(&permission_mode);
    let build_mode = crate::native::settings::permission_mode_auto_approves_build(&permission_mode);
    runner.ctx.auto_approve_opaque_bash = build_mode;
    runner.ctx.auto_approve_readonly_mcp = build_mode;
    if announce_startup {
        let notice = match permission_mode.as_str() {
            crate::native::settings::PERMISSION_MODE_YOLO => Some(
                "[PERMISSION] 完全访问（yolo）：不弹权限确认（含 MCP、工作区钩子与命令）；deny 规则仍拒绝",
            ),
            crate::native::settings::PERMISSION_MODE_BUILD => Some(
                "[PERMISSION] 自动构建（build）：覆盖文件、不透明命令与只读 MCP 直接执行，删除 / 推送 / 强制 Git / 写入型 MCP 仍需确认",
            ),
            crate::native::settings::PERMISSION_MODE_EDIT => Some(
                "[PERMISSION] 自动编辑（edit）：覆盖文件直接执行，删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍需确认",
            ),
            _ => None,
        };
        if let Some(notice) = notice {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                notice.to_string(),
            )
            .await;
        }
        for notice in &startup_notices {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                format!("[WORKTREE] {notice}"),
            )
            .await;
        }
        if let Some(path) = runner
            .ctx
            .worktree_path
            .read()
            .ok()
            .and_then(|slot| slot.clone())
        {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                format!("[WORKTREE] 会话工作目录已隔离到 {path}"),
            )
            .await;
        }
        let rules = runner.ctx.permission_rules_snapshot();
        if !rules.is_empty() {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                format!(
                    "[PERMISSION] 已加载权限规则：允许 {} / 拒绝 {} / 需确认 {}",
                    rules.allow.len(),
                    rules.deny.len(),
                    rules.ask.len()
                ),
            )
            .await;
        }
    }
    // 非 yolo 仍要确认通道：ask 规则、MCP、计划 Bash 与工作区钩子信任。
    {
        let app_perm = app.clone();
        let manager_perm = manager_state.clone();
        let session_perm = session_record_id.clone();
        let profile_perm = profile_id.clone();
        let workspace_perm = workspace_id.clone();
        let kind_perm = kind.clone();
        runner.ctx.request_permission = Some(std::sync::Arc::new(move |prompt, reply| {
            let app = app_perm.clone();
            let manager_state = manager_perm.clone();
            let session_record_id = session_perm.clone();
            let profile_id = profile_perm.clone();
            let workspace_id = workspace_perm.clone();
            let kind = kind_perm.clone();
            tauri::async_runtime::spawn(async move {
                let request = PermissionRequest {
                    origin: prompt.origin.clone(),
                    request_id: prompt.request_id.clone(),
                    profile_id: profile_id.clone(),
                    workspace_id: Some(workspace_id.clone()),
                    session_kind: kind.clone(),
                    tool_name: prompt.tool_name.clone(),
                    kind: prompt.kind,
                    summary: prompt.summary.clone(),
                    remote: prompt.remote,
                    mcp_server_id: prompt.mcp_server_id.clone(),
                    suggested_rule: prompt.suggested_rule.clone(),
                    file_access: prompt.file_access.clone(),
                    allow_once_only: prompt.allow_once_only,
                };
                let should_emit = {
                    let mut manager = manager_state.lock().await;
                    match manager.enqueue_permission(
                        &session_record_id,
                        PendingPermission {
                            request: request.clone(),
                            reply,
                        },
                    ) {
                        Ok(should_emit) => should_emit,
                        Err(_) => return,
                    }
                };
                let location = if prompt.remote {
                    "远程工作区"
                } else {
                    "本地工作区"
                };
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    format!(
                        "[PERMISSION] 等待确认高风险操作（{location} / {}）：{}",
                        prompt.kind.zh_label(),
                        prompt.summary
                    ),
                )
                .await;
                if should_emit {
                    let manager = manager_state.lock().await;
                    if manager
                        .get_session(&session_record_id)
                        .and_then(|session| session.pending_permission.front())
                        .is_none_or(|pending| pending.request.request_id != request.request_id)
                    {
                        return;
                    }
                    let _ = app.emit(
                        "native-permission-request",
                        permission_event(&session_record_id, &request),
                    );
                    crate::app::notifications::notify_if_unfocused(
                        &app,
                        "等待权限确认",
                        &prompt.summary,
                    );
                }
            });
        }));
        let expire_app = app.clone();
        let expire_manager = manager_state.clone();
        let expire_session = session_record_id.clone();
        let expire_profile = profile_id.clone();
        let expire_workspace = workspace_id.clone();
        let expire_kind = kind.clone();
        runner.ctx.expire_permission = Some(std::sync::Arc::new(move |request_id: String| {
            let app = expire_app.clone();
            let manager_state = expire_manager.clone();
            let session_record_id = expire_session.clone();
            let profile_id = expire_profile.clone();
            let workspace_id = expire_workspace.clone();
            let kind = expire_kind.clone();
            tauri::async_runtime::spawn(async move {
                let next = {
                    let mut manager = manager_state.lock().await;
                    manager
                        .expire_permission(&session_record_id, &request_id)
                        .ok()
                        .flatten()
                };
                emit_request_resolved(&app, &session_record_id, &request_id, "permission");
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    "[PERMISSION] 确认请求已失效，已按拒绝处理".to_string(),
                )
                .await;
                if let Some(request) = next {
                    let manager = manager_state.lock().await;
                    if !manager
                        .get_session(&session_record_id)
                        .and_then(|s| s.pending_permission.front())
                        .is_some_and(|p| {
                            p.request.request_id == request.request_id && !p.reply.is_closed()
                        })
                    {
                        return;
                    }
                    let _ = app.emit(
                        "native-permission-request",
                        permission_event(&session_record_id, &request),
                    );
                }
            })
        }));
    }
    // AskUserQuestion 在所有模式可用；ExitPlanMode 需要用户批准计划。
    {
        let app_q = app.clone();
        let manager_q = manager_state.clone();
        let session_q = session_record_id.clone();
        let profile_q = profile_id.clone();
        let workspace_q = workspace_id.clone();
        let kind_q = kind.clone();
        runner.ctx.request_plan_approval = Some(std::sync::Arc::new(move |prompt, reply| {
            let app = app_q.clone();
            let manager_state = manager_q.clone();
            let session_record_id = session_q.clone();
            let profile_id = profile_q.clone();
            let workspace_id = workspace_q.clone();
            let kind = kind_q.clone();
            tauri::async_runtime::spawn(async move {
                let request = PlanApprovalRequest {
                    origin: prompt.origin.clone(),
                    request_id: prompt.request_id.clone(),
                    profile_id: profile_id.clone(),
                    workspace_id: Some(workspace_id.clone()),
                    session_kind: kind.clone(),
                    plan: prompt.plan.clone(),
                };
                {
                    let mut manager = manager_state.lock().await;
                    if !manager
                        .get_session(&session_record_id)
                        .is_some_and(|session| session.accepts_origin(request.origin.as_ref()))
                    {
                        return;
                    }
                    manager.invalidate_plan_authorization(&session_record_id);
                }
                let _operation =
                    lock_agent_session_operation(&manager_state, &session_record_id).await;
                let should_emit = {
                    let mut manager = manager_state.lock().await;
                    match manager.enqueue_plan_approval(
                        &session_record_id,
                        PendingPlanApproval {
                            request: request.clone(),
                            reply,
                        },
                    ) {
                        Ok(should_emit) => should_emit,
                        Err(_) => return,
                    }
                };
                // 落库后即使会话被停止或应用退出，重新打开仍能继续这份计划。
                if !should_emit {
                    return;
                }
                if let Err(error) =
                    save_pending_plan(&app, &session_record_id, &request.request_id, &prompt.plan)
                        .await
                {
                    manager_state
                        .lock()
                        .await
                        .expire_plan_approval(&session_record_id, &request.request_id);
                    eprintln!("[native] {error}");
                    return;
                }
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    format!("[PLAN]\n{}", prompt.plan),
                )
                .await;
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    "[PLAN] 等待用户批准计划".to_string(),
                )
                .await;
                if should_emit {
                    let manager = manager_state.lock().await;
                    if !manager
                        .get_session(&session_record_id)
                        .and_then(|session| session.pending_plan_approval.front())
                        .is_some_and(|pending| {
                            pending.request.request_id == request.request_id
                                && !pending.reply.is_closed()
                        })
                    {
                        return;
                    }
                    let _ = app.emit(
                        "native-plan-approval-request",
                        plan_approval_event(&session_record_id, &request),
                    );
                    crate::app::notifications::notify_if_unfocused(
                        &app,
                        "等待批准计划",
                        "Agent 已提交计划，等待你批准或退回",
                    );
                }
            });
        }));
        let app_q = app.clone();
        let manager_q = manager_state.clone();
        let session_q = session_record_id.clone();
        let profile_q = profile_id.clone();
        let workspace_q = workspace_id.clone();
        let kind_q = kind.clone();
        let expire_app = app.clone();
        let expire_manager = manager_state.clone();
        let expire_session = session_record_id.clone();
        runner.ctx.expire_plan_approval = Some(Arc::new(move |request_id| {
            let app = expire_app.clone();
            let manager = expire_manager.clone();
            let session_id = expire_session.clone();
            tauri::async_runtime::spawn(async move {
                let next = manager
                    .lock()
                    .await
                    .expire_plan_approval(&session_id, &request_id);
                emit_request_resolved(&app, &session_id, &request_id, "plan_approval");
                if let Some(request) = next {
                    let guard = manager.lock().await;
                    if guard
                        .require_plan_approval(&session_id, &request.request_id)
                        .is_err()
                    {
                        return;
                    }
                    let _ = app.emit(
                        "native-plan-approval-request",
                        plan_approval_event(&session_id, &request),
                    );
                }
            })
        }));
        runner.ctx.request_question = Some(std::sync::Arc::new(move |questions, origin, reply| {
            let app = app_q.clone();
            let manager_state = manager_q.clone();
            let session_record_id = session_q.clone();
            let profile_id = profile_q.clone();
            let workspace_id = workspace_q.clone();
            let kind = kind_q.clone();
            tauri::async_runtime::spawn(async move {
                let request_id = uuid::Uuid::new_v4().to_string();
                let request = PlanQuestionRequest {
                    origin,
                    request_id: request_id.clone(),
                    profile_id: profile_id.clone(),
                    workspace_id: Some(workspace_id.clone()),
                    session_kind: kind.clone(),
                    questions: questions.clone(),
                };
                let should_emit = {
                    let mut manager = manager_state.lock().await;
                    match manager.enqueue_question(
                        &session_record_id,
                        PendingPlanQuestion {
                            request: request.clone(),
                            reply,
                        },
                    ) {
                        Ok(should_emit) => should_emit,
                        Err(_) => return,
                    }
                };
                let summary = questions
                    .iter()
                    .map(|item| item.prompt.as_str())
                    .collect::<Vec<_>>()
                    .join("；");
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    format!("[PLAN] 等待用户回答：{summary}"),
                )
                .await;
                if should_emit {
                    let manager = manager_state.lock().await;
                    if !manager
                        .get_session(&session_record_id)
                        .and_then(|s| s.pending_question.front())
                        .is_some_and(|p| {
                            p.request.request_id == request.request_id && !p.reply.is_closed()
                        })
                    {
                        return;
                    }
                    let _ = app.emit(
                        "native-plan-question",
                        question_event(&session_record_id, &request),
                    );
                    crate::app::notifications::notify_if_unfocused(
                        &app,
                        "等待回答计划问题",
                        &summary,
                    );
                }
            });
        }));
    }
    runner.max_turns = crate::native::settings::effective_max_turns(&app);
    runner.max_concurrent_subagents =
        crate::native::settings::effective_max_concurrent_subagents(&app);
    runner.subagent_policy = crate::native::settings::effective_subagent_policy(&app);
    configure_runner_limits(&app, &mut runner, run.context_tokens);
    let git_target = crate::git::resolve_git_target(&app, &workspace_id)
        .await
        .ok();
    let mut parts = crate::native::prompt::NativePromptParts {
        cwd: run_cwd.clone(),
        model: run.model.clone(),
        platform: std::env::consts::OS.to_string(),
        git: if let Some(target) = git_target.as_ref() {
            crate::native::prompt::detect_git(target).await
        } else {
            None
        },
        global_template: crate::native::prompt::load_global_template(&app),
        project_agents: if let Some(ssh) = runner.ctx.ssh.as_ref() {
            crate::native::prompt::read_ssh_project_agents(ssh).await
        } else {
            crate::native::prompt::read_local_project_agents(&run_cwd)
        },
        profile_prompt: run.profile_system_prompt.clone().unwrap_or_default(),
        max_concurrent_subagents: runner.max_concurrent_subagents,
        subagent_policy: runner.subagent_policy.clone(),
        identity_override: String::new(),
        required_subagent_name: String::new(),
        required_subagent_description: String::new(),
        permission_mode: if plan_mode {
            crate::native::settings::PERMISSION_MODE_PLAN.to_string()
        } else {
            permission_mode.clone()
        },
        skills: String::new(),
        hook_context: String::new(),
        memory: String::new(),
    };
    runner.ctx.session_record_id = session_record_id.clone();
    runner.lite_model = run.lite_model.clone();
    runner.ctx.hook_agent = Some(hook_agent_handler(&run));
    runner.live_model = Some(live_model.clone());
    if let Ok(pool) = sqlite_pool(&app).await {
        let recovery = Arc::new(crate::native::recovery::RecoveryState::new(
            pool,
            session_record_id.clone(),
            run.client.attempt_limit(),
        ));
        run.client = run.client.clone().with_recovery(recovery.clone());
        runner.recovery = Some(recovery);
    } else {
        eprintln!("[native] 无法打开数据库，工具恢复账本未启用");
    }
    let execution_target = if runner.ctx.ssh.is_some() {
        Some(crate::app::shared::EXECUTION_TARGET_SSH.to_string())
    } else {
        Some(crate::app::shared::EXECUTION_TARGET_LOCAL.to_string())
    };
    publish_live_run(
        &live_model,
        &mut runner,
        &run,
        &app,
        execution_target.clone(),
    );
    if let Ok(pool) = sqlite_pool(&app).await {
        let goal_app = app.clone();
        let goal_session = session_record_id.clone();
        let goal_profile = profile_id.clone();
        let goal_workspace = workspace_id.clone();
        let goal_kind = kind.clone();
        runner.ctx.session_scope = Some(crate::native::tools::dispatch::SessionScope {
            pool,
            workspace_id: Some(workspace_id.clone()),
            channel_id: run.channel_id.clone(),
            model: run.model.clone(),
            on_goal: Some(Arc::new(move |line| {
                let app = goal_app.clone();
                let session_record_id = goal_session.clone();
                let profile_id = goal_profile.clone();
                let workspace_id = goal_workspace.clone();
                let kind = goal_kind.clone();
                tauri::async_runtime::spawn(async move {
                    emit_native_line(
                        &app,
                        &session_record_id,
                        &profile_id,
                        Some(&workspace_id),
                        &kind,
                        line,
                    )
                    .await;
                });
            })),
        });
    }
    attach_skills_and_hooks(&app, &mut runner, &mut parts).await;
    let memory_settings = crate::native::settings::load_native_settings(&app).ok();
    let memory_dir = attach_memory(&app, &mut runner, &mut parts, memory_settings.as_ref());
    attach_subagent_runtime(
        &app,
        &mut runner,
        &parts,
        Some(&workspace_id),
        run.bound_subagent.as_ref(),
    );
    if !plan_mode {
        apply_bound_subagent(&mut runner, &mut parts, run.bound_subagent.as_ref());
    }
    let system = crate::native::prompt::compose_system(&parts);
    runner
        .messages
        .push(crate::native::model::types::Message::system(system));
    if let Some(resume_id) = resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Ok(pool) = sqlite_pool(&app).await {
            match load_transcript(&pool, resume_id).await {
                Ok(Some(mut history)) => {
                    if let Ok(dir) = app.path().app_config_dir() {
                        let _ = crate::native::images::hydrate_message_media(
                            &pool,
                            &dir.join(crate::native::images::ATTACHMENTS_DIR_NAME),
                            &mut history,
                        )
                        .await;
                    }
                    runner.messages.extend(history);
                    // 恢复到更小窗口的模型（或历史本就很长）时，第一次调用前按 downshift 压缩。
                    if runner.should_compact_context() {
                        runner.request_downshift_compaction();
                    }
                }
                Ok(None) => {
                    eprintln!("[native] 未找到可恢复的上下文，已按新对话开始");
                }
                Err(error) => {
                    eprintln!("[native] 恢复上下文失败：{error}");
                }
            }
        }
    }
    let last_transcript_fingerprint = Arc::new(Mutex::new(None));
    attach_transcript_checkpoint(
        &mut runner,
        app.clone(),
        session_record_id.clone(),
        profile_id.clone(),
        workspace_id.clone(),
        transcript_model.clone(),
        last_transcript_fingerprint.clone(),
    );
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    runner.on_event = Some(event_tx);
    let (usage_tx, mut usage_rx) = mpsc::unbounded_channel();
    runner.on_usage = Some(usage_tx);
    if announce_startup {
        emit_native_line(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            native_startup_banner(
                &run.channel_name,
                &run.protocol,
                &run.model,
                run.effort.as_deref(),
                run.thinking_enabled,
            ),
        )
        .await;
        if plan_mode {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                "[PLAN] 已进入计划模式：等待批准后实施；写入或高风险 Bash 命令须授权".to_string(),
            )
            .await;
        }
    }
    let mcp_workspace_root = runner
        .ctx
        .ssh
        .is_none()
        .then(|| runner.ctx.workspace.root.clone());
    match resolve_session_mcp_servers(&app, Some(&workspace_id), mcp_workspace_root.as_deref()) {
        Ok(servers) => {
            if announce_startup {
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    if servers.is_empty() {
                        "[MCP] 未启用服务器".to_string()
                    } else {
                        format!("[MCP] 将连接 {} 个已启用服务器", servers.len())
                    },
                )
                .await;
            }
            let ssh_config = runner.ctx.ssh.as_ref().map(|item| item.config.clone());
            if announce_startup && ssh_config.is_some() {
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    "[MCP] SSH 会话将在远端拉起 MCP，失败不回退本机".to_string(),
                )
                .await;
            }
            let connected =
                connect_mcp_servers(&app, &servers, ssh_config.as_ref(), &runner.ctx.cancel).await;
            for warning in connected.warnings {
                if announce_startup || is_mcp_error_status(&warning) {
                    emit_native_line(
                        &app,
                        &session_record_id,
                        &profile_id,
                        Some(&workspace_id),
                        &kind,
                        warning,
                    )
                    .await;
                }
            }
            if connected.connected.is_empty() {
                if !servers.is_empty() {
                    emit_native_line(
                        &app,
                        &session_record_id,
                        &profile_id,
                        Some(&workspace_id),
                        &kind,
                        "[MCP] 没有成功连接的服务器".to_string(),
                    )
                    .await;
                }
            } else if announce_startup {
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    format!("[MCP] 已连接：{}", connected.connected.join("、")),
                )
                .await;
            }
            runner.set_extra_tools(connected.session.tool_specs());
            runner.set_extra_tool_contracts(connected.session.tool_contracts());
            runner.ctx.mcp = SharedMcp::from_session(connected.session);
        }
        Err(error) => {
            emit_native_line(
                &app,
                &session_record_id,
                &profile_id,
                Some(&workspace_id),
                &kind,
                format!("[MCP] 读取配置失败：{error}"),
            )
            .await;
        }
    }
    let emit_app = app.clone();
    let emit_session = session_record_id.clone();
    let emit_profile = profile_id.clone();
    let emit_workspace = Some(workspace_id.clone());
    let emit_kind = kind.clone();
    let emit_instance = input_queue.id.clone();
    let emit_join = tokio::spawn(async move {
        forward_native_events(
            emit_app,
            emit_instance,
            emit_session,
            emit_profile,
            emit_workspace,
            emit_kind,
            event_rx,
        )
        .await;
    });
    let usage_app = app.clone();
    let usage_session = session_record_id.clone();
    let usage_join = tokio::spawn(async move {
        while let Some(delta) = usage_rx.recv().await {
            if let Ok(pool) = sqlite_pool(&usage_app).await {
                let _ = apply_session_usage(&pool, &usage_session, &delta).await;
            }
        }
    });

    let mut loaded_images = crate::native::images::load_native_images(image_paths.as_deref());
    if let (Ok(pool), Ok(dir)) = (sqlite_pool(&app).await, app.path().app_config_dir()) {
        let _ = crate::native::images::remember_loaded_media(
            &pool,
            &dir.join(crate::native::images::ATTACHMENTS_DIR_NAME),
            &mut loaded_images,
        )
        .await;
    }
    crate::native::images::cleanup_staged_loaded_images(&loaded_images);
    for line in crate::native::images::image_log_lines(&loaded_images) {
        emit_native_line(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            line,
        )
        .await;
    }
    let mut pending_images = loaded_images.images;

    let mut next = Some(first_prompt);
    let mut last_error: Option<String> = None;
    if let Err(error) = runner.apply_tool_recovery().await {
        if let Some(tx) = &runner.on_event {
            let _ = tx.send(NativeEvent::Line(format!("[ERROR] {error}")));
        }
        last_error = Some(error);
        next = None;
    }
    let await_followups = true;
    while let Some(prompt) = next.take() {
        if cancel.is_cancelled() {
            break;
        }
        runner.ctx.main_origin = Some(input_queue.steer.begin_turn().await);
        let _ = runner.ctx.restore_isolation_worktree();
        emit_turn_state(
            &app,
            &input_queue.steer,
            &working,
            "working",
            runner.on_event.as_ref(),
        )
        .await;
        let images = std::mem::take(&mut pending_images);
        emit_native_output(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            format!("[USER_INPUT] {prompt}"),
            None,
            native_images_for_output(&images),
            None,
        )
        .await;
        // 每回合按关键词回忆相关记忆，附在用户消息后。
        if let Some(dir) = memory_dir.as_deref() {
            let hits = crate::native::memory::recall(dir, &prompt, 3);
            if !hits.is_empty() {
                runner.set_turn_suffix(crate::native::memory::format_recall_block(dir, &hits));
            }
        }
        let prompt = if let Ok(pool) = sqlite_pool(&app).await {
            match plans::reference(&pool, &session_record_id).await {
                Ok(Some(reference)) => format!("{prompt}{reference}"),
                _ => prompt,
            }
        } else {
            prompt
        };
        match runner
            .run_with_client(
                &run.client,
                &prompt,
                &run.model,
                run.effort.as_deref(),
                run.max_output_tokens,
                run.thinking_enabled,
                images,
            )
            .await
        {
            Ok(_) => {
                if let Some(recovery) = &runner.recovery {
                    if let Err(error) = recovery.complete_turn().await {
                        last_error = Some(error.clone());
                        if let Some(tx) = &runner.on_event {
                            let _ = tx.send(NativeEvent::Line(format!("[ERROR] {error}")));
                        }
                        let _ = next_loop_step(await_followups, NativeLoopEvent::Error);
                        break;
                    }
                }
            }
            Err(error) => {
                last_error = Some(error.clone());
                if !is_cancelled_run_error(&error) {
                    if let Some(tx) = &runner.on_event {
                        let _ = tx.send(NativeEvent::Line(format!("[ERROR] {error}")));
                    } else {
                        emit_native_line(
                            &app,
                            &session_record_id,
                            &profile_id,
                            Some(&workspace_id),
                            &kind,
                            format!("[ERROR] {error}"),
                        )
                        .await;
                    }
                }
                let _ = next_loop_step(await_followups, NativeLoopEvent::Error);
                break;
            }
        };
        sync_run_from_live(&mut run, &live_model);
        *transcript_model.lock().await = run.model.clone();
        // Drain the completed answer before a queued input creates the next UI turn.
        if let Some(events) = &runner.on_event {
            let (reply, done) = tokio::sync::oneshot::channel();
            if events.send(NativeEvent::Flush(reply)).is_ok() {
                let _ = done.await;
            }
        }
        persist_runner_transcript(
            &app,
            &session_record_id,
            &profile_id,
            &workspace_id,
            &run.model,
            &runner.messages,
            last_transcript_fingerprint.as_ref(),
        )
        .await;
        match next_loop_step(await_followups, NativeLoopEvent::TurnFinished) {
            NativeLoopAction::WaitFollowup => {
                if cancel.is_cancelled() {
                    let _ = next_loop_step(await_followups, NativeLoopEvent::Cancelled);
                    break;
                }
                if runner.take_steer_finish() {
                    let _ = next_loop_step(await_followups, NativeLoopEvent::FollowupFinish);
                    break;
                }
                announce_isolation_restore(
                    &app,
                    &session_record_id,
                    &profile_id,
                    &workspace_id,
                    &kind,
                    runner.ctx.restore_isolation_worktree(),
                )
                .await;
                emit_turn_state(
                    &app,
                    &input_queue.steer,
                    &working,
                    "waiting_input",
                    runner.on_event.as_ref(),
                )
                .await;
                // 等待输入时先应用模型配置，再处理 /compact 与输入队列。
                let followup = loop {
                    if let Some(request) = take_latest_configuration(&mut config_rx, None) {
                        dispatch_session_configuration(
                            &app,
                            &manager_state,
                            &mut run,
                            &mut runner,
                            request,
                            &session_record_id,
                            &workspace_id,
                            &transcript_model,
                            &last_transcript_fingerprint,
                            &profile_id,
                            &input_queue.id,
                            &mut config_revision,
                            &live_model,
                        )
                        .await;
                        continue;
                    }
                    let mut controls = followup_rx.lock().await;
                    let wait = recv_idle_wait(
                        &mut controls,
                        &mut config_rx,
                        &input_queue,
                        &cancel,
                        &working,
                    )
                    .await;
                    drop(controls);
                    match wait {
                        NativeIdleWait::Configuration(request) => {
                            let Some(request) = take_latest_configuration(&mut config_rx, request)
                            else {
                                break None;
                            };
                            dispatch_session_configuration(
                                &app,
                                &manager_state,
                                &mut run,
                                &mut runner,
                                request,
                                &session_record_id,
                                &workspace_id,
                                &transcript_model,
                                &last_transcript_fingerprint,
                                &profile_id,
                                &input_queue.id,
                                &mut config_revision,
                                &live_model,
                            )
                            .await;
                        }
                        NativeIdleWait::Followup(Some(NativeFollowup::Compact(mut request))) => {
                            emit_turn_state(
                                &app,
                                &input_queue.steer,
                                &working,
                                "working",
                                runner.on_event.as_ref(),
                            )
                            .await;
                            match runner
                                .compact_now(&run.client, request.instructions.take())
                                .await
                            {
                                Ok(None) => {
                                    emit_native_line(
                                        &app,
                                        &session_record_id,
                                        &profile_id,
                                        Some(&workspace_id),
                                        &kind,
                                        "[工具] 当前上下文太短，无需压缩".to_string(),
                                    )
                                    .await;
                                }
                                Ok(Some(_)) => {}
                                Err(error) => {
                                    emit_native_line(
                                        &app,
                                        &session_record_id,
                                        &profile_id,
                                        Some(&workspace_id),
                                        &kind,
                                        format!("[ERROR] {error}"),
                                    )
                                    .await;
                                    break None;
                                }
                            }
                            persist_runner_transcript(
                                &app,
                                &session_record_id,
                                &profile_id,
                                &workspace_id,
                                &run.model,
                                &runner.messages,
                                last_transcript_fingerprint.as_ref(),
                            )
                            .await;
                            emit_turn_state(
                                &app,
                                &input_queue.steer,
                                &working,
                                "waiting_input",
                                runner.on_event.as_ref(),
                            )
                            .await;
                        }
                        NativeIdleWait::Followup(other) => break other,
                        NativeIdleWait::Input(input) => {
                            break input.map(|item| {
                                NativeFollowup::input_with_images(item.text, item.images)
                            });
                        }
                    }
                };
                match followup {
                    Some(NativeFollowup::Input { text, images }) => {
                        match next_loop_step(await_followups, NativeLoopEvent::FollowupInput) {
                            NativeLoopAction::RunFollowup => {
                                pending_images = images;
                                next = Some(text);
                            }
                            _ => break,
                        }
                    }
                    Some(NativeFollowup::Finish) | Some(NativeFollowup::Compact(_)) | None => {
                        let _ = next_loop_step(await_followups, NativeLoopEvent::FollowupFinish);
                        break;
                    }
                }
            }
            NativeLoopAction::Exit | NativeLoopAction::RunFollowup => break,
        }
    }

    if let Err(error) = input_queue
        .steer
        .cancel("回合已停止；未应用的指令未自动重放，图片需要重新选择")
        .await
    {
        eprintln!("[native] {error}");
    }
    input_queue.close();
    persist_runner_transcript(
        &app,
        &session_record_id,
        &profile_id,
        &workspace_id,
        &run.model,
        &runner.messages,
        last_transcript_fingerprint.as_ref(),
    )
    .await;

    let memory_finished = tokio::select! {
        biased;
        _ = crate::app::lifecycle::fast_restart_requested(&app) => true,
        result = tokio::time::timeout(
            Duration::from_secs(20),
            finish_memory(
                &app,
                &run,
                &runner,
                memory_dir.as_deref(),
                memory_settings.as_ref(),
                &session_record_id,
                &profile_id,
                &workspace_id,
                &kind,
                cancel.is_cancelled(),
            ),
        ) => result.is_ok(),
    };
    if !memory_finished {
        eprintln!("[native] 会话结束记忆处理超时，保留已有记录");
    }

    // 会话结束时停掉仍在跑的后台子 Agent。
    runner.background.stop_all();
    runner.on_event.take();
    runner.on_usage.take();
    runner.ctx.mcp.shutdown().await;
    let _ = emit_join.await;
    let _ = usage_join.await;

    let budget = runner.budget_snapshot();
    let context = runner.context_window;
    let diagnostics = format_native_diagnostics(&budget, &context, &runner.diagnostics_snapshot());
    if let Ok(pool) = sqlite_pool(&app).await {
        let _ = insert_session_event(
            &pool,
            &session_record_id,
            "native_token_diagnostics",
            Some(&diagnostics),
        )
        .await;
    }

    let failed = last_error
        .as_deref()
        .is_some_and(|error| !is_cancelled_run_error(error));
    let status = if failed { "failed" } else { "exited" };
    let code = if failed { 1 } else { 0 };
    let ended_at = now_sqlite();
    let mut notification_body = session_record_id.chars().take(8).collect::<String>();
    if let Ok(pool) = sqlite_pool(&app).await {
        let _ = update_agent_session_status(
            &pool,
            &session_record_id,
            status,
            Some(code),
            Some(ended_at.as_str()),
        )
        .await;
        if let Ok(Some(title)) = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(NULLIF(TRIM(title), ''), '') FROM agent_sessions WHERE id = $1",
        )
        .bind(&session_record_id)
        .fetch_optional(&pool)
        .await
        {
            if !title.is_empty() {
                notification_body = title;
            }
        }
    }
    crate::app::notifications::notify_if_unfocused(
        &app,
        if failed {
            "会话失败"
        } else {
            "会话已结束"
        },
        &notification_body,
    );
    let worktree_path = runner
        .ctx
        .restore_isolation_worktree()
        .map(|item| item.path)
        .or_else(|| {
            runner
                .ctx
                .worktree_path
                .read()
                .ok()
                .and_then(|slot| slot.clone())
        });
    let _ = app.emit(
        "native-exit",
        AgentSessionExit {
            instance_id: input_queue.id.clone(),
            profile_id: profile_id.clone(),
            workspace_id: Some(workspace_id),
            session_kind: kind,
            session_record_id: session_record_id.clone(),
            code,
            worktree_path,
        },
    );
    if let Some(lsp) = runner.ctx.lsp.take() {
        lsp.shutdown().await;
    }
    if let Some(processes) = runner.ctx.processes.as_ref() {
        processes.stop_all();
    }
    manager_state
        .lock()
        .await
        .remove_session(&session_record_id);
}

pub(super) async fn stop_native_process(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    session_record_id: &str,
    event_type: &str,
    message: &str,
) -> Result<bool, String> {
    manager_state
        .lock()
        .await
        .invalidate_plan_authorization(session_record_id);
    plans::invalidate_persisted(&sqlite_pool(app).await?, session_record_id, false).await?;
    let info = {
        let manager = manager_state.lock().await;
        manager
            .get_session(session_record_id)
            .map(|item| item.info.clone())
    };
    let Some(info) = info else {
        return Ok(false);
    };

    let pool = sqlite_pool(app).await?;
    update_agent_session_status(&pool, session_record_id, "stopping", None, None).await?;
    insert_session_event(&pool, session_record_id, event_type, Some(message)).await?;
    emit_native_line(
        app,
        session_record_id,
        &info.profile_id,
        info.workspace_id.as_deref(),
        &info.session_kind,
        format!("[内置 Agent] {message}"),
    )
    .await;

    let session = {
        let mut manager = manager_state.lock().await;
        manager.deny_pending_permission(session_record_id);
        if let Some(session) = manager.get_session(session_record_id) {
            session.cancel.cancel();
            session
                .input_queue
                .steer
                .cancel("会话已停止；未应用的指令未自动重放，图片需要重新选择")
                .await?;
        }
        manager.remove_session(session_record_id)
    };
    let Some(session) = session else {
        return Ok(true);
    };
    if let Some(processes) = session.processes.as_ref() {
        processes.stop_all();
    }
    session.cancel.cancel();
    session.input_queue.close();
    let _ = session.followup_tx.send(NativeFollowup::Finish).await;
    let _ = session.join.await;
    Ok(true)
}
