use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};

use crate::app::network_settings::{load_network_settings, proxy_env_vars};
use crate::app::sessions::{
    lock_agent_session_operation, persist_context_usage_with, require_unarchived_session_with,
};
use crate::app::shared::{new_id, now_sqlite, sqlite_pool};
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::app::ssh::validate_password_execution;
use crate::db::models::{
    AgentSessionExit, AgentSessionOutput, AgentSessionRecord, AgentSessionStarted,
    NativeContextUsage, NativePlanModeChanged, NativeSessionConfigurationEvent,
    NativeSessionRuntime, NativeTextDelta, NativeToolEvent, NativeToolImage, NativeTurnState,
    SshConfigRecord, StartNativeSessionInput, UpdateNativeSessionConfigurationInput,
};
use crate::engine::context::{resolve_workspace_execution_context_with_pool, ExecutionContext};
use crate::engine::UsageDelta;
use crate::git::create_checkpoint;
use crate::native::agent::compact::{BudgetSnapshot, CompactTrigger, ContextWindow};
use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;
use crate::native::agent::r#loop::{AgentRunner, NativeEvent, TranscriptCheckpoint};
use crate::native::api_logs::sqlite_call_log_sink;
use crate::native::channels::{fetch_channel_record, require_channel_api_key};
use crate::native::input_queue::{NativeInputQueue, NativeInputQueueSnapshot};
use crate::native::live_model::{write_live_model, LiveModelSnapshot, SharedLiveModel};
use crate::native::manager::{
    take_latest_configuration, NativeAgentManager, NativeCompactionRequest,
    NativeConfigurationRequest, NativeFollowup, NativeLiveSession, NativeSessionInfo,
    PendingPermission, PendingPlanApproval, PendingPlanQuestion, PermissionRequest,
    PlanApprovalRequest, PlanQuestionRequest,
};
use crate::native::mcp_servers::resolve_session_mcp_servers;
use crate::native::model::call_log::{
    CallLogContext, CALL_KIND_CHAT, CALL_KIND_ONE_SHOT, CALL_KIND_PLAN,
};
use crate::native::model::types::StreamDelta;
use crate::native::model::{ModelClient, ModelClientConfig, ResponsesContinuationMode};
use crate::native::model_catalog::{
    apply_catalog_defaults, fill_from_catalog, resolve_runtime_reasoning_effort,
};
use crate::native::protocol::record_to_channel;
use crate::native::tools::dispatch::PlanApprovalAnswer;
use crate::native::tools::permission::{
    NativePermissionDecision, NativeToolRiskKind, PermissionRuleSuggestion,
};
use crate::native::tools::question::PlanQuestionAnswer;
use crate::native::tools::{
    connect_mcp_servers, local::LocalWorkspace, ssh::SshToolRuntime, SharedMcp,
};
use crate::native::transcript::{
    load_transcript, save_transcript, transcript_fingerprint, NativeTranscriptMeta,
};

const ENGINE_LABEL: &str = "内置 Agent";
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
async fn persist_native_transcript(
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

async fn persist_runner_transcript(
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

fn attach_transcript_checkpoint(
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
            persist_runner_transcript(
                &app,
                &session_record_id,
                &profile_id,
                &workspace_id,
                &model,
                &messages,
                last_fingerprint.as_ref(),
            )
            .await;
        })
    });
    runner.on_checkpoint = Some(hook);
}

fn user_turn_count(messages: &[crate::native::model::types::Message]) -> u32 {
    messages
        .iter()
        .filter(|message| message.role == crate::native::model::types::Role::User)
        .count() as u32
}

#[derive(Clone, Serialize)]
struct NativePermissionRequestEvent {
    session_record_id: String,
    request_id: String,
    profile_id: String,
    workspace_id: Option<String>,
    session_kind: String,
    tool_name: String,
    kind: NativeToolRiskKind,
    summary: String,
    remote: bool,
    mcp_server_id: Option<String>,
    suggested_rule: Option<PermissionRuleSuggestion>,
    file_access: Option<crate::native::tools::file_access::FileAccessPrompt>,
    allow_once_only: bool,
}

fn session_kind(plan_mode: bool) -> String {
    if plan_mode {
        "plan".to_string()
    } else {
        "execution".to_string()
    }
}

fn permission_event(
    session_record_id: &str,
    request: &PermissionRequest,
) -> NativePermissionRequestEvent {
    NativePermissionRequestEvent {
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
struct NativePlanQuestionEvent {
    session_record_id: String,
    request_id: String,
    profile_id: String,
    workspace_id: Option<String>,
    session_kind: String,
    questions: Vec<crate::native::tools::question::PlanQuestion>,
}

fn question_event(
    session_record_id: &str,
    request: &PlanQuestionRequest,
) -> NativePlanQuestionEvent {
    NativePlanQuestionEvent {
        session_record_id: session_record_id.to_string(),
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        workspace_id: request.workspace_id.clone(),
        session_kind: request.session_kind.clone(),
        questions: request.questions.clone(),
    }
}

#[derive(Clone, Serialize)]
struct NativePlanApprovalEvent {
    session_record_id: String,
    request_id: String,
    profile_id: String,
    workspace_id: Option<String>,
    session_kind: String,
    plan: String,
}

fn plan_approval_event(
    session_record_id: &str,
    request: &PlanApprovalRequest,
) -> NativePlanApprovalEvent {
    NativePlanApprovalEvent {
        session_record_id: session_record_id.to_string(),
        request_id: request.request_id.clone(),
        profile_id: request.profile_id.clone(),
        workspace_id: request.workspace_id.clone(),
        session_kind: request.session_kind.clone(),
        plan: request.plan.clone(),
    }
}

fn apply_bound_subagent(
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
fn hook_agent_handler(run: &NativeRunSettings) -> crate::native::tools::hooks::HookAgentHandler {
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
            let (message, _usage) = client
                .chat(crate::native::model::client::ChatRequest {
                    messages: &messages,
                    tools: &[],
                    model: &model,
                    effort: effort.as_deref(),
                    max_output_tokens: Some(512),
                    thinking_enabled: false,
                })
                .await?;
            Ok(message.content)
        })
    })
}

