#[allow(unused_imports)]
use super::*;

pub(super) struct NativeRunSettings {
    pub(super) client: ModelClient,
    pub(super) model: String,
    /// 渠道的轻量模型（压缩 / 记忆 / 钩子判定），无则用主模型。
    pub(super) lite_model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) max_output_tokens: Option<u32>,
    pub(super) thinking_enabled: bool,
    pub(super) context_tokens: Option<u32>,
    pub(super) profile_system_prompt: Option<String>,
    pub(super) protocol: String,
    pub(super) channel_id: String,
    pub(super) channel_name: String,
    pub(super) bound_subagent: Option<crate::native::subagents::NativeSubagent>,
}

pub(super) fn live_snapshot_from_run(
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

pub(super) fn sync_run_from_live(run: &mut NativeRunSettings, slot: &SharedLiveModel) {
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

pub(super) fn publish_live_run(
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

pub(super) fn configure_runner_limits(
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

pub(super) fn format_native_diagnostics(
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

pub(super) fn native_startup_banner(
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

pub(super) fn should_announce_session_startup(resume_session_id: Option<&str>) -> bool {
    resume_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
}

pub(super) fn is_cancelled_run_error(error: &str) -> bool {
    error == "已取消"
}

pub(super) fn is_mcp_error_status(text: &str) -> bool {
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
    pub(super) usage: Option<UsageDelta>,
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

pub(super) fn resolve_run_model_config(
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

pub(super) async fn load_native_client_from_channel(
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

pub(super) async fn load_native_client(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    channel_id: &str,
    model: &str,
    reasoning_effort: Option<&str>,
) -> Result<NativeRunSettings, String> {
    load_native_client_from_channel(app, pool, channel_id, model, reasoning_effort).await
}

pub(super) fn native_one_shot_text(
    message: &crate::native::model::types::Message,
) -> Result<String, String> {
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

pub(super) fn one_shot_reasoning_usable(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.contains('{') && trimmed.contains('}'))
        || trimmed.starts_with('#')
        || trimmed.contains("\n# ")
        || trimmed.contains("\n## ")
}

pub(super) async fn run_native_one_shot_with_run(
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
    let response = run
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
    let mut usage = response.usage;
    let mut message = response.complete_message()?;
    if native_one_shot_text(&message).is_err() && run.thinking_enabled {
        if let Ok(retry_response) = run
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
            let retry_usage = retry_response.usage;
            if let Ok(retry_message) = retry_response.complete_message() {
                if native_one_shot_text(&retry_message).is_ok() {
                    message = retry_message;
                    usage = retry_usage;
                }
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
