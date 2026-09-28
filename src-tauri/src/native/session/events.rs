#[allow(unused_imports)]
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeLoopEvent {
    TurnFinished,
    FollowupInput,
    FollowupFinish,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeLoopAction {
    WaitFollowup,
    RunFollowup,
    Exit,
}

pub(crate) fn next_loop_step(await_followups: bool, event: NativeLoopEvent) -> NativeLoopAction {
    match event {
        NativeLoopEvent::Cancelled | NativeLoopEvent::Error => NativeLoopAction::Exit,
        NativeLoopEvent::TurnFinished if await_followups => NativeLoopAction::WaitFollowup,
        NativeLoopEvent::TurnFinished => NativeLoopAction::Exit,
        NativeLoopEvent::FollowupInput => NativeLoopAction::RunFollowup,
        NativeLoopEvent::FollowupFinish => NativeLoopAction::Exit,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn persist_native_transcript(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: Option<&str>,
    model: &str,
    turns: u32,
    messages: &[crate::native::model::types::Message],
    last_fingerprint: &mut Option<u64>,
) {
    let fingerprint = transcript_fingerprint(messages);
    if last_fingerprint.as_ref() == Some(&fingerprint) {
        return;
    }
    let Ok(pool) = sqlite_pool(app).await else {
        eprintln!("[native] 保存会话上下文失败: 无法打开数据库");
        return;
    };
    let meta = NativeTranscriptMeta {
        profile_id: Some(profile_id.to_string()),
        workspace_id: workspace_id.map(ToOwned::to_owned),
        model: model.to_string(),
        turns,
    };
    match save_transcript(&pool, session_record_id, messages, &meta).await {
        Ok(()) => *last_fingerprint = Some(fingerprint),
        Err(error) => eprintln!("[native] 保存会话上下文失败: {error}"),
    }
}

pub(super) async fn persist_runner_transcript(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: &str,
    model: &str,
    messages: &[crate::native::model::types::Message],
    last_fingerprint: &Mutex<Option<u64>>,
) {
    let mut last = last_fingerprint.lock().await;
    persist_native_transcript(
        app,
        session_record_id,
        profile_id,
        Some(workspace_id),
        model,
        user_turn_count(messages),
        messages,
        &mut last,
    )
    .await;
}

pub(super) fn attach_transcript_checkpoint(
    runner: &mut AgentRunner,
    app: AppHandle,
    session_record_id: String,
    profile_id: String,
    workspace_id: String,
    model: Arc<Mutex<String>>,
    last_fingerprint: Arc<Mutex<Option<u64>>>,
) {
    let hook: TranscriptCheckpoint = Arc::new(move |messages| {
        let app = app.clone();
        let session_record_id = session_record_id.clone();
        let profile_id = profile_id.clone();
        let workspace_id = workspace_id.clone();
        let model = model.clone();
        let last_fingerprint = last_fingerprint.clone();
        Box::pin(async move {
            let model = model.lock().await.clone();
            let fingerprint = transcript_fingerprint(&messages);
            let mut last = last_fingerprint.lock().await;
            if last.as_ref() == Some(&fingerprint) {
                return Ok(messages);
            }
            let pool = sqlite_pool(&app)
                .await
                .map_err(|error| format!("保存会话历史失败: {error}"))?;
            let turns = user_turn_count(&messages);
            let mut owned = messages;
            let receipt = commit_model_context(
                &pool,
                HistoryWrite {
                    session_record_id: &session_record_id,
                    profile_id: Some(profile_id.as_str()),
                    workspace_id: Some(workspace_id.as_str()),
                    model: &model,
                    turns,
                    messages: &mut owned,
                    turn_id: None,
                    attempt_id: None,
                    expected_revision: None,
                    request_id: None,
                    links: &[],
                    legacy_baseline: false,
                },
            )
            .await?;
            if receipt.changed {
                let _ = app.emit("native-history-committed", &receipt);
            }
            *last = Some(transcript_fingerprint(&owned));
            Ok(owned)
        })
    });
    runner.on_checkpoint = Some(hook);
}

pub(super) fn user_turn_count(messages: &[crate::native::model::types::Message]) -> u32 {
    messages
        .iter()
        .filter(|message| message.role == crate::native::model::types::Role::User)
        .count() as u32
}

#[derive(Clone, Serialize)]
pub(super) struct NativePermissionRequestEvent {
    pub(super) instance_id: Option<String>,
    pub(super) session_record_id: String,
    pub(super) request_id: String,
    pub(super) profile_id: String,
    pub(super) workspace_id: Option<String>,
    pub(super) session_kind: String,
    pub(super) tool_name: String,
    pub(super) kind: NativeToolRiskKind,
    pub(super) summary: String,
    pub(super) remote: bool,
    pub(super) mcp_server_id: Option<String>,
    pub(super) suggested_rule: Option<PermissionRuleSuggestion>,
    pub(super) file_access: Option<crate::native::tools::file_access::FileAccessPrompt>,
    pub(super) allow_once_only: bool,
}

pub(super) fn session_kind(plan_mode: bool) -> String {
    if plan_mode {
        "plan".to_string()
    } else {
        "execution".to_string()
    }
}

pub(super) fn permission_event(
    session_record_id: &str,
    request: &PermissionRequest,
) -> NativePermissionRequestEvent {
    NativePermissionRequestEvent {
        instance_id: request
            .origin
            .as_ref()
            .map(|origin| origin.instance_id.clone()),
        session_record_id: session_record_id.to_string(),
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        workspace_id: request.workspace_id.clone(),
        session_kind: request.session_kind.clone(),
        tool_name: request.tool_name.clone(),
        kind: request.kind,
        summary: request.summary.clone(),
        remote: request.remote,
        mcp_server_id: request.mcp_server_id.clone(),
        suggested_rule: request.suggested_rule.clone(),
        file_access: request.file_access.clone(),
        allow_once_only: request.allow_once_only,
    }
}

#[derive(Clone, Serialize)]
pub(super) struct NativePlanQuestionEvent {
    pub(super) instance_id: Option<String>,
    pub(super) session_record_id: String,
    pub(super) request_id: String,
    pub(super) profile_id: String,
    pub(super) workspace_id: Option<String>,
    pub(super) session_kind: String,
    pub(super) questions: Vec<crate::native::tools::question::PlanQuestion>,
}

pub(super) fn question_event(
    session_record_id: &str,
    request: &PlanQuestionRequest,
) -> NativePlanQuestionEvent {
    NativePlanQuestionEvent {
        instance_id: request
            .origin
            .as_ref()
            .map(|origin| origin.instance_id.clone()),
        session_record_id: session_record_id.to_string(),
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        workspace_id: request.workspace_id.clone(),
        session_kind: request.session_kind.clone(),
        questions: request.questions.clone(),
    }
}

#[derive(Clone, Serialize)]
pub(super) struct NativePlanApprovalEvent {
    pub(super) instance_id: Option<String>,
    pub(super) session_record_id: String,
    pub(super) request_id: String,
    pub(super) profile_id: String,
    pub(super) workspace_id: Option<String>,
    pub(super) session_kind: String,
    pub(super) plan: String,
}

pub(super) fn plan_approval_event(
    session_record_id: &str,
    request: &PlanApprovalRequest,
) -> NativePlanApprovalEvent {
    NativePlanApprovalEvent {
        instance_id: request
            .origin
            .as_ref()
            .map(|origin| origin.instance_id.clone()),
        session_record_id: session_record_id.to_string(),
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        workspace_id: request.workspace_id.clone(),
        session_kind: request.session_kind.clone(),
        plan: request.plan.clone(),
    }
}

pub(super) fn apply_bound_subagent(
    runner: &mut AgentRunner,
    parts: &mut crate::native::prompt::NativePromptParts,
    bound: Option<&crate::native::subagents::NativeSubagent>,
) {
    let Some(def) = bound else {
        return;
    };
    parts.required_subagent_name = def.name.clone();
    parts.required_subagent_description = def.description.clone();
    runner.required_subagent_type = Some(def.name.clone());
}

/// `agent` 类型钩子：用当前会话的模型做一次无工具的判定，只要求输出 JSON。
pub(super) fn hook_agent_handler(
    run: &NativeRunSettings,
) -> crate::native::tools::hooks::HookAgentHandler {
    use crate::native::model::call_log::{MODEL_ROLE_LITE, MODEL_ROLE_MAIN, OPERATION_HOOK_AGENT};
    let (model, role) = match run.lite_model.as_deref() {
        Some(lite) if !lite.trim().is_empty() => (lite.to_string(), MODEL_ROLE_LITE),
        _ => (run.model.clone(), MODEL_ROLE_MAIN),
    };
    let client = match run.client.call_log_context() {
        Some(context) => run.client.clone().with_call_log_context(
            context
                .clone()
                .with_operation(OPERATION_HOOK_AGENT)
                .with_model_role(role),
        ),
        None => run.client.clone(),
    };
    let effort = if role == MODEL_ROLE_LITE {
        None
    } else {
        run.effort.clone()
    };
    Arc::new(move |prompt, payload| {
        let client = client.clone();
        let model = model.clone();
        let effort = effort.clone();
        Box::pin(async move {
            let messages = vec![
                crate::native::model::types::Message::system(
                    "你是 noxcode 的钩子判定器。根据判定要求与事件载荷，只输出一个 JSON 对象：{\"decision\":\"allow|deny|ask\",\"continue\":true|false,\"reason\":\"简短理由\"}。不要输出其他内容。",
                ),
                crate::native::model::types::Message::user(format!(
                    "判定要求：\n{prompt}\n\n事件载荷：\n{payload}"
                )),
            ];
            let message = client
                .chat(crate::native::model::client::ChatRequest {
                    messages: &messages,
                    tools: &[],
                    model: &model,
                    effort: effort.as_deref(),
                    max_output_tokens: Some(512),
                    thinking_enabled: false,
                })
                .await?
                .complete_message()?;
            Ok(message.content)
        })
    })
}

/// 本地工作区且开启记忆时：把记忆目录加入可读写根、索引块注入系统提示。返回记忆目录。
pub(super) fn attach_memory(
    app: &AppHandle,
    runner: &mut AgentRunner,
    parts: &mut crate::native::prompt::NativePromptParts,
    settings: Option<&crate::db::models::NativeSettings>,
) -> Option<PathBuf> {
    if !settings.is_some_and(|item| item.memory_enabled) || runner.ctx.ssh.is_some() {
        return None;
    }
    let config_dir = app.path().app_config_dir().ok()?;
    let dir = crate::native::memory::memory_dir(
        &config_dir,
        &runner.ctx.workspace.root.to_string_lossy(),
    );
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("[native] 创建记忆目录失败: {error}");
        return None;
    }
    if !dir.join(crate::native::memory::MEMORY_INDEX_FILE).exists() {
        let _ = crate::native::memory::rebuild_index(&dir);
    }
    parts.memory = crate::native::memory::memory_prompt_block(&dir);
    runner.ctx.workspace.extra_write_roots.push(dir.clone());
    Some(dir)
}

/// 会话结束后的记忆抽取与周期性 dream；失败只打日志。
#[allow(clippy::too_many_arguments)]
pub(super) async fn finish_memory(
    app: &AppHandle,
    run: &NativeRunSettings,
    runner: &AgentRunner,
    memory_dir: Option<&Path>,
    settings: Option<&crate::db::models::NativeSettings>,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: &str,
    kind: &str,
    cancelled: bool,
) {
    let Some(dir) = memory_dir else {
        return;
    };
    if cancelled {
        return;
    }
    // 至少有一轮完整对话才值得抽取。
    let exchanges = runner
        .messages
        .iter()
        .filter(|message| {
            matches!(
                message.role,
                crate::native::model::types::Role::User
                    | crate::native::model::types::Role::Assistant
            )
        })
        .count();
    if exchanges < 2 {
        return;
    }
    match crate::native::memory::extract_memories(
        &run.client,
        &run.model,
        run.lite_model.as_deref(),
        &runner.messages,
        dir,
    )
    .await
    {
        Ok(0) => {}
        Ok(count) => {
            emit_native_line(
                app,
                session_record_id,
                profile_id,
                Some(workspace_id),
                kind,
                format!("[记忆] 已保存 {count} 条记忆到 {}", dir.display()),
            )
            .await;
        }
        Err(error) => eprintln!("[native] 记忆抽取失败: {error}"),
    }
    let interval = settings
        .map(|item| item.memory_dream_interval.max(0) as u32)
        .unwrap_or(0);
    if crate::native::memory::dream_due(dir, interval) {
        match crate::native::memory::dream(&run.client, &run.model, run.lite_model.as_deref(), dir)
            .await
        {
            Ok(summary) => {
                emit_native_line(
                    app,
                    session_record_id,
                    profile_id,
                    Some(workspace_id),
                    kind,
                    format!("[记忆] {summary}"),
                )
                .await;
            }
            Err(error) => eprintln!("[native] 记忆整理失败: {error}"),
        }
    }
}

pub(super) async fn attach_skills_and_hooks(
    app: &AppHandle,
    runner: &mut AgentRunner,
    parts: &mut crate::native::prompt::NativePromptParts,
) {
    let global_hooks = crate::native::settings::load_native_settings(app)
        .map(|settings| settings.hooks)
        .unwrap_or_default();
    let config_dir = app.path().app_config_dir().ok();
    let workspace_root = runner
        .ctx
        .ssh
        .is_none()
        .then(|| runner.ctx.workspace.root.clone());
    // 已启用插件贡献的钩子与技能。
    let plugins = crate::native::plugins::load_enabled_plugins(
        config_dir.as_deref(),
        workspace_root.as_deref(),
    );
    let (workspace_plugins, global_plugins): (Vec<_>, Vec<_>) = plugins
        .iter()
        .cloned()
        .partition(|plugin| plugin.source == crate::native::plugins::PluginSource::Workspace);
    let plugin_hooks = crate::native::plugins::plugin_hooks(&global_plugins);
    // 本地工作区再叠加 .noxcode/hooks.json 与 .claude/settings.json 里的钩子。
    let mut workspace_hooks = if runner.ctx.ssh.is_none() {
        crate::native::hooks_config::load_workspace_hooks(&runner.ctx.workspace.root)
    } else {
        Vec::new()
    };
    workspace_hooks.extend(crate::native::plugins::plugin_hooks(&workspace_plugins));
    if !workspace_hooks.is_empty() && !approve_workspace_hooks(&runner.ctx, &workspace_hooks).await
    {
        workspace_hooks.clear();
    }
    runner.ctx.hooks = crate::native::hooks_config::merge_hooks(
        crate::native::hooks_config::merge_hooks(global_hooks, plugin_hooks),
        workspace_hooks,
    );
    // session_start 钩子的输出进入系统提示尾段。
    if !runner.ctx.hooks.is_empty() {
        let context =
            crate::native::tools::hooks::run_session_start_hooks(&runner.ctx.hook_runtime()).await;
        if !context.is_empty() {
            parts.hook_context = context.join("\n");
        }
    }
    let skills = crate::native::skills::load_session_skills(
        &parts.cwd,
        runner.ctx.ssh.as_ref(),
        config_dir.as_deref(),
        &plugins,
    )
    .await;
    parts.skills = crate::native::skills::format_skills_prompt(&skills);
    runner.skills_prompt = parts.skills.clone();
    runner.ctx.skills = skills;
}

// 仓库内的命令配置不等于用户授权；不经过工具自动批准或 permission_request hooks。
// 完全访问（yolo）视为已信任本会话工作区钩子，不再弹 WorkspaceHooks。
pub(super) async fn approve_workspace_hooks(
    ctx: &crate::native::tools::dispatch::ToolCtx,
    hooks: &[crate::db::models::NativeHook],
) -> bool {
    if ctx.cancel.is_cancelled() {
        return false;
    }
    if ctx
        .allow_all_high_risk
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return true;
    }
    let Some(requester) = &ctx.request_permission else {
        return false;
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    requester(
        crate::native::tools::dispatch::PermissionPrompt {
            origin: ctx.request_origin(),
            request_id: request_id.clone(),
            tool_name: "WorkspaceHooks".to_string(),
            kind: NativeToolRiskKind::Opaque,
            summary: format!(
                "工作区 {} 提供了 {} 个自动执行钩子。仅在信任仓库内容时批准本次会话：\n{}",
                ctx.workspace.root.display(),
                hooks.len(),
                serde_json::to_string_pretty(hooks).unwrap_or_default()
            ),
            remote: false,
            mcp_server_id: None,
            suggested_rule: None,
            file_access: None,
            allow_once_only: false,
        },
        tx,
    );
    let timeout = if ctx.permission_timeout.is_zero() {
        Duration::from_secs(120)
    } else {
        ctx.permission_timeout
    };
    let decision = tokio::select! {
        result = rx => result.ok(),
        _ = tokio::time::sleep(timeout) => None,
        _ = async { while !ctx.cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }} => None,
    };
    if decision.is_none() {
        if let Some(expire) = &ctx.expire_permission {
            let _ = expire(request_id).await;
        }
    }
    ctx.execution_current()
        && matches!(
            decision,
            Some(NativePermissionDecision::AllowOnce | NativePermissionDecision::AllowSession)
        )
}

pub(super) fn attach_subagent_runtime(
    app: &AppHandle,
    runner: &mut AgentRunner,
    parts: &crate::native::prompt::NativePromptParts,
    workspace_id: Option<&str>,
    bound: Option<&crate::native::subagents::NativeSubagent>,
) {
    runner.workspace_context = crate::native::prompt::workspace_context_block(parts);
    runner.project_agents = parts.project_agents.clone();
    // 本地工作区还会读取 .noxcode/agents、.claude/agents 与全局 agents 目录下的 .md 档案。
    let workspace_root = runner
        .ctx
        .ssh
        .is_none()
        .then(|| runner.ctx.workspace.root.clone());
    let loaded = crate::native::subagents::load_session_subagents(app, workspace_root.as_deref());
    runner.custom_subagents =
        crate::native::subagents::catalog_for_session(&loaded, workspace_id, bound);
    let app_reload = app.clone();
    let workspace_id_owned = workspace_id.map(ToOwned::to_owned);
    let bound_owned = bound.cloned();
    runner.reload_custom_subagents = Some(std::sync::Arc::new(move || {
        let loaded = crate::native::subagents::load_session_subagents(
            &app_reload,
            workspace_root.as_deref(),
        );
        crate::native::subagents::catalog_for_session(
            &loaded,
            workspace_id_owned.as_deref(),
            bound_owned.as_ref(),
        )
    }));
    let app_load = app.clone();
    runner.child_model_loader = Some(std::sync::Arc::new(move |channel_id, model, effort| {
        let app = app_load.clone();
        Box::pin(async move {
            crate::native::subagents::resolve_child_model(
                &app,
                &channel_id,
                &model,
                effort.as_deref(),
            )
            .await
        })
    }));
}

pub(super) async fn announce_isolation_restore(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: &str,
    session_kind: &str,
    restore: Option<crate::native::tools::dispatch::IsolationRestore>,
) {
    let Some(restore) = restore.filter(|item| item.switched) else {
        return;
    };
    emit_native_line(
        app,
        session_record_id,
        profile_id,
        Some(workspace_id),
        session_kind,
        format!("[WORKTREE] 已回到隔离工作树 {}", restore.path),
    )
    .await;
}

pub(super) async fn emit_turn_state(
    app: &AppHandle,
    mailbox: &crate::native::steer::SteerMailbox,
    working: &AtomicBool,
    state: &str,
    events: Option<&mpsc::UnboundedSender<NativeEvent>>,
) {
    working.store(state == "working", Ordering::SeqCst);
    if let Some(lifecycle) = mailbox.lifecycle_state(state).await {
        if let Some(events) = events {
            let _ = events.send(NativeEvent::TurnIdentity {
                instance_id: lifecycle.instance_id.clone(),
                turn_id: lifecycle.turn_id.clone(),
            });
        }
        let _ = app.emit("native-turn-state", lifecycle);
    }
}

pub(super) fn emit_plan_mode(
    app: &AppHandle,
    session_record_id: &str,
    input_queue_id: &str,
    plan_mode: bool,
) {
    let _ = app.emit(
        "native-plan-mode",
        NativePlanModeChanged {
            session_record_id: session_record_id.to_string(),
            plan_mode,
            input_queue_id: Some(input_queue_id.to_string()),
        },
    );
}

pub(super) fn extra_headers_map(raw: Option<&str>) -> HashMap<String, String> {
    let Some(text) = raw.filter(|item| !item.trim().is_empty()) else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(text).unwrap_or_default()
}

pub(super) const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(80);
pub(super) const DELTA_FLUSH_BYTES: usize = 512;
pub(super) const DELTA_SEGMENT_TEXT: &str = "text";
pub(super) const DELTA_SEGMENT_REASONING: &str = "reasoning";

pub(super) struct NativeDeltaEmitter {
    pub(super) instance_id: String,
    pub(super) turn_id: Option<String>,
    pub(super) app: AppHandle,
    pub(super) session_record_id: String,
    pub(super) pending: Option<(&'static str, String)>,
    pub(super) assistant: Option<crate::db::models::NativeAssistantFragment>,
}

impl NativeDeltaEmitter {
    fn push(&mut self, segment: &'static str, text: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(current, _)| *current != segment)
        {
            self.flush();
        }
        let (_, buffer) = self.pending.get_or_insert_with(|| (segment, String::new()));
        buffer.push_str(text);
        if buffer.len() >= DELTA_FLUSH_BYTES {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let Some((segment, text)) = self.pending.take() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.emit(segment, text, false);
    }

    fn clear(&mut self) {
        self.pending = None;
        self.emit(DELTA_SEGMENT_TEXT, String::new(), true);
    }

    fn emit(&self, segment: &str, delta: String, clear: bool) {
        let _ = self.app.emit(
            "native-text-delta",
            NativeTextDelta {
                instance_id: self.instance_id.clone(),
                turn_id: self.turn_id.clone(),
                session_record_id: self.session_record_id.clone(),
                kind: segment.to_string(),
                text: delta,
                clear,
                assistant: if segment == DELTA_SEGMENT_TEXT {
                    self.assistant.clone()
                } else {
                    None
                },
            },
        );
    }
}

pub(super) async fn forward_native_events(
    app: AppHandle,
    instance_id: String,
    session_record_id: String,
    profile_id: String,
    workspace_id: Option<String>,
    session_kind: String,
    mut event_rx: mpsc::UnboundedReceiver<NativeEvent>,
) {
    let mut deltas = NativeDeltaEmitter {
        instance_id,
        turn_id: None,
        app: app.clone(),
        session_record_id: session_record_id.clone(),
        pending: None,
        assistant: None,
    };
    let mut ticker = tokio::time::interval(DELTA_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                let Some(event) = event else {
                    deltas.flush();
                    break;
                };
                match event {
                    NativeEvent::TurnIdentity { instance_id, turn_id } => {
                        deltas.flush();
                        deltas.instance_id = instance_id;
                        deltas.turn_id = Some(turn_id);
                    }
                    NativeEvent::Flush(reply) => {
                        deltas.flush();
                        let _ = reply.send(());
                    }
                    NativeEvent::Line(line) => {
                        deltas.flush();
                        emit_native_line(
                            &app,
                            &session_record_id,
                            &profile_id,
                            workspace_id.as_deref(),
                            &session_kind,
                            line,
                        )
                        .await;
                    }
                    NativeEvent::ModelCall(fragment) => {
                        deltas.clear();
                        deltas.assistant = Some(fragment);
                    }
                    NativeEvent::Assistant { text, fragment } => {
                        deltas.flush();
                        emit_native_output(
                            &app, &session_record_id, &profile_id,
                            workspace_id.as_deref(), &session_kind,
                            text, None, None, Some(fragment),
                        ).await;
                    }
                    NativeEvent::UserInput { text, images } => {
                        deltas.flush();
                        emit_native_output(
                            &app,
                            &session_record_id,
                            &profile_id,
                            workspace_id.as_deref(),
                            &session_kind,
                            format!("[USER_INPUT] {text}"),
                            None,
                            native_images_for_output(&images),
                            None,
                        )
                        .await;
                    }
                    NativeEvent::Tool { line, event, images } => {
                        deltas.flush();
                        let live_images = if images.is_empty() {
                            None
                        } else {
                            Some(
                                images
                                    .into_iter()
                                    .map(|image| NativeToolImage {
                                        name: image.name.clone(),
                                        mime_type: image.mime_type.clone(),
                                        data_url: image.data_url(),
                                        attachment_id: (!image.attachment_id.is_empty())
                                            .then(|| image.attachment_id.clone()),
                                    })
                                    .collect(),
                            )
                        };
                        emit_native_output(
                            &app,
                            &session_record_id,
                            &profile_id,
                            workspace_id.as_deref(),
                            &session_kind,
                            line,
                            Some(event),
                            live_images,
                            None,
                        )
                        .await;
                    }
                    NativeEvent::Delta(StreamDelta::Text(text)) => {
                        deltas.push(DELTA_SEGMENT_TEXT, &text);
                    }
                    NativeEvent::Delta(StreamDelta::Reasoning(text)) => {
                        deltas.push(DELTA_SEGMENT_REASONING, &text);
                    }
                    NativeEvent::Delta(StreamDelta::Reset) => deltas.clear(),
                    NativeEvent::ContextUsage(snapshot) => {
                        let usage = NativeContextUsage {
                            session_record_id: session_record_id.clone(),
                            used_tokens: snapshot.used_tokens,
                            limit_tokens: snapshot.limit_tokens,
                            generation: snapshot.generation,
                            compactions: snapshot.compactions,
                            mcp_tokens: snapshot.mcp_tokens,
                            system_tool_tokens: snapshot.system_tool_tokens,
                            skill_tokens: snapshot.skill_tokens,
                            system_prompt_tokens: snapshot.system_prompt_tokens,
                            other_tokens: snapshot.other_tokens,
                            message_tokens: snapshot.message_tokens,
                            prompt_tokens: snapshot.prompt_tokens,
                            cached_tokens: snapshot.cached_tokens,
                        };
                        let _ = app.emit("native-context-usage", usage.clone());
                        if let Ok(pool) = sqlite_pool(&app).await {
                            if let Err(error) = persist_context_usage_with(&pool, &usage).await {
                                eprintln!("[native] 保存上下文用量失败: {error}");
                            }
                        }
                    }
                }
            }
            _ = ticker.tick() => deltas.flush(),
        }
    }
}

pub(super) async fn persist_steer_receipt(
    pool: &sqlx::SqlitePool,
    receipt: &crate::native::steer::SteerReceipt,
) -> Result<(), String> {
    let message = serde_json::to_string(receipt).map_err(|error| error.to_string())?;
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    if receipt.status == crate::native::steer::SteerStatus::Accepted {
        plans::invalidate_persisted(&mut *transaction, &receipt.session_record_id, true).await?;
    }
    insert_session_event(
        &mut *transaction,
        &receipt.session_record_id,
        "native_steer",
        Some(&message),
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())
}

pub(super) async fn insert_session_event<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    session_record_id: &str,
    event_type: &str,
    message: Option<&str>,
) -> Result<String, String> {
    let id = new_id();
    let now = now_sqlite();
    sqlx::query(
        "INSERT INTO agent_session_events (id, session_id, event_type, message, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(session_record_id)
    .bind(event_type)
    .bind(message)
    .bind(&now)
    .execute(executor)
    .await
    .map_err(|error| format!("写入会话事件失败: {error}"))?;
    Ok(id)
}

pub(super) async fn emit_native_line(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: Option<&str>,
    session_kind: &str,
    line: String,
) {
    emit_native_output(
        app,
        session_record_id,
        profile_id,
        workspace_id,
        session_kind,
        line,
        None,
        None,
        None,
    )
    .await;
}

pub(super) fn persist_stdout_message(
    line: &str,
    tool: Option<&NativeToolEvent>,
    images: Option<&[NativeToolImage]>,
    assistant: Option<&crate::db::models::NativeAssistantFragment>,
) -> String {
    let has_images = images.map(|items| !items.is_empty()).unwrap_or(false);
    if tool.is_none() && !has_images && assistant.is_none() {
        return line.to_string();
    }
    let mut value = serde_json::json!({
        "nox": 1,
        "line": line,
    });
    if let Some(tool) = tool {
        value["tool"] = serde_json::to_value(tool).unwrap_or(serde_json::Value::Null);
    }
    if let Some(assistant) = assistant {
        value["assistant"] = serde_json::to_value(assistant).unwrap_or(serde_json::Value::Null);
    }
    if let Some(images) = images {
        if tool.is_some_and(|event| event.mcp_server.is_some()) {
            let references: Vec<_> = images
                .iter()
                .filter_map(|image| {
                    image.attachment_id.as_ref().map(|id| {
                        serde_json::json!({
                            "name": image.name,
                            "mime_type": image.mime_type,
                            "data_url": "",
                            "attachment_id": id,
                        })
                    })
                })
                .collect();
            if !references.is_empty() {
                value["images"] = serde_json::json!(references);
            }
        } else if !images.is_empty() {
            value["images"] = serde_json::to_value(images).unwrap_or(serde_json::Value::Null);
        }
    }
    value.to_string()
}

pub(super) fn native_images_for_output(
    images: &[crate::native::model::types::NativeImage],
) -> Option<Vec<NativeToolImage>> {
    if images.is_empty() {
        None
    } else {
        Some(
            images
                .iter()
                .map(|image| NativeToolImage {
                    name: image.name.clone(),
                    mime_type: image.mime_type.clone(),
                    data_url: image.data_url(),
                    attachment_id: (!image.attachment_id.is_empty())
                        .then(|| image.attachment_id.clone()),
                })
                .collect(),
        )
    }
}

async fn retain_mcp_event_images(
    pool: &sqlx::SqlitePool,
    root: &std::path::Path,
    event_id: &str,
    images: &[NativeToolImage],
) -> Result<(), String> {
    let service = crate::native::attachments::AttachmentService::new(
        crate::native::images::attachments_dir(root),
        pool.clone(),
        "tool",
    );
    for (position, image) in images.iter().enumerate() {
        let Some(id) = image.attachment_id.as_deref() else {
            continue;
        };
        service
            .add_use(id, "event", event_id, position as i64, "{}")
            .await
            .map_err(|error| error.to_string())?;
        service
            .release_attachment_draft(id)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn emit_native_output(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: Option<&str>,
    session_kind: &str,
    line: String,
    tool: Option<NativeToolEvent>,
    images: Option<Vec<NativeToolImage>>,
    assistant: Option<crate::db::models::NativeAssistantFragment>,
) {
    let pool = match sqlite_pool(app).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    let persisted =
        persist_stdout_message(&line, tool.as_ref(), images.as_deref(), assistant.as_ref());
    let event_id = insert_session_event(&pool, session_record_id, "stdout", Some(&persisted))
        .await
        .ok();
    if tool
        .as_ref()
        .is_some_and(|event| event.mcp_server.is_some())
    {
        if let (Some(id), Some(images)) = (event_id.as_deref(), images.as_deref()) {
            let saved = match app.path().app_config_dir() {
                Ok(root) => retain_mcp_event_images(&pool, &root, id, images).await,
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = saved {
                eprintln!("[native] 保存 MCP 截图引用失败: {error}");
            }
        }
    }
    let _ = app.emit(
        "native-stdout",
        AgentSessionOutput {
            profile_id: profile_id.to_string(),
            workspace_id: workspace_id.map(ToOwned::to_owned),
            session_kind: session_kind.to_string(),
            session_record_id: session_record_id.to_string(),
            session_event_id: event_id.unwrap_or_default(),
            line,
            tool,
            images,
            assistant,
        },
    );
}