/// 本地工作区且开启记忆时：把记忆目录加入可读写根、索引块注入系统提示。返回记忆目录。
fn attach_memory(
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
async fn finish_memory(
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

async fn attach_skills_and_hooks(
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
async fn approve_workspace_hooks(
    ctx: &crate::native::tools::dispatch::ToolCtx,
    hooks: &[crate::db::models::NativeHook],
) -> bool {
    let Some(requester) = &ctx.request_permission else {
        return false;
    };
    if ctx.cancel.is_cancelled() {
        return false;
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    requester(
        crate::native::tools::dispatch::PermissionPrompt {
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
    matches!(
        decision,
        Some(NativePermissionDecision::AllowOnce | NativePermissionDecision::AllowSession)
    )
}

fn attach_subagent_runtime(
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
    runner.child_model_loader = Some(std::sync::Arc::new(move |channel_id, model| {
        let app = app_load.clone();
        Box::pin(async move {
            crate::native::subagents::resolve_child_model(&app, &channel_id, &model).await
        })
    }));
}

fn emit_turn_state(app: &AppHandle, session_record_id: &str, working: &AtomicBool, state: &str) {
    working.store(state == "working", Ordering::SeqCst);
    let _ = app.emit(
        "native-turn-state",
        NativeTurnState {
            session_record_id: session_record_id.to_string(),
            state: state.to_string(),
        },
    );
}

fn emit_plan_mode(app: &AppHandle, session_record_id: &str, input_queue_id: &str, plan_mode: bool) {
    let _ = app.emit(
        "native-plan-mode",
        NativePlanModeChanged {
            session_record_id: session_record_id.to_string(),
            plan_mode,
            input_queue_id: Some(input_queue_id.to_string()),
        },
    );
}

fn extra_headers_map(raw: Option<&str>) -> HashMap<String, String> {
    let Some(text) = raw.filter(|item| !item.trim().is_empty()) else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(text).unwrap_or_default()
}

const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(80);
const DELTA_FLUSH_BYTES: usize = 512;
const DELTA_SEGMENT_TEXT: &str = "text";
const DELTA_SEGMENT_REASONING: &str = "reasoning";

struct NativeDeltaEmitter {
    app: AppHandle,
    session_record_id: String,
    pending: Option<(&'static str, String)>,
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
                session_record_id: self.session_record_id.clone(),
                kind: segment.to_string(),
                text: delta,
                clear,
            },
        );
    }
}

async fn forward_native_events(
    app: AppHandle,
    session_record_id: String,
    profile_id: String,
    workspace_id: Option<String>,
    session_kind: String,
    mut event_rx: mpsc::UnboundedReceiver<NativeEvent>,
) {
    let mut deltas = NativeDeltaEmitter {
        app: app.clone(),
        session_record_id: session_record_id.clone(),
        pending: None,
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

async fn insert_session_event(
    pool: &sqlx::SqlitePool,
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
    .execute(pool)
    .await
    .map_err(|error| format!("写入会话事件失败: {error}"))?;
    Ok(id)
}

async fn emit_native_line(
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
    )
    .await;
}

fn persist_stdout_message(
    line: &str,
    tool: Option<&NativeToolEvent>,
    images: Option<&[NativeToolImage]>,
) -> String {
    let has_images = images.map(|items| !items.is_empty()).unwrap_or(false);
    if tool.is_none() && !has_images {
        return line.to_string();
    }
    let mut value = serde_json::json!({
        "nox": 1,
        "line": line,
    });
    if let Some(tool) = tool {
        value["tool"] = serde_json::to_value(tool).unwrap_or(serde_json::Value::Null);
    }
    if let Some(images) = images {
        if !images.is_empty() {
            value["images"] = serde_json::to_value(images).unwrap_or(serde_json::Value::Null);
        }
    }
    value.to_string()
}

fn native_images_for_output(
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
                })
                .collect(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn emit_native_output(
    app: &AppHandle,
    session_record_id: &str,
    profile_id: &str,
    workspace_id: Option<&str>,
    session_kind: &str,
    line: String,
    tool: Option<NativeToolEvent>,
    images: Option<Vec<NativeToolImage>>,
) {
    let pool = match sqlite_pool(app).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    let persisted = persist_stdout_message(&line, tool.as_ref(), images.as_deref());
    let event_id = insert_session_event(&pool, session_record_id, "stdout", Some(&persisted))
        .await
        .ok();
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
        },
    );
}

struct NativeRunSettings {
    client: ModelClient,
    model: String,
    /// 渠道的轻量模型（压缩 / 记忆 / 钩子判定），无则用主模型。
    lite_model: Option<String>,
    effort: Option<String>,
    max_output_tokens: Option<u32>,
    thinking_enabled: bool,
    context_tokens: Option<u32>,
    profile_system_prompt: Option<String>,
    protocol: String,
    channel_id: String,
    channel_name: String,
    bound_subagent: Option<crate::native::subagents::NativeSubagent>,
}

fn live_snapshot_from_run(
    run: &NativeRunSettings,
    context_token_limit: usize,
    execution_target: Option<String>,
    hook_agent: Option<crate::native::tools::hooks::HookAgentHandler>,
) -> LiveModelSnapshot {
    LiveModelSnapshot {
        revision: 0,
        client: run.client.clone(),
        model: run.model.clone(),
        channel_id: run.channel_id.clone(),
        channel_name: run.channel_name.clone(),
        protocol: run.protocol.clone(),
        lite_model: run.lite_model.clone(),
        effort: run.effort.clone(),
        max_output_tokens: run.max_output_tokens,
        thinking_enabled: run.thinking_enabled,
        context_tokens: run.context_tokens,
        context_token_limit,
        execution_target,
        hook_agent,
    }
}

fn sync_run_from_live(run: &mut NativeRunSettings, slot: &SharedLiveModel) {
    let snap = slot
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    run.client = snap.client;
    run.model = snap.model;
    run.channel_id = snap.channel_id;
    run.channel_name = snap.channel_name;
    run.protocol = snap.protocol;
    run.lite_model = snap.lite_model;
    run.effort = snap.effort;
    run.max_output_tokens = snap.max_output_tokens;
    run.thinking_enabled = snap.thinking_enabled;
    run.context_tokens = snap.context_tokens;
}

fn publish_live_run(
    slot: &SharedLiveModel,
    runner: &mut AgentRunner,
    run: &NativeRunSettings,
    app: &AppHandle,
    execution_target: Option<String>,
) {
    let limit =
        crate::native::settings::session_context_window_tokens(app, run.context_tokens) as usize;
    let revision = write_live_model(
        slot,
        live_snapshot_from_run(run, limit, execution_target, runner.ctx.hook_agent.clone()),
    );
    runner.note_live_model_revision(revision);
}

fn configure_runner_limits(
    app: &AppHandle,
    runner: &mut AgentRunner,
    model_context_tokens: Option<u32>,
) {
    let context_tokens =
        crate::native::settings::session_context_window_tokens(app, model_context_tokens) as usize;
    runner.context_char_limit = context_tokens.saturating_mul(2);
    runner.context_window.set_token_limit(context_tokens);
    if let Ok(settings) = crate::native::settings::load_native_settings(app) {
        runner.set_compaction_options(
            settings.auto_compact_threshold_percent.max(0) as u32,
            settings.microcompact_enabled,
        );
    }
    runner.tool_result_token_limit =
        crate::native::settings::effective_max_tool_output_tokens(app) as usize;
    runner.set_rollout_budget_limit(crate::native::settings::effective_rollout_token_budget(app));
    runner.max_subagent_turns = crate::native::settings::effective_max_subagent_turns(app);
    runner.subagent_budget_share_percent =
        crate::native::settings::effective_subagent_budget_share_percent(app);
    let timeout_secs = crate::native::settings::effective_permission_timeout_secs(app);
    runner.ctx.permission_timeout = if timeout_secs == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs(timeout_secs)
    };
}

fn format_native_diagnostics(
    budget: &BudgetSnapshot,
    context: &ContextWindow,
    diagnostics: &AgentDiagnosticsSnapshot,
) -> String {
    let limit = if budget.limit == 0 {
        "不限制".to_string()
    } else {
        format!("{} token", budget.limit)
    };
    format!(
        "Token 诊断：已用 {}，预算 {}，剩余 {}，活动预留 {}；上下文窗口代数 {}，压缩 {} 次，重置 {} 次，上限 {} token；工具结果截断 {} 次，启动子 Agent {} 个，预算停止 {} 次",
        budget.spent,
        limit,
        if budget.limit == 0 {
            "不限制".to_string()
        } else {
            format!("{} token", budget.remaining)
        },
        budget.active_reservations,
        context.generation,
        context.compactions,
        context.resets,
        context.token_limit,
        diagnostics.tool_results_truncated,
        diagnostics.subagents_started,
        diagnostics.budget_stops,
    )
}

fn native_startup_banner(
    channel_name: &str,
    protocol: &str,
    model: &str,
    effort: Option<&str>,
    thinking_enabled: bool,
) -> String {
    let effort = effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("默认");
    let thinking = if thinking_enabled { "on" } else { "off" };
    format!(
        "[内置 Agent] 启动会话 渠道={channel_name} 协议={protocol} model={model} effort={effort} thinking={thinking}"
    )
}

fn should_announce_session_startup(resume_session_id: Option<&str>) -> bool {
    resume_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
}

fn is_cancelled_run_error(error: &str) -> bool {
    error == "已取消"
}

fn is_mcp_error_status(text: &str) -> bool {
    let line = text.trim();
    line.starts_with("[MCP]")
        && (line.contains("无法连接")
            || line.contains("握手失败")
            || line.contains("读取配置失败")
            || line.contains("没有成功连接")
            || line.contains("已取消"))
}

pub struct NativeOneShotResult {
    pub text: String,
    pub usage_line: Option<String>,
    usage: Option<UsageDelta>,
}

pub(crate) struct NativeOneShotArgs<'a> {
    pub channel_id: &'a str,
    pub workspace_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub prompt: String,
    pub image_paths: Option<Vec<String>>,
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub operation: Option<&'a str>,
}

fn resolve_run_model_config(
    channel_models: &[crate::db::models::ChannelModelConfig],
    model: &str,
) -> crate::db::models::ChannelModelConfig {
    let mut config = channel_models
        .iter()
        .find(|item| item.id == model)
        .cloned()
        .unwrap_or_else(|| apply_catalog_defaults(model));
    fill_from_catalog(&mut config);
    config
}

async fn load_native_client_from_channel(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    channel_id: &str,
    model: &str,
    reasoning_effort: Option<&str>,
) -> Result<NativeRunSettings, String> {
    let record = fetch_channel_record(pool, channel_id).await?;
    if record.enabled == 0 {
        return Err(format!("渠道「{}」已停用", record.name));
    }
    let api_key = require_channel_api_key(&record)?;
    let channel = record_to_channel(record)?;
    let model = if model.trim().is_empty() {
        channel
            .models
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_else(|| "default".to_string())
    } else {
        model.to_string()
    };
    let model_config = resolve_run_model_config(&channel.models, &model);
    let thinking_enabled = model_config.thinking_enabled.unwrap_or(false);
    let effort = resolve_runtime_reasoning_effort(&model_config, reasoning_effort);
    let client = ModelClient::new(ModelClientConfig {
        protocol: channel.protocol.clone(),
        base_url: channel.base_url.clone(),
        api_key,
        extra_headers: extra_headers_map(channel.extra_headers_json.as_deref()),
        retry: crate::native::settings::effective_model_retry_config(app),
        timeout: Duration::from_secs(if thinking_enabled { 300 } else { 120 }),
        network: load_network_settings(app)?,
        responses_continuation: ResponsesContinuationMode::from_stored(
            &channel.responses_continuation,
        ),
    })?
    .with_call_log(
        CallLogContext {
            channel_id: Some(channel.id.clone()),
            channel_name: Some(channel.name.clone()),
            session_id: None,
            profile_id: None,
            workspace_id: None,
            subagent_id: None,
            call_kind: Some(CALL_KIND_CHAT.to_string()),
            execution_target: None,
            operation: None,
            model_role: None,
        },
        sqlite_call_log_sink(pool.clone()),
    );
    Ok(NativeRunSettings {
        client,
        lite_model: channel.lite_model.clone(),
        model,
        effort,
        max_output_tokens: model_config.max_output_tokens,
        thinking_enabled,
        context_tokens: model_config.context_tokens,
        profile_system_prompt: None,
        protocol: channel.protocol.clone(),
        channel_id: channel.id.clone(),
        channel_name: channel.name.clone(),
        bound_subagent: None,
    })
}

async fn load_native_client(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    channel_id: &str,
    model: &str,
    reasoning_effort: Option<&str>,
) -> Result<NativeRunSettings, String> {
    load_native_client_from_channel(app, pool, channel_id, model, reasoning_effort).await
}

fn native_one_shot_text(message: &crate::native::model::types::Message) -> Result<String, String> {
    let content = message.content.trim();
    if !content.is_empty() {
        return Ok(content.to_string());
    }
    let reasoning = message.reasoning_content.trim();
    if reasoning.is_empty() {
        return Err("内置 Agent 未返回可用内容".to_string());
    }
    if one_shot_reasoning_usable(reasoning) {
        return Ok(reasoning.to_string());
    }
    Err(format!(
        "模型只返回了思考内容（{} 字），没有正文。请将推理强度从 max 改为 high 或 low 后重试。",
        reasoning.chars().count()
    ))
}

fn one_shot_reasoning_usable(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.contains('{') && trimmed.contains('}'))
        || trimmed.starts_with('#')
        || trimmed.contains("\n# ")
        || trimmed.contains("\n## ")
}

