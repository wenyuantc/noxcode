use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex};

use crate::app::network_settings::load_network_settings;
use crate::app::shared::{new_id, now_sqlite, sqlite_pool, EXECUTION_TARGET_SSH};
use crate::app::ssh::configs::fetch_ssh_config_record_by_id;
use crate::db::models::{
    AgentSessionExit, AgentSessionOutput, AgentSessionRecord, AgentSessionStarted,
    NativeContextUsage, NativeTextDelta, NativeTurnState, StartNativeSessionInput,
};
use crate::engine::context::resolve_workspace_execution_context_with_pool;
use crate::engine::UsageDelta;
use crate::git::create_checkpoint;
use crate::native::agent::compact::{BudgetSnapshot, ContextWindow};
use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;
use crate::native::agent::r#loop::{AgentRunner, NativeEvent};
use crate::native::api_logs::sqlite_call_log_sink;
use crate::native::channels::{fetch_channel_record, require_channel_api_key};
use crate::native::manager::{
    NativeAgentManager, NativeFollowup, NativeLiveSession, NativeSessionInfo, PendingPermission,
    PendingPlanQuestion, PermissionRequest, PlanQuestionRequest,
};
use crate::native::mcp_servers::resolve_effective_mcp_servers;
use crate::native::model::call_log::{
    CallLogContext, CALL_KIND_CHAT, CALL_KIND_ONE_SHOT, CALL_KIND_PLAN,
};
use crate::native::model::types::StreamDelta;
use crate::native::model::{ModelClient, ModelClientConfig, RetryConfig};
use crate::native::model_catalog::{
    apply_catalog_defaults, fill_from_catalog, resolve_runtime_reasoning_effort,
};
use crate::native::protocol::record_to_channel;
use crate::native::tools::permission::{NativePermissionDecision, NativeToolRiskKind};
use crate::native::tools::question::PlanQuestionAnswer;
use crate::native::tools::{
    connect_mcp_servers, local::LocalWorkspace, ssh::SshToolRuntime, SharedMcp,
};
use crate::native::transcript::{
    load_transcript, save_transcript, transcript_fingerprint, NativeTranscriptMeta,
};

const ENGINE_LABEL: &str = "内置 Agent";
const EXECUTE_AFTER_PLAN: &str =
    "计划阶段已结束，写工具现已可用。按你刚才输出的方案立即实施，不要重新规划。";