async fn run_native_one_shot_with_run(
    run: NativeRunSettings,
    prompt: String,
    image_paths: Option<Vec<String>>,
) -> Result<NativeOneShotResult, String> {
    let loaded = crate::native::images::load_native_images(image_paths.as_deref());
    for path in &loaded.missing {
        eprintln!("[native] one-shot 附件图片不存在，已跳过: {path}");
    }
    for reason in &loaded.skipped {
        eprintln!("[native] one-shot 跳过图片: {reason}");
    }
    let user = if loaded.images.is_empty() {
        crate::native::model::types::Message::user(prompt)
    } else {
        crate::native::model::types::Message::user_with_images(prompt, loaded.images)
    };
    let (mut message, mut usage) = run
        .client
        .chat(crate::native::model::client::ChatRequest {
            messages: std::slice::from_ref(&user),
            tools: &[],
            model: &run.model,
            effort: run.effort.as_deref(),
            max_output_tokens: run.max_output_tokens,
            thinking_enabled: run.thinking_enabled,
        })
        .await
        .map_err(|error| format!("内置 Agent 一次性调用失败：{error}"))?;
    if native_one_shot_text(&message).is_err() && run.thinking_enabled {
        if let Ok((retry_message, retry_usage)) = run
            .client
            .chat(crate::native::model::client::ChatRequest {
                messages: std::slice::from_ref(&user),
                tools: &[],
                model: &run.model,
                effort: None,
                max_output_tokens: run.max_output_tokens,
                thinking_enabled: false,
            })
            .await
        {
            if native_one_shot_text(&retry_message).is_ok() {
                message = retry_message;
                usage = retry_usage;
            }
        }
    }
    let usage = crate::native::model::usage_to_delta(usage);
    Ok(NativeOneShotResult {
        text: native_one_shot_text(&message)?,
        usage_line: usage
            .as_ref()
            .and_then(|delta| delta.format_terminal_line()),
        usage,
    })
}

/// 思考模型只回 `reasoning_content` 的兜底，以及 DeepSeek `thinking.type=disabled`。
pub(crate) async fn run_native_one_shot(
    app: &AppHandle,
    args: NativeOneShotArgs<'_>,
) -> Result<NativeOneShotResult, String> {
    let pool = sqlite_pool(app).await?;
    let model = args.model.unwrap_or("");
    let mut run =
        load_native_client(app, &pool, args.channel_id, model, args.reasoning_effort).await?;
    let execution_target = if let Some(workspace_id) = args.workspace_id {
        resolve_workspace_execution_context_with_pool(&pool, workspace_id)
            .await?
            .execution_target
    } else {
        crate::app::shared::EXECUTION_TARGET_LOCAL.to_string()
    };
    let mut context = CallLogContext::for_session(
        Some(run.channel_id.clone()),
        Some(run.channel_name.clone()),
        args.session_id.map(ToOwned::to_owned),
        None,
        args.workspace_id.map(ToOwned::to_owned),
        CALL_KIND_ONE_SHOT,
        Some(execution_target),
    );
    if let Some(operation) = args.operation {
        context = context.with_operation(operation);
    }
    run.client = run.client.with_call_log_context(context);
    run_native_one_shot_with_run(run, args.prompt, args.image_paths).await
}

pub(crate) fn session_title(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(30).collect())
}

async fn resolve_insert_title(
    pool: &sqlx::SqlitePool,
    prompt: &str,
    resume_session_id: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(resume_id) = resume_session_id {
        let inherited = sqlx::query_scalar::<_, Option<String>>(
            "SELECT title FROM agent_sessions WHERE id = $1 LIMIT 1",
        )
        .bind(resume_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取续聊标题失败: {error}"))?;
        if let Some(title) = inherited {
            return Ok(title);
        }
    }
    Ok(session_title(prompt))
}

#[allow(clippy::too_many_arguments)]
async fn insert_agent_session(
    pool: &sqlx::SqlitePool,
    id: &str,
    ai_channel_id: &str,
    workspace_id: &str,
    working_dir: &str,
    execution_target: &str,
    ssh_config_id: Option<&str>,
    target_host_label: Option<&str>,
    kind: &str,
    resume_session_id: Option<&str>,
    prompt: &str,
) -> Result<String, String> {
    let now = now_sqlite();
    let title = resolve_insert_title(pool, prompt, resume_session_id).await?;
    sqlx::query(
        r#"
        INSERT INTO agent_sessions (
            id, ai_channel_id, workspace_id, working_dir, execution_target,
            ssh_config_id, target_host_label, session_kind, status,
            started_at, resume_session_id, created_at, title
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'running', $9, $10, $9, $11)
        "#,
    )
    .bind(id)
    .bind(ai_channel_id)
    .bind(workspace_id)
    .bind(working_dir)
    .bind(execution_target)
    .bind(ssh_config_id)
    .bind(target_host_label)
    .bind(kind)
    .bind(&now)
    .bind(resume_session_id)
    .bind(title)
    .execute(pool)
    .await
    .map_err(|error| format!("创建会话失败: {error}"))?;
    Ok(id.to_string())
}

async fn enqueue_live_input(
    manager: &Mutex<NativeAgentManager>,
    session_record_id: &str,
    input: &str,
    image_paths: Option<&[String]>,
) -> Result<Option<(NativeSessionInfo, NativeInputQueueSnapshot)>, String> {
    let trimmed = input.trim();
    let loaded = crate::native::images::load_native_images(image_paths);
    crate::native::images::cleanup_staged_loaded_images(&loaded);
    if trimmed.is_empty() && loaded.images.is_empty() {
        return Err("输入内容不能为空".to_string());
    }
    let manager = manager.lock().await;
    manager.require_running()?;
    let Some(session) = manager.get_session(session_record_id) else {
        return Ok(None);
    };
    if session.closing {
        return Err("内置 Agent 正在结束，请稍后重试".to_string());
    }
    let snapshot = session.input_queue.enqueue(trimmed, loaded.images)?;
    session.working.store(true, Ordering::SeqCst);
    Ok(Some((session.info.clone(), snapshot)))
}

#[allow(clippy::too_many_arguments)]
async fn reactivate_agent_session(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    workspace_id: &str,
    ai_channel_id: &str,
    working_dir: &str,
    execution_target: &str,
    ssh_config_id: Option<&str>,
    target_host_label: Option<&str>,
    kind: &str,
) -> Result<String, String> {
    require_unarchived_session_with(pool, session_id).await?;
    let session = sqlx::query_as::<_, AgentSessionRecord>(
        "SELECT * FROM agent_sessions WHERE id = $1 LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("读取会话失败: {error}"))?
    .ok_or_else(|| format!("会话不存在: {session_id}"))?;
    if session.workspace_id.as_deref() != Some(workspace_id) {
        return Err("会话不属于当前工作区".to_string());
    }
    let now = now_sqlite();
    sqlx::query(
        r#"
        UPDATE agent_sessions SET
            ai_channel_id = $1,
            workspace_id = $2,
            working_dir = $3,
            execution_target = $4,
            ssh_config_id = $5,
            target_host_label = $6,
            session_kind = $7,
            status = 'running',
            started_at = $8,
            ended_at = NULL,
            exit_code = NULL
        WHERE id = $9
        "#,
    )
    .bind(ai_channel_id)
    .bind(workspace_id)
    .bind(working_dir)
    .bind(execution_target)
    .bind(ssh_config_id)
    .bind(target_host_label)
    .bind(kind)
    .bind(&now)
    .bind(session_id)
    .execute(pool)
    .await
    .map_err(|error| format!("恢复会话失败: {error}"))?;
    Ok(session_id.to_string())
}

async fn update_agent_session_status(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    status: &str,
    exit_code: Option<i32>,
    ended_at: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE agent_sessions SET status = $1, exit_code = COALESCE($2, exit_code), ended_at = COALESCE($3, ended_at) WHERE id = $4",
    )
    .bind(status)
    .bind(exit_code)
    .bind(ended_at)
    .bind(session_record_id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新会话状态失败: {error}"))?;
    Ok(())
}

async fn apply_session_usage(
    pool: &sqlx::SqlitePool,
    session_record_id: &str,
    delta: &UsageDelta,
) -> Result<(), String> {
    sqlx::query(
        r#"
        UPDATE agent_sessions SET
            input_tokens = COALESCE(input_tokens, 0) + $1,
            output_tokens = COALESCE(output_tokens, 0) + $2,
            total_tokens = COALESCE(total_tokens, 0) + $3,
            reasoning_tokens = COALESCE(reasoning_tokens, 0) + $4,
            cached_tokens = COALESCE(cached_tokens, 0) + $5
        WHERE id = $6
        "#,
    )
    .bind(delta.input_tokens.unwrap_or(0) as i64)
    .bind(delta.output_tokens.unwrap_or(0) as i64)
    .bind(delta.total_tokens.unwrap_or(0) as i64)
    .bind(delta.reasoning_tokens.unwrap_or(0) as i64)
    .bind(delta.cached_tokens.unwrap_or(0) as i64)
    .bind(session_record_id)
    .execute(pool)
    .await
    .map_err(|error| format!("更新会话用量失败: {error}"))?;
    Ok(())
}

fn attach_mutation_checkpoint(
    app: &AppHandle,
    runner: &mut AgentRunner,
    workspace_id: String,
    session_record_id: String,
) {
    let enabled = crate::native::settings::load_native_settings(app)
        .map(|settings| settings.auto_checkpoint_after_tool_call)
        .unwrap_or(true);
    if !enabled {
        runner.ctx.on_mutation = None;
        return;
    }

    let inflight = Arc::new(AtomicBool::new(false));
    let app = app.clone();
    runner.ctx.on_mutation = Some(Arc::new(move |tool_name: &str| {
        if inflight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let inflight = inflight.clone();
        let app = app.clone();
        let workspace_id = workspace_id.clone();
        let session_record_id = session_record_id.clone();
        let label = format!("after_tool_call:{tool_name}");
        tauri::async_runtime::spawn(async move {
            let result = async {
                let pool = sqlite_pool(&app).await?;
                let target = crate::git::resolve_git_target(&app, &workspace_id).await?;
                create_checkpoint(
                    &pool,
                    &target,
                    &workspace_id,
                    &session_record_id,
                    Some(&label),
                    Some("after_tool_call"),
                )
                .await
                .map_err(String::from)?;
                Ok::<(), String>(())
            }
            .await;
            if let Err(error) = result {
                eprintln!("[native] after_tool_call 打点失败: {error}");
            }
            inflight.store(false, Ordering::SeqCst);
        });
    }));
}

/// 按设置装配本地工具运行时：Bash 默认超时、shell 快照、ripgrep、artifact 存储。
/// SSH 工作区不导出本机 shell 快照，但 artifact 目录仍在本机。
async fn configure_local_tool_runtime(
    app: &AppHandle,
    runner: &mut AgentRunner,
    session_record_id: &str,
) {
    let settings = match crate::native::settings::load_native_settings(app) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("[native] 读取运行时设置失败，使用默认值: {error}");
            return;
        }
    };
    runner.ctx.workspace.bash_default_timeout =
        Duration::from_secs(settings.bash_default_timeout_secs.max(1) as u64);
    let config_dir = app.path().app_config_dir().ok();
    if settings.rg_sidecar_enabled {
        let bundled = app.path().resource_dir().ok().map(|dir| dir.join("tools"));
        runner.ctx.workspace.rg_binary =
            crate::native::tools::local::locate_ripgrep(bundled.as_deref());
    }
    if settings.shell_snapshot_enabled && runner.ctx.ssh.is_none() {
        if let Some(dir) = config_dir.as_ref() {
            match crate::native::tools::shell_snapshot::capture_shell_snapshot(dir).await {
                Ok(path) => runner.ctx.workspace.shell_snapshot = Some(path),
                Err(error) => eprintln!("[native] 导出 shell 快照失败，回退 bash -lc: {error}"),
            }
        }
    }
    if let Some(dir) = config_dir.as_ref() {
        let record_app = app.clone();
        let store = crate::native::artifacts::ArtifactStore::new(dir, session_record_id)
            .with_recorder(Arc::new(move |record| {
                let app = record_app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(pool) = sqlite_pool(&app).await {
                        if let Err(error) =
                            crate::native::artifacts::insert_artifact_row(&pool, &record).await
                        {
                            eprintln!("[native] {error}");
                        }
                    }
                });
            }));
        runner
            .ctx
            .workspace
            .extra_read_roots
            .push(store.dir().to_path_buf());
        runner.set_artifact_store(Arc::new(store));
    }
}

async fn load_session_ssh_config(
    pool: &sqlx::SqlitePool,
    execution_context: &ExecutionContext,
) -> Result<Option<SshConfigRecord>, String> {
    if !execution_context.is_ssh() {
        return Ok(None);
    }
    let ssh_id = execution_context
        .ssh_config_id
        .as_deref()
        .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
    let config = fetch_ssh_config_record_by_id(pool, ssh_id).await?;
    validate_password_execution(&config)?;
    Ok(Some(config))
}

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
    start_native_session_locked(app, manager_state, payload).await
}