fn usable_native_plan_text(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeLoopEvent {
    TurnFinished { plan_pending: bool },
    FollowupInput,
    FollowupFinish,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeLoopAction {
    PersistPlanAndExecute,
    WaitFollowup,
    RunFollowup,
    Exit,
}

pub(crate) fn next_loop_step(await_followups: bool, event: NativeLoopEvent) -> NativeLoopAction {
    match event {
        NativeLoopEvent::Cancelled | NativeLoopEvent::Error => NativeLoopAction::Exit,
        NativeLoopEvent::TurnFinished { plan_pending: true } => {
            NativeLoopAction::PersistPlanAndExecute
        }
        NativeLoopEvent::TurnFinished {
            plan_pending: false,
        } if await_followups => NativeLoopAction::WaitFollowup,
        NativeLoopEvent::TurnFinished {
            plan_pending: false,
        } => NativeLoopAction::Exit,
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

async fn attach_skills_and_hooks(
    app: &AppHandle,
    runner: &mut AgentRunner,
    parts: &mut crate::native::prompt::NativePromptParts,
) {
    if let Ok(settings) = crate::native::settings::load_native_settings(app) {
        runner.ctx.hooks = settings.hooks;
    }
    let config_dir = app.path().app_config_dir().ok();
    let skills = crate::native::skills::load_session_skills(
        &parts.cwd,
        runner.ctx.ssh.as_ref(),
        config_dir.as_deref(),
    )
    .await;
    parts.skills = crate::native::skills::format_skills_prompt(&skills);
    runner.skills_prompt = parts.skills.clone();
    runner.ctx.skills = skills;
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
    let loaded = crate::native::subagents::load_native_subagents(app).unwrap_or_default();
    runner.custom_subagents =
        crate::native::subagents::catalog_for_session(&loaded, workspace_id, bound);
    let app_reload = app.clone();
    let workspace_id_owned = workspace_id.map(ToOwned::to_owned);
    let bound_owned = bound.cloned();
    runner.reload_custom_subagents = Some(std::sync::Arc::new(move || {
        let loaded =
            crate::native::subagents::load_native_subagents(&app_reload).unwrap_or_default();
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

fn emit_turn_state(app: &AppHandle, session_record_id: &str, state: &str) {
    let _ = app.emit(
        "native-turn-state",
        NativeTurnState {
            session_record_id: session_record_id.to_string(),
            state: state.to_string(),
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
                    NativeEvent::Delta(StreamDelta::Text(text)) => {
                        deltas.push(DELTA_SEGMENT_TEXT, &text);
                    }
                    NativeEvent::Delta(StreamDelta::Reasoning(text)) => {
                        deltas.push(DELTA_SEGMENT_REASONING, &text);
                    }
                    NativeEvent::Delta(StreamDelta::Reset) => deltas.clear(),
                    NativeEvent::ContextUsage(snapshot) => {
                        let _ = app.emit(
                            "native-context-usage",
                            NativeContextUsage {
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
                            },
                        );
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
    let pool = match sqlite_pool(app).await {
        Ok(pool) => pool,
        Err(_) => return,
    };
    let event_id = insert_session_event(&pool, session_record_id, "stdout", Some(&line))
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
        },
    );
}

struct NativeRunSettings {
    client: ModelClient,
    model: String,
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

fn configure_runner_limits(
    app: &AppHandle,
    runner: &mut AgentRunner,
    model_context_tokens: Option<u32>,
) {
    let context_tokens =
        crate::native::settings::session_context_window_tokens(app, model_context_tokens) as usize;
    runner.context_char_limit = context_tokens.saturating_mul(2);
    runner.context_window.set_token_limit(context_tokens);
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
        retry: RetryConfig::default(),
        timeout: Duration::from_secs(if thinking_enabled { 300 } else { 120 }),
        network: load_network_settings(app)?,
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
        },
        sqlite_call_log_sink(pool.clone()),
    );
    Ok(NativeRunSettings {
        client,
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
#[allow(dead_code)]
pub(crate) async fn run_native_one_shot(
    app: &AppHandle,
    channel_id: &str,
    workspace_id: Option<&str>,
    prompt: String,
    image_paths: Option<Vec<String>>,
) -> Result<NativeOneShotResult, String> {
    let pool = sqlite_pool(app).await?;
    let mut run = load_native_client(app, &pool, channel_id, "", None).await?;
    let execution_target = if let Some(workspace_id) = workspace_id {
        resolve_workspace_execution_context_with_pool(&pool, workspace_id)
            .await?
            .execution_target
    } else {
        crate::app::shared::EXECUTION_TARGET_LOCAL.to_string()
    };
    run.client = run
        .client
        .with_call_log_context(CallLogContext::for_session(
            Some(run.channel_id.clone()),
            Some(run.channel_name.clone()),
            None,
            None,
            workspace_id.map(ToOwned::to_owned),
            CALL_KIND_ONE_SHOT,
            Some(execution_target),
        ));
    run_native_one_shot_with_run(run, prompt, image_paths).await
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
    let id = new_id();
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
    .bind(&id)
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
    Ok(id)
}

async fn enqueue_live_input(
    manager: &Mutex<NativeAgentManager>,
    session_record_id: &str,
    input: &str,
) -> Result<Option<NativeSessionInfo>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("输入内容不能为空".to_string());
    }
    let live = {
        let manager = manager.lock().await;
        manager
            .get_session(session_record_id)
            .map(|session| (session.info.clone(), session.followup_tx.clone()))
    };
    let Some((info, tx)) = live else {
        return Ok(None);
    };
    tx.send(NativeFollowup::Input(trimmed.to_string()))
        .await
        .map_err(|_| "内置 Agent 会话已结束，无法发送输入".to_string())?;
    Ok(Some(info))
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

#[tauri::command]
pub async fn start_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
    start_native_with_manager(app, state.inner().clone(), payload).await
}

async fn start_native_with_manager(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
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
        if let Some(info) =
            enqueue_live_input(manager_state.as_ref(), resume_id, &payload.prompt).await?
        {
            let started = AgentSessionStarted {
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
        ));

    let ssh = if execution_context.execution_target == EXECUTION_TARGET_SSH {
        let ssh_id = execution_context
            .ssh_config_id
            .as_deref()
            .ok_or_else(|| "SSH 工作区缺少 ssh_config_id".to_string())?;
        let config = fetch_ssh_config_record_by_id(&pool, ssh_id).await?;
        Some(SshToolRuntime {
            app: app.clone(),
            config,
            root: run_cwd.clone(),
        })
    } else {
        None
    };

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

    let started = AgentSessionStarted {
        profile_id: String::new(),
        workspace_id: workspace_id.clone(),
        session_kind: kind.clone(),
        session_record_id: session_record_id.clone(),
    };
    let _ = app.emit("native-session", &started);

    let (followup_tx, followup_rx) = mpsc::channel(8);
    let cancel = crate::native::tools::CancelFlag::new();
    let permission_mode = crate::native::settings::effective_permission_mode(&app);
    let allow_all_high_risk = Arc::new(AtomicBool::new(
        permission_mode == crate::native::settings::PERMISSION_MODE_FULL,
    ));
    let (loop_ready_tx, loop_ready_rx) = tokio::sync::oneshot::channel();
    let manager_spawn = manager_state.clone();
    let app_spawn = app.clone();
    let cancel_run = cancel.clone();
    let allow_all_run = allow_all_high_risk.clone();
    let session_spawn = session_record_id.clone();
    let profile_spawn = String::new();
    let workspace_spawn = workspace_id.clone();
    let kind_spawn = kind.clone();
    let image_paths = payload.image_paths.clone();
    let resume_run = payload.resume_session_id.clone();
    let join = tokio::spawn(async move {
        let _ = loop_ready_rx.await;
        run_native_loop(
            app_spawn,
            manager_spawn,
            run,
            prompt,
            run_cwd,
            ssh,
            cancel_run,
            allow_all_run,
            followup_rx,
            session_spawn,
            profile_spawn,
            workspace_spawn,
            kind_spawn,
            image_paths,
            plan_mode,
            resume_run,
        )
        .await;
    });

    manager_state.lock().await.add_session(NativeLiveSession {
        info: NativeSessionInfo {
            profile_id: String::new(),
            channel_id: channel_id.clone(),
            workspace_id: Some(workspace_id.clone()),
            session_kind: kind,
            session_record_id: session_record_id.clone(),
        },
        cancel,
        followup_tx,
        join,
        allow_all_high_risk,
        pending_permission: std::collections::VecDeque::new(),
        pending_question: std::collections::VecDeque::new(),
    });
    let _ = loop_ready_tx.send(());
    Ok(started)
}

#[allow(clippy::too_many_arguments)]
async fn run_native_loop(
    app: AppHandle,
    manager_state: Arc<Mutex<NativeAgentManager>>,
    run: NativeRunSettings,
    first_prompt: String,
    run_cwd: String,
    ssh: Option<SshToolRuntime>,
    cancel: crate::native::tools::CancelFlag,
    allow_all_high_risk: Arc<AtomicBool>,
    followup_rx: mpsc::Receiver<NativeFollowup>,
    session_record_id: String,
    profile_id: String,
    workspace_id: String,
    kind: String,
    image_paths: Option<Vec<String>>,
    plan_mode: bool,
    resume_session_id: Option<String>,
) {
    let followup_rx = Arc::new(Mutex::new(followup_rx));
    let mut runner = AgentRunner::new(LocalWorkspace::new(PathBuf::from(&run_cwd)));
    runner.ctx.ssh = ssh;
    runner.ctx.cancel = cancel.clone();
    runner.ctx.allow_all_high_risk = allow_all_high_risk;
    runner.steer_rx = Some(followup_rx.clone());
    if plan_mode {
        runner.set_read_only(true);
        runner.set_plan_mode(true);
    }
    attach_mutation_checkpoint(
        &app,
        &mut runner,
        workspace_id.clone(),
        session_record_id.clone(),
    );
    let announce_startup = should_announce_session_startup(resume_session_id.as_deref());
    let permission_mode = crate::native::settings::effective_permission_mode(&app);
    runner.ctx.auto_approve_overwrite =
        permission_mode == crate::native::settings::PERMISSION_MODE_AUTO_EDIT;
    if announce_startup && permission_mode == crate::native::settings::PERMISSION_MODE_FULL {
        emit_native_line(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            "[PERMISSION] 已在设置中关闭高风险确认，本会话工具将直接执行".to_string(),
        )
        .await;
    }
    if announce_startup && permission_mode == crate::native::settings::PERMISSION_MODE_AUTO_EDIT {
        emit_native_line(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            "[PERMISSION] 已开启自动编辑：覆盖文件直接执行，删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍需确认".to_string(),
        )
        .await;
    }
    if permission_mode != crate::native::settings::PERMISSION_MODE_FULL {
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
                        "[PERMISSION] 等待确认高风险操作（{location} / {:?}）：{}",
                        prompt.kind, prompt.summary
                    ),
                )
                .await;
                if should_emit {
                    let _ = app.emit(
                        "native-permission-request",
                        permission_event(&session_record_id, &request),
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
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    "[PERMISSION] 确认超时，已按拒绝处理".to_string(),
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
    if plan_mode {
        let app_q = app.clone();
        let manager_q = manager_state.clone();
        let session_q = session_record_id.clone();
        let profile_q = profile_id.clone();
        let workspace_q = workspace_id.clone();
        let kind_q = kind.clone();
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
            "plan".to_string()
        } else {
            String::new()
        },
        skills: String::new(),
    };
    attach_skills_and_hooks(&app, &mut runner, &mut parts).await;
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
                "[PLAN] 已进入计划模式：只读摸底，本轮结束后自动开始执行".to_string(),
            )
            .await;
        }
    }
    match resolve_effective_mcp_servers(&app) {
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
    let mut plan_pending = plan_mode;
    let mut last_transcript_fingerprint: Option<u64> = None;
    let await_followups = true;
    while let Some(prompt) = next.take() {
        emit_turn_state(&app, &session_record_id, "working");
        emit_native_line(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &kind,
            format!("[USER_INPUT] {prompt}"),
        )
        .await;
        let images = std::mem::take(&mut pending_images);
        let plan_text = match runner
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
            Ok(text) => text,
            Err(error) => {
                last_error = Some(error.clone());
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
                let _ = next_loop_step(await_followups, NativeLoopEvent::Error);
                break;
            }
        };
        persist_native_transcript(
            &app,
            &session_record_id,
            &profile_id,
            Some(&workspace_id),
            &run.model,
            user_turn_count(&runner.messages),
            &runner.messages,
            &mut last_transcript_fingerprint,
        )
        .await;
        match next_loop_step(
            await_followups,
            NativeLoopEvent::TurnFinished { plan_pending },
        ) {
            NativeLoopAction::PersistPlanAndExecute => {
                if let Some(plan) = usable_native_plan_text(&plan_text) {
                    emit_native_line(
                        &app,
                        &session_record_id,
                        &profile_id,
                        Some(&workspace_id),
                        &kind,
                        format!("[PLAN]\n{plan}"),
                    )
                    .await;
                }
                if cancel.is_cancelled() {
                    break;
                }
                plan_pending = false;
                runner.set_read_only(false);
                runner.set_plan_mode(false);
                runner.ctx.request_question = None;
                emit_native_line(
                    &app,
                    &session_record_id,
                    &profile_id,
                    Some(&workspace_id),
                    &kind,
                    "[PLAN] 开始执行".to_string(),
                )
                .await;
                next = Some(if let Some(def) = run.bound_subagent.as_ref() {
                    runner.required_subagent_type = Some(def.name.clone());
                    crate::native::prompt::wrap_prompt_for_required_subagent(
                        EXECUTE_AFTER_PLAN,
                        &def.name,
                    )
                } else {
                    EXECUTE_AFTER_PLAN.to_string()
                });
                continue;
            }
            NativeLoopAction::WaitFollowup => {
                if cancel.is_cancelled() {
                    let _ = next_loop_step(await_followups, NativeLoopEvent::Cancelled);
                    break;
                }
                if runner.take_steer_finish() {
                    let _ = next_loop_step(await_followups, NativeLoopEvent::FollowupFinish);
                    break;
                }
                emit_turn_state(&app, &session_record_id, "waiting_input");
                match followup_rx.lock().await.recv().await {
                    Some(NativeFollowup::Input(input)) => {
                        match next_loop_step(await_followups, NativeLoopEvent::FollowupInput) {
                            NativeLoopAction::RunFollowup => {
                                emit_turn_state(&app, &session_record_id, "working");
                                next = Some(input);
                            }
                            _ => break,
                        }
                    }
                    Some(NativeFollowup::Finish) | None => {
                        let _ = next_loop_step(await_followups, NativeLoopEvent::FollowupFinish);
                        break;
                    }
                }
            }
            NativeLoopAction::Exit | NativeLoopAction::RunFollowup => break,
        }
    }

    persist_native_transcript(
        &app,
        &session_record_id,
        &profile_id,
        Some(&workspace_id),
        &run.model,
        user_turn_count(&runner.messages),
        &runner.messages,
        &mut last_transcript_fingerprint,
    )
    .await;

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

    let failed = last_error.is_some() && last_error.as_deref() != Some("已取消");
    let status = if failed { "failed" } else { "exited" };
    let code = if failed { 1 } else { 0 };
    let ended_at = now_sqlite();
    if let Ok(pool) = sqlite_pool(&app).await {
        let _ = update_agent_session_status(
            &pool,
            &session_record_id,
            status,
            Some(code),
            Some(ended_at.as_str()),
        )
        .await;
    }
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
) -> Result<(), String> {
    let next = state
        .lock()
        .await
        .resolve_permission(&session_record_id, &request_id, decision)?;
    if let Some(request) = next {
        let _ = app.emit(
            "native-permission-request",
            permission_event(&session_record_id, &request),
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
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
    input: String,
) -> Result<(), String> {
    if enqueue_live_input(state.inner().as_ref(), &session_record_id, &input)
        .await?
        .is_none()
    {
        return Err(format!(
            "会话 {session_record_id} 当前没有运行中的内置 Agent"
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn finish_native_input(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    session_record_id: String,
) -> Result<(), String> {
    if !stop_native_process(
        &app,
        state.inner(),
        &session_record_id,
        "stopping_requested",
        "收到结束输入请求",
    )
    .await?
    {
        return Err(format!(
            "会话 {session_record_id} 当前没有运行中的内置 Agent"
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn restart_native_session(
    app: AppHandle,
    state: State<'_, Arc<Mutex<NativeAgentManager>>>,
    payload: StartNativeSessionInput,
) -> Result<AgentSessionStarted, String> {
    let restart_id = payload
        .resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned);
    if let Some(session_id) = restart_id {
        let _ = stop_native_process(
            &app,
            state.inner(),
            &session_id,
            "restart_requested",
            "收到重启请求",
        )
        .await?;
    }
    start_native_with_manager(app, state.inner().clone(), payload).await
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
        format_native_diagnostics, is_mcp_error_status, native_startup_banner, next_loop_step,
        should_announce_session_startup, usable_native_plan_text, NativeLoopAction,
        NativeLoopEvent,
    };
    use crate::native::agent::compact::{BudgetSnapshot, ContextWindow};
    use crate::native::agent::r#loop::AgentDiagnosticsSnapshot;

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
    fn usable_native_plan_text_requires_non_empty_body() {
        assert_eq!(
            usable_native_plan_text("  目标与范围  "),
            Some("目标与范围")
        );
        assert_eq!(usable_native_plan_text("   \n\t"), None);
        assert_eq!(usable_native_plan_text(""), None);
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
            next_loop_step(false, NativeLoopEvent::TurnFinished { plan_pending: true }),
            NativeLoopAction::PersistPlanAndExecute
        );
        assert_eq!(
            next_loop_step(
                true,
                NativeLoopEvent::TurnFinished {
                    plan_pending: false
                }
            ),
            NativeLoopAction::WaitFollowup
        );
        assert_eq!(
            next_loop_step(
                false,
                NativeLoopEvent::TurnFinished {
                    plan_pending: false
                }
            ),
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
    async fn enqueue_live_input_sends_followup_without_reactivate() {
        use crate::native::manager::{NativeAgentManager, NativeFollowup, NativeLiveSession};
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
            cancel: CancelFlag::new(),
            followup_tx: tx,
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
        });
        let manager = tokio::sync::Mutex::new(manager);

        let info = super::enqueue_live_input(&manager, "sess-1", "  下一条  ")
            .await
            .expect("enqueue")
            .expect("live");
        assert_eq!(info.session_record_id, "sess-1");
        match rx.recv().await {
            Some(NativeFollowup::Input(text)) => assert_eq!(text, "下一条"),
            Some(NativeFollowup::Finish) => panic!("unexpected finish"),
            None => panic!("channel closed"),
        }
        assert!(super::enqueue_live_input(&manager, "missing", "x")
            .await
            .expect("missing")
            .is_none());
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