async fn start_native_session_locked(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    payload: StartNativeSessionInput,
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
        if let Some((info, queue)) = enqueue_live_input(
            manager_state.as_ref(),
            resume_id,
            &payload.prompt,
            payload.image_paths.as_deref(),
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
    let execution_context =
        resolve_workspace_execution_context_with_pool(&pool, &workspace_id).await?;
    let run_cwd = execution_context
        .working_dir
        .clone()
        .ok_or_else(|| format!("{ENGINE_LABEL} 工作区缺少工作目录"))?;
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

    if let Ok(target) = crate::git::resolve_git_target(&app, &workspace_id).await {
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
    };
    let input_queue = Arc::new(NativeInputQueue::new(&session_record_id));
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
            Some(PathBuf::from(&run_cwd))
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
        )
        .await;
    });

    let registered = manager_state.lock().await.add_session(NativeLiveSession {
        runtime: Some(runtime),
        plan_mode: Arc::new(AtomicBool::new(plan_mode)),
        background: None,
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
    let _ = app.emit("native-session", &started);
    let _ = loop_ready_tx.send(());
    Ok(started)
}

fn validate_live_configuration(
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

fn apply_run_settings_to_runner(runner: &mut AgentRunner, run: &NativeRunSettings) {
    runner.lite_model = run.lite_model.clone();
    runner.ctx.hook_agent = Some(hook_agent_handler(run));
    if let Some(scope) = runner.ctx.session_scope.as_mut() {
        scope.channel_id = run.channel_id.clone();
        scope.model = run.model.clone();
    }
}

async fn update_agent_session_channel(
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

fn bind_run_to_session(
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
async fn apply_session_configuration(
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
        let runtime = NativeSessionRuntime {
            ai_channel_id: next.channel_id.clone(),
            model: next.model.clone(),
            reasoning_effort: next.effort.clone(),
            permission_mode,
            plan_mode,
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
            .await
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
async fn dispatch_session_configuration(
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

#[allow(clippy::too_many_arguments)]
async fn run_native_loop(
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
) {
    let followup_rx = Arc::new(Mutex::new(followup_rx));
    let mut config_revision = 0_u64;
    let mut runner = AgentRunner::new(LocalWorkspace::new(PathBuf::from(&run_cwd)));
    runner.ctx.ssh = ssh;
    runner.ctx.extra_env = load_network_settings(&app)
        .map(|settings| proxy_env_vars(&settings))
        .unwrap_or_default();
    configure_local_tool_runtime(&app, &mut runner, &session_record_id).await;
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
    if let Some(session) = manager_state
        .lock()
        .await
        .get_session_mut(&session_record_id)
    {
        session.background = Some(runner.background.clone());
        runner.ctx.plan_mode = session.plan_mode.clone();
        runner.ctx.allow_session_commands = session.allow_session_commands.clone();
    }
    runner.steer_rx = Some(followup_rx.clone());
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
                "[PERMISSION] 完全访问（yolo）：包含工作区外文件访问；deny 规则仍拒绝，ask 规则仍需确认",
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
    // yolo 模式也保留确认通道：ask 规则命中时仍要问用户。
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
                    request_id: prompt.request_id.clone(),
                    profile_id: profile_id.clone(),
                    workspace_id: Some(workspace_id.clone()),
                    session_kind: kind.clone(),
                    plan: prompt.plan.clone(),
                };
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
                    let _ = app.emit(
                        "native-plan-approval-request",
                        plan_approval_event(&session_id, &request),
                    );
                }
            })
        }));
        runner.ctx.request_question = Some(std::sync::Arc::new(move |questions, reply| {
            let app = app_q.clone();
            let manager_state = manager_q.clone();
            let session_record_id = session_q.clone();
            let profile_id = profile_q.clone();
            let workspace_id = workspace_q.clone();
            let kind = kind_q.clone();
            tauri::async_runtime::spawn(async move {
                let request_id = uuid::Uuid::new_v4().to_string();
                let request = PlanQuestionRequest {
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
                Ok(Some(history)) => {
                    runner.messages.extend(history);
                    // 恢复到更小窗口的模型（或历史本就很长）时，第一次调用前按 downshift 压缩。
                    if runner.context_window.should_compact(&runner.messages) {
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
    let emit_join = tokio::spawn(async move {
        forward_native_events(
            emit_app,
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

    let loaded_images = crate::native::images::load_native_images(image_paths.as_deref());
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
    let await_followups = true;
    while let Some(prompt) = next.take() {
        if cancel.is_cancelled() {
            break;
        }
        emit_turn_state(&app, &session_record_id, &working, "working");
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
        )
        .await;
        // 每回合按关键词回忆相关记忆，附在用户消息后。
        if let Some(dir) = memory_dir.as_deref() {
            let hits = crate::native::memory::recall(dir, &prompt, 3);
            if !hits.is_empty() {
                runner.set_turn_suffix(crate::native::memory::format_recall_block(dir, &hits));
            }
        }
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
            Ok(_) => {}
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
                emit_turn_state(&app, &session_record_id, &working, "waiting_input");
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
                            emit_turn_state(&app, &session_record_id, &working, "working");
                            if runner
                                .compact_now(&run.client, request.instructions.take())
                                .await
                                .is_none()
                            {
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
                            emit_turn_state(&app, &session_record_id, &working, "waiting_input");
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
                                emit_turn_state(&app, &session_record_id, &working, "working");
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
    let _ = app.emit(
        "native-exit",
        AgentSessionExit {
            profile_id: profile_id.clone(),
            workspace_id: Some(workspace_id),
            session_kind: kind,
            session_record_id: session_record_id.clone(),
            code,
        },
    );
    manager_state
        .lock()
        .await
        .remove_session(&session_record_id);
}

async fn stop_native_process(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    session_record_id: &str,
    event_type: &str,
    message: &str,
) -> Result<bool, String> {
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
        manager.remove_session(session_record_id)
    };
    let Some(session) = session else {
        return Ok(true);
    };
    session.cancel.cancel();
    session.input_queue.close();
    let _ = session.followup_tx.send(NativeFollowup::Finish).await;
    let _ = session.join.await;
    Ok(true)
}

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
    drop(manager);
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

/// `/fork [checkpoint_id]`：复制会话上下文到一条新的（已结束、可续聊的）会话记录；
/// 给了 checkpoint 时先把工作区回滚到该检查点。返回新会话 id。
#[tauri::command]
pub async fn fork_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    checkpoint_id: Option<String>,
) -> Result<String, String> {
    if state.lock().await.get_session(&session_record_id).is_some() {
        return Err("会话仍在运行，请先等它完成或停止后再分叉".to_string());
    }
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
    if let Some(checkpoint_id) = checkpoint_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let workspace_id = source
            .workspace_id
            .as_deref()
            .ok_or_else(|| "会话没有工作区，无法回滚检查点".to_string())?;
        let target = crate::git::resolve_git_target(&app, workspace_id).await?;
        crate::git::restore_checkpoint(&pool, &target, workspace_id, checkpoint_id, &[])
            .await
            .map_err(|error| format!("回滚检查点失败: {error}"))?;
    }
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
    save_transcript(
        &pool,
        &new_id,
        &messages,
        &NativeTranscriptMeta {
            profile_id: None,
            workspace_id: source.workspace_id.clone(),
            model,
            turns,
        },
    )
    .await?;
    let _ = insert_session_event(
        &pool,
        &new_id,
        "stdout",
        Some(&format!(
            "[续聊] 从会话 {session_record_id} 分叉，共复制 {} 条消息",
            messages.len()
        )),
    )
    .await;
    Ok(new_id)
}

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

async fn apply_plan_implementation_model(
    app: &AppHandle,
    manager_state: &Arc<Mutex<NativeAgentManager>>,
    session_record_id: &str,
    ai_channel_id: &str,
    model: &str,
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
        if current_channel == ai_channel_id && current_model == model {
            return Ok(());
        }
        let effort = runtime
            .as_ref()
            .and_then(|item| item.reasoning_effort.clone());
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
        (
            effort,
            workspace_id,
            profile_id,
            session_kind,
            input_queue_id,
            execution_target,
        )
    };
    let (effort, workspace_id, profile_id, session_kind, input_queue_id, execution_target) =
        snapshot;
    let pool = sqlite_pool(app).await?;
    let mut next = load_native_client(app, &pool, ai_channel_id, model, effort.as_deref()).await?;
    next = bind_run_to_session(
        next,
        session_record_id,
        &workspace_id,
        false,
        execution_target.clone(),
    );
    update_agent_session_channel(&pool, session_record_id, &next.channel_id).await?;
    let context_token_limit =
        crate::native::settings::session_context_window_tokens(app, next.context_tokens) as usize;
    let hook_agent = hook_agent_handler(&next);
    let runtime = {
        let mut manager = manager_state.lock().await;
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
        let runtime = NativeSessionRuntime {
            ai_channel_id: next.channel_id.clone(),
            model: next.model.clone(),
            reasoning_effort: next.effort.clone(),
            permission_mode,
            plan_mode,
        };
        session.runtime = Some(runtime.clone());
        let transcript = session
            .transcript_model
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(next.model.clone())));
        (runtime, transcript)
    };
    let (runtime, transcript_model) = runtime;
    *transcript_model.lock().await = next.model.clone();
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
            "[内置 Agent] 实施改用 渠道={} 协议={} model={}",
            next.channel_name, next.protocol, next.model
        ),
    )
    .await;
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
) -> Result<(), String> {
    state
        .lock()
        .await
        .require_plan_approval(&session_record_id, &request_id)?;
    let channel = ai_channel_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty());
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty());
    if approved {
        if let (Some(channel), Some(model)) = (channel, model) {
            apply_plan_implementation_model(
                &app,
                state.inner(),
                &session_record_id,
                channel,
                model,
            )
            .await?;
        }
    }
    let next = state.lock().await.resolve_plan_approval(
        &session_record_id,
        &request_id,
        PlanApprovalAnswer {
            approved,
            feedback: feedback.unwrap_or_default(),
            ai_channel_id: channel.map(ToOwned::to_owned),
            model: model.map(ToOwned::to_owned),
        },
    )?;
    emit_request_resolved(&app, &session_record_id, &request_id, "plan_approval");
    if let Some(request) = next {
        let _ = app.emit(
            "native-plan-approval-request",
            plan_approval_event(&session_record_id, &request),
        );
    }
    Ok(())
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
    let next = state.lock().await.resolve_question(
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

#[tauri::command]
pub async fn send_native_input(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    input: String,
) -> Result<NativeInputQueueSnapshot, String> {
    crate::app::lifecycle::require_running(&app)?;
    let _operation = lock_agent_session_operation(&state, &session_record_id).await;
    require_unarchived_session_with(&sqlite_pool(&app).await?, &session_record_id).await?;
    enqueue_live_input(state.inner().as_ref(), &session_record_id, &input, None)
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

fn emit_request_resolved(app: &AppHandle, session_record_id: &str, request_id: &str, kind: &str) {
    let _ = app.emit(
        "native-request-resolved",
        serde_json::json!({
            "session_record_id": session_record_id, "request_id": request_id, "kind": kind,
        }),
    );
}

async fn background_registry(
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

async fn finish_live_input(
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
    start_native_session_locked(app, state.inner().clone(), payload).await
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

#[cfg(test)]
mod tests {
    use super::{
        format_native_diagnostics, is_cancelled_run_error, is_mcp_error_status,
        native_startup_banner, next_loop_step, should_announce_session_startup, NativeLoopAction,
        NativeLoopEvent,
    };
    use crate::native::agent::compact::{BudgetSnapshot, ContextWindow};
    use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;
    use crate::native::input_queue::NativeInputQueue;
    use crate::native::manager::NativeConfigurationRequest;

    #[tokio::test]
    async fn ssh_session_preflight_requires_verified_password_and_returns_config() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO ssh_configs (id, name, host, username, auth_type) VALUES ('ssh-preflight', 'test', 'example.test', 'tester', 'password')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let context = crate::engine::context::ExecutionContext {
            execution_target: "ssh".to_string(),
            working_dir: Some("/repo".to_string()),
            ssh_config_id: Some("ssh-preflight".to_string()),
            target_host_label: Some("tester@example.test:22".to_string()),
        };
        for status in [None, Some("failed"), Some("unknown")] {
            sqlx::query("UPDATE ssh_configs SET password_probe_status = $1, last_check_status = 'passed' WHERE id = 'ssh-preflight'")
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
            let error = super::load_session_ssh_config(&pool, &context)
                .await
                .unwrap_err();
            assert!(error.contains("密码"), "{error}");
            assert!(error.contains("设置"), "{error}");
        }
        for status in ["passed", "available"] {
            sqlx::query(
                "UPDATE ssh_configs SET password_probe_status = $1 WHERE id = 'ssh-preflight'",
            )
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
            let config = super::load_session_ssh_config(&pool, &context)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(config.id, "ssh-preflight");
            assert_eq!(config.password_probe_status.as_deref(), Some(status));
        }
        let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(session_count, 0);
    }

    #[tokio::test]
    async fn ssh_session_preflight_keeps_local_and_key_sessions_available() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        let mut context = crate::engine::context::ExecutionContext::local_default();
        assert!(super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap()
            .is_none());
        context.execution_target = "ssh".to_string();
        assert!(super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap_err()
            .contains("ssh_config_id"));
        context.ssh_config_id = Some("ssh-key-preflight".to_string());
        assert!(super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap_err()
            .contains("不存在"));
        sqlx::query(
            "INSERT INTO ssh_configs (id, name, host, username, auth_type, private_key_path) VALUES ('ssh-key-preflight', 'test', 'example.test', 'tester', 'key', '/test/id_ed25519')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let config = super::load_session_ssh_config(&pool, &context)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config.auth_type, "key");
        assert!(config.password_probe_status.is_none());
    }

    #[test]
    fn live_followup_rejects_silently_ignored_configuration_changes() {
        let runtime = crate::db::models::NativeSessionRuntime {
            ai_channel_id: "channel".to_string(),
            model: "model".to_string(),
            reasoning_effort: Some("high".to_string()),
            permission_mode: "default".to_string(),
            plan_mode: false,
        };
        let base =
            serde_json::json!({"ai_channel_id":"channel", "workspace_id":"ws", "prompt":"next"});
        let input = serde_json::from_value(base.clone()).unwrap();
        assert!(super::validate_live_configuration(&runtime, &input).is_ok());
        for (key, value) in [
            ("ai_channel_id", serde_json::json!("other-channel")),
            ("model", serde_json::json!("other-model")),
            ("reasoning_effort", serde_json::json!("low")),
            ("permission_mode", serde_json::json!("yolo")),
            ("plan_mode", serde_json::json!(true)),
        ] {
            let mut changed = base.clone();
            changed[key] = value;
            let input = serde_json::from_value(changed).unwrap();
            assert!(
                super::validate_live_configuration(&runtime, &input).is_err(),
                "{key}"
            );
        }
    }

    #[tokio::test]
    async fn recv_idle_wait_prefers_configuration_over_queued_input() {
        let (_followup_tx, mut followup_rx) = tokio::sync::mpsc::channel(8);
        let (config_tx, mut config_rx) = tokio::sync::mpsc::channel(8);
        let queue = NativeInputQueue::new("sess-config");
        queue
            .enqueue("already queued", Vec::new())
            .expect("enqueue");
        let (reply, _done) = tokio::sync::oneshot::channel();
        config_tx
            .send(NativeConfigurationRequest {
                request_id: "req-1".into(),
                ai_channel_id: "ch-new".into(),
                model: "model-new".into(),
                reasoning_effort: None,
                reply,
            })
            .await
            .unwrap();
        let wait = super::recv_idle_wait(
            &mut followup_rx,
            &mut config_rx,
            &queue,
            &crate::native::tools::CancelFlag::new(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .await;
        match wait {
            super::NativeIdleWait::Configuration(Some(request)) => {
                assert_eq!(request.request_id, "req-1");
                assert_eq!(request.model, "model-new");
            }
            _ => panic!("expected configuration first"),
        }
        assert_eq!(queue.snapshot().items.len(), 1);
        assert_eq!(queue.snapshot().items[0].text, "already queued");
    }

    #[tokio::test]
    async fn update_agent_session_channel_keeps_the_same_session_id() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type, created_at, updated_at) VALUES ('ws-ch', 'ws', 'local', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-old', 'old', 'openai', 'http://x'), ('ch-new', 'new', 'openai', 'http://y')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO agent_sessions (id, ai_channel_id, workspace_id, session_kind, status, started_at, created_at) VALUES ('sess-ch', 'ch-old', 'ws-ch', 'execution', 'running', 't', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
        super::update_agent_session_channel(&pool, "sess-ch", "ch-new")
            .await
            .unwrap();
        let channel: String =
            sqlx::query_scalar("SELECT ai_channel_id FROM agent_sessions WHERE id = 'sess-ch'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(channel, "ch-new");
    }

    #[tokio::test]
    async fn workspace_hooks_require_explicit_trust_even_with_yolo() {
        use crate::native::tools::dispatch::ToolCtx;
        use crate::native::tools::local::LocalWorkspace;
        use crate::native::tools::permission::NativePermissionDecision;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let root = tempfile::tempdir().unwrap();
        let hooks = vec![crate::db::models::NativeHook::shell(
            "workspace",
            "session_start",
            "*",
            "printf trusted",
            5,
            true,
        )];
        let mut ctx = ToolCtx::new(LocalWorkspace::new(root.path().to_path_buf()));
        ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
        ctx.auto_approve_opaque_bash = true;
        ctx.hooks = vec![crate::db::models::NativeHook::shell(
            "auto-allow",
            "permission_request",
            "*",
            "printf '{\"decision\":\"allow\"}'",
            5,
            true,
        )];
        assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        ctx.request_permission = Some(Arc::new(move |prompt, tx| {
            assert_eq!(prompt.tool_name, "WorkspaceHooks");
            assert!(prompt.suggested_rule.is_none());
            seen.fetch_add(1, Ordering::SeqCst);
            tx.send(NativePermissionDecision::Deny).unwrap();
        }));
        assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        ctx.request_permission = Some(Arc::new(|_, tx| {
            tx.send(NativePermissionDecision::AllowOnce).unwrap();
        }));
        assert!(super::approve_workspace_hooks(&ctx, &hooks).await);
        ctx.cancel.cancel();
        assert!(!super::approve_workspace_hooks(&ctx, &hooks).await);
    }

    #[tokio::test]
    async fn workspace_hook_trust_timeout_expires_request() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let root = tempfile::tempdir().unwrap();
        let mut ctx = crate::native::tools::dispatch::ToolCtx::new(
            crate::native::tools::local::LocalWorkspace::new(root.path().to_path_buf()),
        );
        ctx.permission_timeout = std::time::Duration::from_millis(10);
        let pending = Arc::new(std::sync::Mutex::new(None));
        let pending_request = pending.clone();
        ctx.request_permission = Some(Arc::new(move |_, tx| {
            *pending_request.lock().unwrap() = Some(tx);
        }));
        let expired = Arc::new(AtomicBool::new(false));
        let expired_handler = expired.clone();
        ctx.expire_permission = Some(Arc::new(move |_| {
            let expired = expired_handler.clone();
            tauri::async_runtime::spawn(async move {
                expired.store(true, Ordering::SeqCst);
            })
        }));
        assert!(!super::approve_workspace_hooks(&ctx, &[]).await);
        assert!(expired.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn graceful_finish_waits_for_completion_and_blocks_racing_input() {
        use crate::native::manager::{
            NativeAgentManager, NativeFollowup, NativeLiveSession, NativeSessionInfo,
        };
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let manager = Arc::new(tokio::sync::Mutex::new(NativeAgentManager::new()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let cancel = crate::native::tools::CancelFlag::new();
        let cancelled = cancel.clone();
        let manager_run = manager.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let finished_run = finished.clone();
        let (closing_tx, closing_rx) = tokio::sync::oneshot::channel();
        let join = tokio::spawn(async move {
            assert!(matches!(rx.recv().await, Some(NativeFollowup::Finish)));
            assert!(!cancelled.is_cancelled());
            let _ = closing_tx.send(());
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            finished_run.store(true, Ordering::SeqCst);
            manager_run.lock().await.remove_session("finish-test");
        });
        manager.lock().await.add_session(NativeLiveSession {
            info: NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch".to_string(),
                workspace_id: Some("ws".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: "finish-test".to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            allow_session_commands: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel,
            followup_tx: tx,
            config_tx: crate::native::manager::unused_config_tx(),
            input_queue: Arc::new(NativeInputQueue::new("sess-1")),
            join,
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            pending_compactions: Arc::default(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_permission: Default::default(),
            pending_question: Default::default(),
            pending_plan_approval: Default::default(),
            live_model: None,
            transcript_model: None,
        });
        let manager_finish = manager.clone();
        let finish =
            tokio::spawn(
                async move { super::finish_live_input(&manager_finish, "finish-test").await },
            );
        closing_rx.await.unwrap();
        assert!(!finished.load(Ordering::SeqCst));
        assert!(
            super::enqueue_live_input(&manager, "finish-test", "race", None)
                .await
                .is_err()
        );
        finish.await.unwrap().unwrap();
        assert!(finished.load(Ordering::SeqCst));
        assert!(manager.lock().await.get_session("finish-test").is_none());
        super::finish_live_input(&manager, "finish-test")
            .await
            .unwrap();
    }

    #[test]
    fn cancelled_run_error_is_not_a_failure() {
        assert!(is_cancelled_run_error("已取消"));
        assert!(!is_cancelled_run_error("模型超时"));
    }

    #[test]
    fn permission_event_includes_suggested_rule() {
        use crate::native::manager::PermissionRequest;
        use crate::native::tools::contract::{PatternSource, PermissionCapability};
        use crate::native::tools::permission::{NativeToolRiskKind, PermissionRuleSuggestion};

        let request = PermissionRequest {
            request_id: "r1".to_string(),
            profile_id: "p1".to_string(),
            workspace_id: Some("w1".to_string()),
            session_kind: "execution".to_string(),
            tool_name: "Bash".to_string(),
            kind: NativeToolRiskKind::Opaque,
            summary: "rm -rf /tmp/x".to_string(),
            file_access: None,
            allow_once_only: false,
            remote: false,
            mcp_server_id: None,
            suggested_rule: Some(PermissionRuleSuggestion {
                capability: PermissionCapability::Bash,
                pattern: "rm".to_string(),
                source: PatternSource::Command,
                plan_bash: None,
            }),
        };
        let event = super::permission_event("sess-1", &request);
        assert_eq!(event.suggested_rule.as_ref().unwrap().pattern, "rm");
        let json = serde_json::to_value(&event).expect("json");
        assert_eq!(json["suggested_rule"]["pattern"], "rm");
        assert_eq!(json["suggested_rule"]["capability"], "bash");
    }

    #[test]
    fn persist_stdout_message_wraps_tool_events() {
        use crate::db::models::{NativeToolEvent, NativeToolPhase};

        let tool = NativeToolEvent {
            phase: NativeToolPhase::Start,
            call_id: "c1".to_string(),
            name: "Read".to_string(),
            title: "读取 a.ts".to_string(),
            args_summary: "a.ts".to_string(),
            ok: None,
            duration_ms: None,
            result_preview: None,
            subagent_tag: None,
            mcp_server: None,
            mcp_tool: None,
            image_names: Vec::new(),
        };
        let raw = super::persist_stdout_message("[读取] a.ts", Some(&tool), None);
        let value: serde_json::Value = serde_json::from_str(&raw).expect("envelope");
        assert_eq!(value["nox"], 1);
        assert_eq!(value["line"], "[读取] a.ts");
        assert_eq!(value["tool"]["call_id"], "c1");
        assert_eq!(
            super::persist_stdout_message("[读取] a.ts", None, None),
            "[读取] a.ts"
        );
        let images = [crate::db::models::NativeToolImage {
            name: "a.png".to_string(),
            mime_type: "image/png".to_string(),
            data_url: "data:image/png;base64,QQ==".to_string(),
        }];
        let with_images =
            super::persist_stdout_message("[USER_INPUT] 看图", None, Some(images.as_slice()));
        let image_value: serde_json::Value = serde_json::from_str(&with_images).expect("envelope");
        assert_eq!(image_value["line"], "[USER_INPUT] 看图");
        assert_eq!(image_value["images"][0]["name"], "a.png");
    }

    #[test]
    fn native_one_shot_text_requires_non_empty_assistant() {
        let mut message = crate::native::model::types::Message::assistant_text("  ok  ");
        assert_eq!(super::native_one_shot_text(&message).as_deref(), Ok("ok"));
        message.content = "   ".to_string();
        assert_eq!(
            super::native_one_shot_text(&message).unwrap_err(),
            "内置 Agent 未返回可用内容"
        );
    }

    #[test]
    fn native_one_shot_text_uses_plan_shaped_reasoning() {
        let mut message = crate::native::model::types::Message::assistant_text("");
        message.reasoning_content =
            "{\"markdown\":\"# 计划\",\"steps\":[{\"title\":\"a\"}]}".to_string();
        assert!(super::native_one_shot_text(&message)
            .expect("usable reasoning")
            .contains("计划"));
    }

    #[test]
    fn native_one_shot_text_rejects_plain_reasoning() {
        let mut message = crate::native::model::types::Message::assistant_text("");
        message.reasoning_content = "先分析任务边界再给出步骤".to_string();
        let error = super::native_one_shot_text(&message).unwrap_err();
        assert!(error.contains("思考内容"));
        assert!(error.contains("没有正文"));
    }

    #[test]
    fn runtime_effort_clamps_to_channel_allowed_levels() {
        let mut config = crate::native::model_catalog::apply_catalog_defaults("gpt-5.6-luna");
        config.thinking_enabled = Some(true);
        config.thinking_levels = Some(vec!["low".to_string(), "high".to_string()]);
        config.thinking_level = Some("high".to_string());
        crate::native::model_catalog::fill_from_catalog(&mut config);
        let resolved =
            super::resolve_run_model_config(std::slice::from_ref(&config), "gpt-5.6-luna");
        assert_eq!(
            crate::native::model_catalog::resolve_runtime_reasoning_effort(&resolved, Some("max"))
                .as_deref(),
            Some("high")
        );
    }

    #[test]
    fn native_startup_banner_includes_model_and_channel() {
        assert_eq!(
            native_startup_banner("CRS", "codex", "gpt-5.6-luna", Some("high"), true),
            "[内置 Agent] 启动会话 渠道=CRS 协议=codex model=gpt-5.6-luna effort=high thinking=on"
        );
        assert_eq!(
            native_startup_banner("DeepSeek", "openai", "deepseek-v4-flash", None, false),
            "[内置 Agent] 启动会话 渠道=DeepSeek 协议=openai model=deepseek-v4-flash effort=默认 thinking=off"
        );
    }

    #[test]
    fn resume_does_not_announce_startup_or_restore_banners() {
        assert!(should_announce_session_startup(None));
        assert!(should_announce_session_startup(Some("")));
        assert!(should_announce_session_startup(Some("   ")));
        assert!(!should_announce_session_startup(Some("sess-1")));
        assert!(is_mcp_error_status(
            "[MCP] 无法连接 files：timeout（已跳过，不回退到其他位置）"
        ));
        assert!(is_mcp_error_status("[MCP] 握手失败 git：boom（已跳过）"));
        assert!(is_mcp_error_status("[MCP] 读取配置失败：bad json"));
        assert!(is_mcp_error_status("[MCP] 没有成功连接的服务器"));
        assert!(!is_mcp_error_status("[MCP] 未启用服务器"));
        assert!(!is_mcp_error_status("[MCP] 已连接：a"));
        assert!(!is_mcp_error_status("[续聊] 已恢复上一会话 2 条上下文"));
    }

    #[test]
    fn native_diagnostics_describe_budget_and_context_window() {
        let budget = BudgetSnapshot {
            limit: 200_000,
            spent: 12_345,
            remaining: 187_655,
            active_reservations: 256,
        };
        let context = ContextWindow {
            generation: 2,
            token_limit: 16_000,
            compactions: 1,
            resets: 1,
            threshold_percent: 85,
        };
        let details =
            format_native_diagnostics(&budget, &context, &AgentDiagnosticsSnapshot::default());
        assert!(details.contains("已用 12345"));
        assert!(details.contains("上下文窗口代数 2"));
        assert!(details.contains("压缩 1 次"));
        assert!(details.contains("重置 1 次"));
    }

    #[test]
    fn next_loop_step_covers_plan_followup_and_exit() {
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::TurnFinished),
            NativeLoopAction::WaitFollowup
        );
        assert_eq!(
            next_loop_step(false, NativeLoopEvent::TurnFinished),
            NativeLoopAction::Exit
        );
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::FollowupInput),
            NativeLoopAction::RunFollowup
        );
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::FollowupFinish),
            NativeLoopAction::Exit
        );
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::Cancelled),
            NativeLoopAction::Exit
        );
        assert_eq!(
            next_loop_step(true, NativeLoopEvent::Error),
            NativeLoopAction::Exit
        );
    }

    #[test]
    fn user_turn_count_counts_user_messages() {
        let messages = vec![
            crate::native::model::types::Message::system("s"),
            crate::native::model::types::Message::user("a"),
            crate::native::model::types::Message::assistant_text("b"),
            crate::native::model::types::Message::user("c"),
        ];
        assert_eq!(super::user_turn_count(&messages), 2);
    }

    #[test]
    fn session_title_truncates_unicode_scalars() {
        assert_eq!(super::session_title("  hello  ").as_deref(), Some("hello"));
        assert_eq!(super::session_title("   "), None);
        let chinese = "一二三四五六七八九十";
        let thirty = format!("{chinese}{chinese}{chinese}");
        let over = format!("{thirty}超出");
        assert_eq!(thirty.chars().count(), 30);
        assert!(over.len() > 30);
        assert_eq!(
            super::session_title(&over).as_deref(),
            Some(thirty.as_str())
        );
    }

    #[tokio::test]
    async fn insert_resume_inherits_source_title() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x')",
        )
        .execute(&pool)
        .await
        .expect("ch");
        let prompt = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十超出";
        let source = super::insert_agent_session(
            &pool,
            &crate::app::shared::new_id(),
            "ch-t",
            "ws-t",
            "/tmp",
            "local",
            None,
            None,
            "execution",
            None,
            prompt,
        )
        .await
        .expect("source");
        let source_title: Option<String> =
            sqlx::query_scalar("SELECT title FROM agent_sessions WHERE id = $1")
                .bind(&source)
                .fetch_one(&pool)
                .await
                .expect("source title");
        let resumed = super::insert_agent_session(
            &pool,
            &crate::app::shared::new_id(),
            "ch-t",
            "ws-t",
            "/tmp",
            "local",
            None,
            None,
            "execution",
            Some(&source),
            "继续",
        )
        .await
        .expect("resume");
        let resume_title: Option<String> =
            sqlx::query_scalar("SELECT title FROM agent_sessions WHERE id = $1")
                .bind(&resumed)
                .fetch_one(&pool)
                .await
                .expect("resume title");
        assert_eq!(
            source_title.as_deref(),
            Some("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十")
        );
        assert_eq!(resume_title, source_title);
    }

    #[tokio::test]
    async fn enqueue_live_input_queues_without_steering_the_active_turn() {
        use crate::native::manager::{NativeAgentManager, NativeLiveSession};
        use crate::native::tools::CancelFlag;
        use std::collections::VecDeque;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let mut manager = NativeAgentManager::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        manager.add_session(NativeLiveSession {
            info: crate::native::manager::NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch-1".to_string(),
                workspace_id: Some("ws-1".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: "sess-1".to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            allow_session_commands: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel: CancelFlag::new(),
            followup_tx: tx,
            config_tx: crate::native::manager::unused_config_tx(),
            input_queue: Arc::new(NativeInputQueue::new("sess-1")),
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            pending_compactions: Arc::default(),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_plan_approval: VecDeque::new(),
            live_model: None,
            transcript_model: None,
        });
        let manager = tokio::sync::Mutex::new(manager);

        let (info, snapshot) = super::enqueue_live_input(&manager, "sess-1", "  下一条  ", None)
            .await
            .expect("enqueue")
            .expect("live");
        assert_eq!(info.session_record_id, "sess-1");
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(snapshot.items[0].text, "下一条");
        let queue = manager
            .lock()
            .await
            .get_session("sess-1")
            .unwrap()
            .input_queue
            .clone();
        let item = queue
            .recv(&CancelFlag::new(), &AtomicBool::new(false))
            .await
            .unwrap();
        assert_eq!(item.text, "下一条");
        assert!(item.images.is_empty());
        assert!(super::enqueue_live_input(&manager, "missing", "x", None)
            .await
            .expect("missing")
            .is_none());
    }

    #[tokio::test]
    async fn enqueue_live_input_allows_images_without_text() {
        use crate::native::manager::{NativeAgentManager, NativeLiveSession};
        use crate::native::tools::CancelFlag;
        use std::collections::VecDeque;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("noxcode-followup-img-{stamp}.png"));
        std::fs::write(&path, b"\x89PNG\r\n").expect("png");

        let mut manager = NativeAgentManager::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        manager.add_session(NativeLiveSession {
            info: crate::native::manager::NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch-1".to_string(),
                workspace_id: Some("ws-1".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: "sess-1".to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            allow_session_commands: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel: CancelFlag::new(),
            followup_tx: tx,
            config_tx: crate::native::manager::unused_config_tx(),
            input_queue: Arc::new(NativeInputQueue::new("sess-1")),
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            pending_compactions: Arc::default(),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_plan_approval: VecDeque::new(),
            live_model: None,
            transcript_model: None,
        });
        let manager = tokio::sync::Mutex::new(manager);
        let paths = [path.to_string_lossy().into_owned()];
        super::enqueue_live_input(&manager, "sess-1", "   ", Some(paths.as_slice()))
            .await
            .expect("enqueue")
            .expect("live");
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        let queue = manager
            .lock()
            .await
            .get_session("sess-1")
            .unwrap()
            .input_queue
            .clone();
        let item = queue
            .recv(&CancelFlag::new(), &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(item.text.is_empty());
        assert_eq!(item.images.len(), 1);
        assert_eq!(
            item.images[0].name,
            path.file_name().unwrap().to_string_lossy()
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn reactivate_session_reuses_row_and_preserves_identity() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local'), ('ws-other', 'other', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x'), ('ch-new', 'ch2', 'openai', 'http://y')",
        )
        .execute(&pool)
        .await
        .expect("ch");
        sqlx::query(
            r#"
            INSERT INTO agent_sessions (
                id, ai_channel_id, workspace_id, working_dir, execution_target,
                session_kind, status, started_at, ended_at, exit_code, created_at,
                title, pinned, input_tokens, output_tokens, total_tokens
            ) VALUES (
                'sess-keep', 'ch-t', 'ws-t', '/old', 'local',
                'execution', 'exited', '2026-01-01 00:00:00', '2026-01-02 00:00:00', 0,
                '2026-01-01 00:00:00', '原标题', 1, 11, 22, 33
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed");

        let mismatched = super::reactivate_agent_session(
            &pool,
            "sess-keep",
            "ws-other",
            "ch-new",
            "/new",
            "local",
            None,
            None,
            "execution",
        )
        .await;
        assert!(mismatched
            .expect_err("workspace mismatch")
            .contains("会话不属于当前工作区"));

        let missing = super::reactivate_agent_session(
            &pool,
            "missing",
            "ws-t",
            "ch-new",
            "/new",
            "local",
            None,
            None,
            "execution",
        )
        .await;
        assert!(missing
            .expect_err("missing")
            .contains("会话不存在: missing"));

        let reactivated = super::reactivate_agent_session(
            &pool,
            "sess-keep",
            "ws-t",
            "ch-new",
            "/new",
            "local",
            None,
            None,
            "plan",
        )
        .await
        .expect("reactivate");
        assert_eq!(reactivated, "sess-keep");

        let row = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
            "SELECT * FROM agent_sessions WHERE id = 'sess-keep'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM agent_sessions")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
        assert_eq!(row.id, "sess-keep");
        assert_eq!(row.title.as_deref(), Some("原标题"));
        assert_eq!(row.pinned, 1);
        assert_eq!(row.created_at, "2026-01-01 00:00:00");
        assert_eq!(row.input_tokens, Some(11));
        assert_eq!(row.output_tokens, Some(22));
        assert_eq!(row.total_tokens, Some(33));
        assert_eq!(row.status, "running");
        assert_eq!(row.ai_channel_id.as_deref(), Some("ch-new"));
        assert_eq!(row.working_dir.as_deref(), Some("/new"));
        assert_eq!(row.session_kind, "plan");
        assert!(row.ended_at.is_none());
        assert!(row.exit_code.is_none());
        assert_ne!(row.started_at, "2026-01-01 00:00:00");
    }

    #[tokio::test]
    async fn reactivate_archived_session_is_rejected_without_changing_metadata() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO agent_sessions (id, title, archived, pinned, status, working_dir) VALUES ('archived', '保留名称', 1, 1, 'exited', '/old')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = super::reactivate_agent_session(
            &pool,
            "archived",
            "workspace",
            "channel",
            "/new",
            "local",
            None,
            None,
            "plan",
        )
        .await
        .unwrap_err();
        assert!(error.contains("已归档"));
        let row = sqlx::query_as::<_, crate::db::models::AgentSessionRecord>(
            "SELECT * FROM agent_sessions WHERE id = 'archived'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.archived, 1);
        assert_eq!(row.pinned, 1);
        assert_eq!(row.title.as_deref(), Some("保留名称"));
        assert_eq!(row.working_dir.as_deref(), Some("/old"));
        assert_eq!(row.status, "exited");
    }

    #[tokio::test]
    async fn reactivate_stale_running_session_keeps_same_id() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, workspace_type) VALUES ('ws-t', 'ws', 'local')",
        )
        .execute(&pool)
        .await
        .expect("ws");
        sqlx::query(
            "INSERT INTO ai_channels (id, name, protocol, base_url) VALUES ('ch-t', 'ch', 'openai', 'http://x')",
        )
        .execute(&pool)
        .await
        .expect("ch");
        sqlx::query(
            r#"
            INSERT INTO agent_sessions (
                id, ai_channel_id, workspace_id, working_dir, execution_target,
                session_kind, status, started_at, created_at, title
            ) VALUES (
                'sess-stale', 'ch-t', 'ws-t', '/old', 'local',
                'execution', 'running', '2026-01-01 00:00:00', '2026-01-01 00:00:00', '卡住'
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed");

        let reactivated = super::reactivate_agent_session(
            &pool,
            "sess-stale",
            "ws-t",
            "ch-t",
            "/old",
            "local",
            None,
            None,
            "execution",
        )
        .await
        .expect("reactivate stale");
        assert_eq!(reactivated, "sess-stale");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM agent_sessions")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
        let status: String =
            sqlx::query_scalar("SELECT status FROM agent_sessions WHERE id = 'sess-stale'")
                .fetch_one(&pool)
                .await
                .expect("status");
        assert_eq!(status, "running");
    }
}
