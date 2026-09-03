#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::db::models::{NativeHook, NativeSettings, UpdateNativeSettings};

const SETTINGS_FILE_NAME: &str = "native-settings.json";
pub const DEFAULT_NATIVE_MAX_TURNS: i32 = 40;
pub const DEFAULT_NATIVE_MAX_SUBAGENT_TURNS: i32 = 20;
const MAX_NATIVE_MAX_TURNS: i32 = 500;
pub const DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS: i32 = 1;
const MAX_NATIVE_MAX_CONCURRENT_SUBAGENTS: i32 = 16;
/// Keep normal coding turns well below the provider's advertised context
/// window. The runner can still compact and continue when this threshold is
/// reached.
pub const DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS: i32 = 128_000;
const MIN_NATIVE_CONTEXT_WINDOW_TOKENS: i32 = 8_000;
const MAX_NATIVE_CONTEXT_WINDOW_TOKENS: i32 = 1_000_000;
/// A rollout budget is shared by the parent and all child agents. Zero keeps
/// the legacy unlimited behavior for users who explicitly opt out.
pub const DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET: i64 = 10_000_000;
const MAX_NATIVE_ROLLOUT_TOKEN_BUDGET: i64 = 100_000_000;
pub const DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS: i32 = 4_096;
const MIN_NATIVE_MAX_TOOL_OUTPUT_TOKENS: i32 = 256;
const MAX_NATIVE_MAX_TOOL_OUTPUT_TOKENS: i32 = 65_536;
pub const DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS: i32 = 300;
const MAX_NATIVE_PERMISSION_TIMEOUT_SECS: i32 = 86_400;
pub const DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS: i32 = 7;
const MAX_NATIVE_CHECKPOINT_RETENTION_DAYS: i32 = 365;
pub const DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 40;
const MIN_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 5;
const MAX_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 100;
pub const SUBAGENT_POLICY_CONSERVATIVE: &str = "conservative";
pub const SUBAGENT_POLICY_BALANCED: &str = "balanced";
pub const SUBAGENT_POLICY_AGGRESSIVE: &str = "aggressive";
pub const DEFAULT_NATIVE_SUBAGENT_POLICY: &str = SUBAGENT_POLICY_CONSERVATIVE;
/// 权限模式（对齐 ZCode 的 default / edit / build / yolo；plan 是会话态不落盘）。
pub const PERMISSION_MODE_DEFAULT: &str = "default";
pub const PERMISSION_MODE_EDIT: &str = "edit";
pub const PERMISSION_MODE_BUILD: &str = "build";
pub const PERMISSION_MODE_YOLO: &str = "yolo";
pub const PERMISSION_MODE_PLAN: &str = "plan";
/// 旧文件里的名字，读入时映射到新模式。
pub const LEGACY_PERMISSION_MODE_CONFIRM: &str = "confirm";
pub const LEGACY_PERMISSION_MODE_AUTO_EDIT: &str = "auto_edit";
pub const LEGACY_PERMISSION_MODE_FULL: &str = "full";
pub const DEFAULT_NATIVE_PERMISSION_MODE: &str = PERMISSION_MODE_DEFAULT;
const MAX_NATIVE_HOOKS: usize = 32;
const DEFAULT_NATIVE_HOOK_TIMEOUT_SECS: i32 = 30;
const MAX_NATIVE_HOOK_TIMEOUT_SECS: i32 = 120;
pub const HOOK_EVENT_SESSION_START: &str = "session_start";
pub const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "user_prompt_submit";
pub const HOOK_EVENT_PRE_TOOL_USE: &str = "pre_tool_use";
pub const HOOK_EVENT_POST_TOOL_USE: &str = "post_tool_use";
pub const HOOK_EVENT_POST_TOOL_USE_FAILURE: &str = "post_tool_use_failure";
pub const HOOK_EVENT_PERMISSION_REQUEST: &str = "permission_request";
pub const HOOK_EVENT_STOP: &str = "stop";
pub const HOOK_EVENTS: &[&str] = &[
    HOOK_EVENT_SESSION_START,
    HOOK_EVENT_USER_PROMPT_SUBMIT,
    HOOK_EVENT_PRE_TOOL_USE,
    HOOK_EVENT_POST_TOOL_USE,
    HOOK_EVENT_POST_TOOL_USE_FAILURE,
    HOOK_EVENT_PERMISSION_REQUEST,
    HOOK_EVENT_STOP,
];
pub const HOOK_HANDLER_COMMAND: &str = "command";
pub const HOOK_HANDLER_HTTP: &str = "http";
pub const HOOK_HANDLER_AGENT: &str = "agent";
pub const HOOK_SOURCE_GLOBAL: &str = "global";
pub const HOOK_SOURCE_WORKSPACE: &str = "workspace";
pub const HOOK_SOURCE_PLUGIN: &str = "plugin";

/// 接受本项目的 snake_case 与 Claude Code 的 PascalCase 事件名。
pub fn normalize_hook_event(event: &str) -> Option<&'static str> {
    match event.trim() {
        HOOK_EVENT_SESSION_START | "SessionStart" => Some(HOOK_EVENT_SESSION_START),
        HOOK_EVENT_USER_PROMPT_SUBMIT | "UserPromptSubmit" => Some(HOOK_EVENT_USER_PROMPT_SUBMIT),
        HOOK_EVENT_PRE_TOOL_USE | "PreToolUse" => Some(HOOK_EVENT_PRE_TOOL_USE),
        HOOK_EVENT_POST_TOOL_USE | "PostToolUse" => Some(HOOK_EVENT_POST_TOOL_USE),
        HOOK_EVENT_POST_TOOL_USE_FAILURE | "PostToolUseFailure" => {
            Some(HOOK_EVENT_POST_TOOL_USE_FAILURE)
        }
        HOOK_EVENT_PERMISSION_REQUEST | "PermissionRequest" => Some(HOOK_EVENT_PERMISSION_REQUEST),
        HOOK_EVENT_STOP | "Stop" => Some(HOOK_EVENT_STOP),
        _ => None,
    }
}

pub fn normalize_hook_handler_type(value: &str) -> &'static str {
    match value.trim() {
        HOOK_HANDLER_HTTP => HOOK_HANDLER_HTTP,
        HOOK_HANDLER_AGENT => HOOK_HANDLER_AGENT,
        _ => HOOK_HANDLER_COMMAND,
    }
}

pub fn normalize_hook_source(value: &str) -> &'static str {
    match value.trim() {
        HOOK_SOURCE_WORKSPACE => HOOK_SOURCE_WORKSPACE,
        HOOK_SOURCE_PLUGIN => HOOK_SOURCE_PLUGIN,
        _ => HOOK_SOURCE_GLOBAL,
    }
}
pub const DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS: i32 = 7;
const MAX_NATIVE_ARTIFACT_RETENTION_DAYS: i32 = 365;
pub const DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES: i32 = 6;
const MAX_NATIVE_MODEL_RETRY_MAX_RETRIES: i32 = 20;
pub const DEFAULT_NATIVE_MODEL_RETRY_BASE_DELAY_MS: i32 = 1_000;
const MIN_NATIVE_MODEL_RETRY_BASE_DELAY_MS: i32 = 100;
const MAX_NATIVE_MODEL_RETRY_BASE_DELAY_MS: i32 = 60_000;
pub const DEFAULT_NATIVE_MODEL_RETRY_MAX_DELAY_MS: i32 = 30_000;
const MAX_NATIVE_MODEL_RETRY_MAX_DELAY_MS: i32 = 300_000;
pub const DEFAULT_NATIVE_MODEL_RETRY_BACKOFF_FACTOR: f64 = 2.0;
const MIN_NATIVE_MODEL_RETRY_BACKOFF_FACTOR: f64 = 1.0;
const MAX_NATIVE_MODEL_RETRY_BACKOFF_FACTOR: f64 = 4.0;
pub const DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS: i32 = 120;
const MAX_NATIVE_BASH_DEFAULT_TIMEOUT_SECS: i32 = 600;
pub const DEFAULT_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT: i32 = 85;
pub const DEFAULT_NATIVE_MEMORY_DREAM_INTERVAL: i32 = 10;
const MAX_NATIVE_MEMORY_DREAM_INTERVAL: i32 = 1000;
const MIN_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT: i32 = 30;
const MAX_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT: i32 = 99;

#[derive(Debug, Default, Deserialize, Serialize)]
struct RawNativeSettings {
    #[serde(default)]
    artifact_retention_days: Option<i32>,
    #[serde(default)]
    model_retry_max_retries: Option<i32>,
    #[serde(default)]
    model_retry_base_delay_ms: Option<i32>,
    #[serde(default)]
    model_retry_max_delay_ms: Option<i32>,
    #[serde(default)]
    model_retry_backoff_factor: Option<f64>,
    #[serde(default)]
    bash_default_timeout_secs: Option<i32>,
    #[serde(default)]
    shell_snapshot_enabled: Option<bool>,
    #[serde(default)]
    rg_sidecar_enabled: Option<bool>,
    #[serde(default)]
    auto_compact_threshold_percent: Option<i32>,
    #[serde(default)]
    microcompact_enabled: Option<bool>,
    #[serde(default)]
    memory_enabled: Option<bool>,
    #[serde(default)]
    memory_dream_interval: Option<i32>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    max_subagent_turns: Option<i32>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirm_high_risk: Option<bool>,
    #[serde(default)]
    max_concurrent_subagents: Option<i32>,
    #[serde(default)]
    subagent_policy: Option<String>,
    #[serde(default)]
    context_window_tokens: Option<i32>,
    #[serde(default)]
    use_custom_context_window: Option<bool>,
    #[serde(default)]
    rollout_token_budget: Option<i64>,
    #[serde(default)]
    max_tool_output_tokens: Option<i32>,
    #[serde(default)]
    permission_timeout_secs: Option<i32>,
    #[serde(default)]
    subagent_budget_share_percent: Option<i32>,
    #[serde(default)]
    auto_checkpoint_after_tool_call: Option<bool>,
    #[serde(default)]
    checkpoint_retention_days: Option<i32>,
    #[serde(default)]
    desktop_notifications: Option<bool>,
    #[serde(default)]
    hooks: Option<Vec<NativeHook>>,
    #[serde(default)]
    global_prompt_template: Option<String>,
}

pub fn normalize_native_max_turns(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_MAX_TURNS).contains(&value) => value,
        _ => DEFAULT_NATIVE_MAX_TURNS,
    }
}

pub fn normalize_native_max_subagent_turns(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_MAX_TURNS).contains(&value) => value,
        _ => DEFAULT_NATIVE_MAX_SUBAGENT_TURNS,
    }
}

pub fn normalize_native_max_concurrent_subagents(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (1..=MAX_NATIVE_MAX_CONCURRENT_SUBAGENTS).contains(&value) => value,
        _ => DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS,
    }
}

pub fn normalize_native_context_window_tokens(value: Option<i32>) -> i32 {
    match value {
        Some(value)
            if (MIN_NATIVE_CONTEXT_WINDOW_TOKENS..=MAX_NATIVE_CONTEXT_WINDOW_TOKENS)
                .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS,
    }
}

pub fn normalize_native_rollout_token_budget(value: Option<i64>) -> i64 {
    match value {
        Some(value) if (0..=MAX_NATIVE_ROLLOUT_TOKEN_BUDGET).contains(&value) => value,
        _ => DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET,
    }
}

pub fn normalize_native_permission_timeout_secs(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_PERMISSION_TIMEOUT_SECS).contains(&value) => value,
        _ => DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS,
    }
}

pub fn normalize_native_checkpoint_retention_days(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_CHECKPOINT_RETENTION_DAYS).contains(&value) => value,
        _ => DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS,
    }
}

pub fn normalize_native_artifact_retention_days(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_ARTIFACT_RETENTION_DAYS).contains(&value) => value,
        _ => DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS,
    }
}

pub fn normalize_native_model_retry_max_retries(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_MODEL_RETRY_MAX_RETRIES).contains(&value) => value,
        _ => DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES,
    }
}

pub fn normalize_native_model_retry_base_delay_ms(value: Option<i32>) -> i32 {
    match value {
        Some(value)
            if (MIN_NATIVE_MODEL_RETRY_BASE_DELAY_MS..=MAX_NATIVE_MODEL_RETRY_BASE_DELAY_MS)
                .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_MODEL_RETRY_BASE_DELAY_MS,
    }
}

pub fn normalize_native_model_retry_max_delay_ms(value: Option<i32>, base_delay_ms: i32) -> i32 {
    match value {
        Some(value) if (base_delay_ms..=MAX_NATIVE_MODEL_RETRY_MAX_DELAY_MS).contains(&value) => {
            value
        }
        _ => DEFAULT_NATIVE_MODEL_RETRY_MAX_DELAY_MS.max(base_delay_ms),
    }
}

pub fn normalize_native_model_retry_backoff_factor(value: Option<f64>) -> f64 {
    match value {
        Some(value)
            if value.is_finite()
                && (MIN_NATIVE_MODEL_RETRY_BACKOFF_FACTOR
                    ..=MAX_NATIVE_MODEL_RETRY_BACKOFF_FACTOR)
                    .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_MODEL_RETRY_BACKOFF_FACTOR,
    }
}

pub fn normalize_native_auto_compact_threshold_percent(value: Option<i32>) -> i32 {
    match value {
        Some(value)
            if (MIN_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT
                ..=MAX_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT)
                .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT,
    }
}

pub fn normalize_native_memory_dream_interval(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (0..=MAX_NATIVE_MEMORY_DREAM_INTERVAL).contains(&value) => value,
        _ => DEFAULT_NATIVE_MEMORY_DREAM_INTERVAL,
    }
}

pub fn normalize_native_bash_default_timeout_secs(value: Option<i32>) -> i32 {
    match value {
        Some(value) if (1..=MAX_NATIVE_BASH_DEFAULT_TIMEOUT_SECS).contains(&value) => value,
        _ => DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS,
    }
}

pub fn normalize_native_subagent_budget_share_percent(value: Option<i32>) -> i32 {
    match value {
        Some(value)
            if (MIN_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT
                ..=MAX_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT)
                .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT,
    }
}

pub fn normalize_native_max_tool_output_tokens(value: Option<i32>) -> i32 {
    match value {
        Some(value)
            if (MIN_NATIVE_MAX_TOOL_OUTPUT_TOKENS..=MAX_NATIVE_MAX_TOOL_OUTPUT_TOKENS)
                .contains(&value) =>
        {
            value
        }
        _ => DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS,
    }
}

pub fn normalize_subagent_policy(value: Option<&str>) -> String {
    match value.map(str::trim).unwrap_or("") {
        SUBAGENT_POLICY_CONSERVATIVE => SUBAGENT_POLICY_CONSERVATIVE.to_string(),
        SUBAGENT_POLICY_AGGRESSIVE => SUBAGENT_POLICY_AGGRESSIVE.to_string(),
        SUBAGENT_POLICY_BALANCED => SUBAGENT_POLICY_BALANCED.to_string(),
        _ => DEFAULT_NATIVE_SUBAGENT_POLICY.to_string(),
    }
}

pub fn subagent_policy_label_zh(policy: &str) -> &'static str {
    match policy {
        SUBAGENT_POLICY_CONSERVATIVE => "保守",
        SUBAGENT_POLICY_AGGRESSIVE => "积极",
        _ => "均衡",
    }
}

/// 归一化模式名：接受本项目旧名（confirm / auto_edit / full）与 Claude Code 名
/// （acceptEdits / auto / bypassPermissions / dontAsk），输出 default / edit / build / yolo。
pub fn normalize_permission_mode(value: Option<&str>) -> String {
    match value.map(str::trim).unwrap_or("") {
        PERMISSION_MODE_EDIT
        | LEGACY_PERMISSION_MODE_AUTO_EDIT
        | "acceptEdits"
        | "accept_edits"
        | "autoEdit" => PERMISSION_MODE_EDIT.to_string(),
        PERMISSION_MODE_BUILD | "auto" => PERMISSION_MODE_BUILD.to_string(),
        PERMISSION_MODE_YOLO
        | LEGACY_PERMISSION_MODE_FULL
        | "bypassPermissions"
        | "bypass_permissions"
        | "dontAsk"
        | "dont_ask" => PERMISSION_MODE_YOLO.to_string(),
        _ => DEFAULT_NATIVE_PERMISSION_MODE.to_string(),
    }
}

pub fn permission_mode_label_zh(mode: &str) -> &'static str {
    match normalize_permission_mode(Some(mode)).as_str() {
        PERMISSION_MODE_EDIT => "自动编辑",
        PERMISSION_MODE_BUILD => "自动构建",
        PERMISSION_MODE_YOLO => "完全访问",
        _ => "变更前确认",
    }
}

/// edit / build / yolo 都自动放行覆盖文件。
pub fn permission_mode_auto_approves_edits(mode: &str) -> bool {
    matches!(
        normalize_permission_mode(Some(mode)).as_str(),
        PERMISSION_MODE_EDIT | PERMISSION_MODE_BUILD | PERMISSION_MODE_YOLO
    )
}

/// build 与 yolo 放行不透明 shell 与只读 MCP。
pub fn permission_mode_auto_approves_build(mode: &str) -> bool {
    matches!(
        normalize_permission_mode(Some(mode)).as_str(),
        PERMISSION_MODE_BUILD | PERMISSION_MODE_YOLO
    )
}

pub fn permission_mode_is_yolo(mode: &str) -> bool {
    normalize_permission_mode(Some(mode)) == PERMISSION_MODE_YOLO
}

fn resolve_permission_mode(
    permission_mode: Option<String>,
    confirm_high_risk: Option<bool>,
) -> String {
    if let Some(mode) = permission_mode {
        let trimmed = mode.trim();
        if !trimmed.is_empty() {
            return normalize_permission_mode(Some(trimmed));
        }
    }
    match confirm_high_risk {
        Some(false) => PERMISSION_MODE_YOLO.to_string(),
        _ => DEFAULT_NATIVE_PERMISSION_MODE.to_string(),
    }
}

fn app_config_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|error| format!("无法读取应用配置目录: {error}"))
}

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(SETTINGS_FILE_NAME))
}

fn default_settings() -> NativeSettings {
    NativeSettings {
        max_turns: DEFAULT_NATIVE_MAX_TURNS,
        max_subagent_turns: DEFAULT_NATIVE_MAX_SUBAGENT_TURNS,
        permission_mode: DEFAULT_NATIVE_PERMISSION_MODE.to_string(),
        max_concurrent_subagents: DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS,
        subagent_policy: DEFAULT_NATIVE_SUBAGENT_POLICY.to_string(),
        context_window_tokens: DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS,
        use_custom_context_window: false,
        rollout_token_budget: DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET,
        max_tool_output_tokens: DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS,
        permission_timeout_secs: DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS,
        subagent_budget_share_percent: DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT,
        auto_checkpoint_after_tool_call: true,
        checkpoint_retention_days: DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS,
        desktop_notifications: true,
        artifact_retention_days: DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS,
        model_retry_max_retries: DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES,
        model_retry_base_delay_ms: DEFAULT_NATIVE_MODEL_RETRY_BASE_DELAY_MS,
        model_retry_max_delay_ms: DEFAULT_NATIVE_MODEL_RETRY_MAX_DELAY_MS,
        model_retry_backoff_factor: DEFAULT_NATIVE_MODEL_RETRY_BACKOFF_FACTOR,
        bash_default_timeout_secs: DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS,
        shell_snapshot_enabled: true,
        rg_sidecar_enabled: true,
        auto_compact_threshold_percent: DEFAULT_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT,
        microcompact_enabled: true,
        memory_enabled: true,
        memory_dream_interval: DEFAULT_NATIVE_MEMORY_DREAM_INTERVAL,
        hooks: Vec::new(),
        global_prompt_template: String::new(),
    }
}

fn normalize_settings(raw: RawNativeSettings) -> NativeSettings {
    let model_retry_base_delay_ms =
        normalize_native_model_retry_base_delay_ms(raw.model_retry_base_delay_ms);
    NativeSettings {
        artifact_retention_days: normalize_native_artifact_retention_days(
            raw.artifact_retention_days,
        ),
        model_retry_max_retries: normalize_native_model_retry_max_retries(
            raw.model_retry_max_retries,
        ),
        model_retry_base_delay_ms,
        model_retry_max_delay_ms: normalize_native_model_retry_max_delay_ms(
            raw.model_retry_max_delay_ms,
            model_retry_base_delay_ms,
        ),
        model_retry_backoff_factor: normalize_native_model_retry_backoff_factor(
            raw.model_retry_backoff_factor,
        ),
        bash_default_timeout_secs: normalize_native_bash_default_timeout_secs(
            raw.bash_default_timeout_secs,
        ),
        shell_snapshot_enabled: raw.shell_snapshot_enabled.unwrap_or(true),
        rg_sidecar_enabled: raw.rg_sidecar_enabled.unwrap_or(true),
        auto_compact_threshold_percent: normalize_native_auto_compact_threshold_percent(
            raw.auto_compact_threshold_percent,
        ),
        microcompact_enabled: raw.microcompact_enabled.unwrap_or(true),
        memory_enabled: raw.memory_enabled.unwrap_or(true),
        memory_dream_interval: normalize_native_memory_dream_interval(raw.memory_dream_interval),
        max_turns: normalize_native_max_turns(raw.max_turns),
        max_subagent_turns: normalize_native_max_subagent_turns(raw.max_subagent_turns),
        permission_mode: resolve_permission_mode(raw.permission_mode, raw.confirm_high_risk),
        max_concurrent_subagents: normalize_native_max_concurrent_subagents(
            raw.max_concurrent_subagents,
        ),
        subagent_policy: normalize_subagent_policy(raw.subagent_policy.as_deref()),
        context_window_tokens: normalize_native_context_window_tokens(raw.context_window_tokens),
        use_custom_context_window: raw.use_custom_context_window.unwrap_or(false),
        rollout_token_budget: normalize_native_rollout_token_budget(raw.rollout_token_budget),
        max_tool_output_tokens: normalize_native_max_tool_output_tokens(raw.max_tool_output_tokens),
        permission_timeout_secs: normalize_native_permission_timeout_secs(
            raw.permission_timeout_secs,
        ),
        subagent_budget_share_percent: normalize_native_subagent_budget_share_percent(
            raw.subagent_budget_share_percent,
        ),
        auto_checkpoint_after_tool_call: raw.auto_checkpoint_after_tool_call.unwrap_or(true),
        checkpoint_retention_days: normalize_native_checkpoint_retention_days(
            raw.checkpoint_retention_days,
        ),
        desktop_notifications: raw.desktop_notifications.unwrap_or(true),
        hooks: normalize_native_hooks(raw.hooks.unwrap_or_default()),
        global_prompt_template: raw
            .global_prompt_template
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

pub fn load_native_settings<R: Runtime>(app: &AppHandle<R>) -> Result<NativeSettings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(default_settings());
    }
    let text =
        fs::read_to_string(&path).map_err(|error| format!("读取内置 Agent 设置失败: {error}"))?;
    let raw = serde_json::from_str::<RawNativeSettings>(&text).unwrap_or_default();
    Ok(normalize_settings(raw))
}

pub fn effective_max_turns<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| settings.max_turns.max(0) as u32)
        .unwrap_or(DEFAULT_NATIVE_MAX_TURNS as u32)
}

pub fn effective_max_subagent_turns<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| settings.max_subagent_turns.max(0) as u32)
        .unwrap_or(DEFAULT_NATIVE_MAX_SUBAGENT_TURNS as u32)
}

fn save_native_settings<R: Runtime>(
    app: &AppHandle<R>,
    settings: &NativeSettings,
) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建内置 Agent 设置目录失败: {error}"))?;
    }
    let base_delay =
        normalize_native_model_retry_base_delay_ms(Some(settings.model_retry_base_delay_ms));
    let raw = RawNativeSettings {
        artifact_retention_days: Some(normalize_native_artifact_retention_days(Some(
            settings.artifact_retention_days,
        ))),
        model_retry_max_retries: Some(normalize_native_model_retry_max_retries(Some(
            settings.model_retry_max_retries,
        ))),
        model_retry_base_delay_ms: Some(base_delay),
        model_retry_max_delay_ms: Some(normalize_native_model_retry_max_delay_ms(
            Some(settings.model_retry_max_delay_ms),
            base_delay,
        )),
        model_retry_backoff_factor: Some(normalize_native_model_retry_backoff_factor(Some(
            settings.model_retry_backoff_factor,
        ))),
        bash_default_timeout_secs: Some(normalize_native_bash_default_timeout_secs(Some(
            settings.bash_default_timeout_secs,
        ))),
        shell_snapshot_enabled: Some(settings.shell_snapshot_enabled),
        rg_sidecar_enabled: Some(settings.rg_sidecar_enabled),
        auto_compact_threshold_percent: Some(normalize_native_auto_compact_threshold_percent(
            Some(settings.auto_compact_threshold_percent),
        )),
        microcompact_enabled: Some(settings.microcompact_enabled),
        memory_enabled: Some(settings.memory_enabled),
        memory_dream_interval: Some(normalize_native_memory_dream_interval(Some(
            settings.memory_dream_interval,
        ))),
        max_turns: Some(normalize_native_max_turns(Some(settings.max_turns))),
        max_subagent_turns: Some(normalize_native_max_subagent_turns(Some(
            settings.max_subagent_turns,
        ))),
        permission_mode: Some(normalize_permission_mode(Some(
            settings.permission_mode.as_str(),
        ))),
        confirm_high_risk: None,
        max_concurrent_subagents: Some(normalize_native_max_concurrent_subagents(Some(
            settings.max_concurrent_subagents,
        ))),
        subagent_policy: Some(normalize_subagent_policy(Some(
            settings.subagent_policy.as_str(),
        ))),
        context_window_tokens: Some(normalize_native_context_window_tokens(Some(
            settings.context_window_tokens,
        ))),
        use_custom_context_window: Some(settings.use_custom_context_window),
        rollout_token_budget: Some(normalize_native_rollout_token_budget(Some(
            settings.rollout_token_budget,
        ))),
        max_tool_output_tokens: Some(normalize_native_max_tool_output_tokens(Some(
            settings.max_tool_output_tokens,
        ))),
        permission_timeout_secs: Some(normalize_native_permission_timeout_secs(Some(
            settings.permission_timeout_secs,
        ))),
        subagent_budget_share_percent: Some(normalize_native_subagent_budget_share_percent(Some(
            settings.subagent_budget_share_percent,
        ))),
        auto_checkpoint_after_tool_call: Some(settings.auto_checkpoint_after_tool_call),
        checkpoint_retention_days: Some(normalize_native_checkpoint_retention_days(Some(
            settings.checkpoint_retention_days,
        ))),
        desktop_notifications: Some(settings.desktop_notifications),
        hooks: Some(settings.hooks.clone()),
        global_prompt_template: Some(settings.global_prompt_template.clone()),
    };
    let json = serde_json::to_string_pretty(&raw)
        .map_err(|error| format!("序列化内置 Agent 设置失败: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("写入内置 Agent 设置失败: {error}"))
}

fn max_turns_activity_details(max_turns: i32) -> String {
    if max_turns == 0 {
        "内置 Agent 最大工具轮次：不限制".to_string()
    } else {
        format!("内置 Agent 最大工具轮次：{max_turns}")
    }
}

fn max_subagent_turns_activity_details(max_subagent_turns: i32) -> String {
    if max_subagent_turns == 0 {
        "子 Agent 最大工具轮次：不限制".to_string()
    } else {
        format!("子 Agent 最大工具轮次：{max_subagent_turns}")
    }
}

async fn merge_native_settings<R: Runtime>(
    app: &AppHandle<R>,
    updates: UpdateNativeSettings,
) -> Result<NativeSettings, String> {
    let previous = load_native_settings(app)?;
    let mut next = previous.clone();
    if let Some(max_turns) = updates.max_turns {
        next.max_turns = normalize_native_max_turns(Some(max_turns));
    }
    if let Some(max_subagent_turns) = updates.max_subagent_turns {
        next.max_subagent_turns = normalize_native_max_subagent_turns(Some(max_subagent_turns));
    }
    if let Some(permission_mode) = updates.permission_mode {
        next.permission_mode = normalize_permission_mode(Some(permission_mode.as_str()));
    }
    if let Some(max_concurrent_subagents) = updates.max_concurrent_subagents {
        next.max_concurrent_subagents =
            normalize_native_max_concurrent_subagents(Some(max_concurrent_subagents));
    }
    if let Some(subagent_policy) = updates.subagent_policy {
        next.subagent_policy = normalize_subagent_policy(Some(subagent_policy.as_str()));
    }
    if let Some(context_window_tokens) = updates.context_window_tokens {
        next.context_window_tokens =
            normalize_native_context_window_tokens(Some(context_window_tokens));
    }
    if let Some(use_custom_context_window) = updates.use_custom_context_window {
        next.use_custom_context_window = use_custom_context_window;
    }
    if let Some(rollout_token_budget) = updates.rollout_token_budget {
        next.rollout_token_budget =
            normalize_native_rollout_token_budget(Some(rollout_token_budget));
    }
    if let Some(max_tool_output_tokens) = updates.max_tool_output_tokens {
        next.max_tool_output_tokens =
            normalize_native_max_tool_output_tokens(Some(max_tool_output_tokens));
    }
    if let Some(permission_timeout_secs) = updates.permission_timeout_secs {
        next.permission_timeout_secs =
            normalize_native_permission_timeout_secs(Some(permission_timeout_secs));
    }
    if let Some(subagent_budget_share_percent) = updates.subagent_budget_share_percent {
        next.subagent_budget_share_percent =
            normalize_native_subagent_budget_share_percent(Some(subagent_budget_share_percent));
    }
    if let Some(auto_checkpoint_after_tool_call) = updates.auto_checkpoint_after_tool_call {
        next.auto_checkpoint_after_tool_call = auto_checkpoint_after_tool_call;
    }
    if let Some(checkpoint_retention_days) = updates.checkpoint_retention_days {
        next.checkpoint_retention_days =
            normalize_native_checkpoint_retention_days(Some(checkpoint_retention_days));
    }
    if let Some(desktop_notifications) = updates.desktop_notifications {
        next.desktop_notifications = desktop_notifications;
    }
    if let Some(artifact_retention_days) = updates.artifact_retention_days {
        next.artifact_retention_days =
            normalize_native_artifact_retention_days(Some(artifact_retention_days));
    }
    if let Some(model_retry_max_retries) = updates.model_retry_max_retries {
        next.model_retry_max_retries =
            normalize_native_model_retry_max_retries(Some(model_retry_max_retries));
    }
    if let Some(model_retry_base_delay_ms) = updates.model_retry_base_delay_ms {
        next.model_retry_base_delay_ms =
            normalize_native_model_retry_base_delay_ms(Some(model_retry_base_delay_ms));
    }
    if let Some(model_retry_max_delay_ms) = updates.model_retry_max_delay_ms {
        next.model_retry_max_delay_ms = normalize_native_model_retry_max_delay_ms(
            Some(model_retry_max_delay_ms),
            next.model_retry_base_delay_ms,
        );
    }
    next.model_retry_max_delay_ms = normalize_native_model_retry_max_delay_ms(
        Some(next.model_retry_max_delay_ms),
        next.model_retry_base_delay_ms,
    );
    if let Some(model_retry_backoff_factor) = updates.model_retry_backoff_factor {
        next.model_retry_backoff_factor =
            normalize_native_model_retry_backoff_factor(Some(model_retry_backoff_factor));
    }
    if let Some(bash_default_timeout_secs) = updates.bash_default_timeout_secs {
        next.bash_default_timeout_secs =
            normalize_native_bash_default_timeout_secs(Some(bash_default_timeout_secs));
    }
    if let Some(shell_snapshot_enabled) = updates.shell_snapshot_enabled {
        next.shell_snapshot_enabled = shell_snapshot_enabled;
    }
    if let Some(rg_sidecar_enabled) = updates.rg_sidecar_enabled {
        next.rg_sidecar_enabled = rg_sidecar_enabled;
    }
    if let Some(auto_compact_threshold_percent) = updates.auto_compact_threshold_percent {
        next.auto_compact_threshold_percent =
            normalize_native_auto_compact_threshold_percent(Some(auto_compact_threshold_percent));
    }
    if let Some(microcompact_enabled) = updates.microcompact_enabled {
        next.microcompact_enabled = microcompact_enabled;
    }
    if let Some(memory_enabled) = updates.memory_enabled {
        next.memory_enabled = memory_enabled;
    }
    if let Some(memory_dream_interval) = updates.memory_dream_interval {
        next.memory_dream_interval =
            normalize_native_memory_dream_interval(Some(memory_dream_interval));
    }
    if let Some(hooks) = updates.hooks {
        next.hooks = normalize_native_hooks(hooks);
    }
    if let Some(global_prompt_template) = updates.global_prompt_template {
        next.global_prompt_template = global_prompt_template.trim().to_string();
    }
    save_native_settings(app, &next)?;
    Ok(next)
}

fn native_settings_activity_details(settings: &NativeSettings) -> String {
    format!(
        "{}；{}；权限模式：{}；确认超时：{}；同轮子 Agent 上限：{}；子 Agent 策略：{}；子 Agent 预算占比：{}%；上下文窗口：{} token；自定义上下文：{}；会话预算：{}；单条工具结果：{} token；工具后自动检查点：{}；检查点保留：{} 天；桌面通知：{}；钩子：{} 条",
        max_turns_activity_details(settings.max_turns),
        max_subagent_turns_activity_details(settings.max_subagent_turns),
        permission_mode_label_zh(&settings.permission_mode),
        if settings.permission_timeout_secs == 0 {
            "不超时".to_string()
        } else {
            format!("{} 秒", settings.permission_timeout_secs)
        },
        settings.max_concurrent_subagents,
        subagent_policy_label_zh(&settings.subagent_policy),
        settings.subagent_budget_share_percent,
        settings.context_window_tokens,
        if settings.use_custom_context_window {
            "开"
        } else {
            "关"
        },
        if settings.rollout_token_budget == 0 {
            "不限制".to_string()
        } else {
            format!("{} token", settings.rollout_token_budget)
        },
        settings.max_tool_output_tokens,
        if settings.auto_checkpoint_after_tool_call {
            "开"
        } else {
            "关"
        },
        settings.checkpoint_retention_days,
        if settings.desktop_notifications {
            "开"
        } else {
            "关"
        },
        settings.hooks.len()
    )
}

/// 由设置得到模型层重试配置（指数退避 + 抖动）。
pub fn model_retry_config(settings: &NativeSettings) -> crate::native::model::RetryConfig {
    crate::native::model::RetryConfig {
        max_retries: settings.model_retry_max_retries.max(0) as u32,
        base_delay_ms: settings.model_retry_base_delay_ms.max(1) as u64,
        max_delay_ms: settings
            .model_retry_max_delay_ms
            .max(settings.model_retry_base_delay_ms)
            .max(1) as u64,
        backoff_factor: settings.model_retry_backoff_factor,
        jitter: true,
    }
}

pub fn effective_model_retry_config<R: Runtime>(
    app: &AppHandle<R>,
) -> crate::native::model::RetryConfig {
    load_native_settings(app)
        .map(|settings| model_retry_config(&settings))
        .unwrap_or_default()
}

pub fn effective_subagent_policy<R: Runtime>(app: &AppHandle<R>) -> String {
    load_native_settings(app)
        .map(|settings| normalize_subagent_policy(Some(settings.subagent_policy.as_str())))
        .unwrap_or_else(|_| DEFAULT_NATIVE_SUBAGENT_POLICY.to_string())
}

pub fn effective_max_concurrent_subagents<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| settings.max_concurrent_subagents.max(1) as u32)
        .unwrap_or(DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS as u32)
}

pub fn effective_context_window_tokens<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| {
            settings
                .context_window_tokens
                .max(MIN_NATIVE_CONTEXT_WINDOW_TOKENS) as u32
        })
        .unwrap_or(DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as u32)
}

pub fn resolve_session_context_window_tokens(
    use_custom: bool,
    configured_tokens: u32,
    model_context_tokens: Option<u32>,
) -> u32 {
    let model = model_context_tokens.filter(|value| *value > 0);
    if use_custom {
        let configured = configured_tokens.max(1);
        match model {
            Some(model) => configured.min(model),
            None => configured,
        }
    } else {
        model
            .unwrap_or(DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as u32)
            .max(1)
    }
}

pub fn session_context_window_tokens<R: Runtime>(
    app: &AppHandle<R>,
    model_context_tokens: Option<u32>,
) -> u32 {
    match load_native_settings(app) {
        Ok(settings) => resolve_session_context_window_tokens(
            settings.use_custom_context_window,
            settings
                .context_window_tokens
                .max(MIN_NATIVE_CONTEXT_WINDOW_TOKENS) as u32,
            model_context_tokens,
        ),
        Err(_) => resolve_session_context_window_tokens(
            false,
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as u32,
            model_context_tokens,
        ),
    }
}

pub fn effective_rollout_token_budget<R: Runtime>(app: &AppHandle<R>) -> u64 {
    load_native_settings(app)
        .map(|settings| settings.rollout_token_budget.max(0) as u64)
        .unwrap_or(DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET as u64)
}

pub fn effective_max_tool_output_tokens<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| {
            settings
                .max_tool_output_tokens
                .max(MIN_NATIVE_MAX_TOOL_OUTPUT_TOKENS) as u32
        })
        .unwrap_or(DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS as u32)
}

pub fn effective_permission_timeout_secs<R: Runtime>(app: &AppHandle<R>) -> u64 {
    load_native_settings(app)
        .map(|settings| settings.permission_timeout_secs.max(0) as u64)
        .unwrap_or(DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS as u64)
}

pub fn effective_subagent_budget_share_percent<R: Runtime>(app: &AppHandle<R>) -> u32 {
    load_native_settings(app)
        .map(|settings| {
            settings.subagent_budget_share_percent.clamp(
                MIN_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT,
                MAX_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT,
            ) as u32
        })
        .unwrap_or(DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT as u32)
}

pub fn effective_permission_mode<R: Runtime>(app: &AppHandle<R>) -> String {
    load_native_settings(app)
        .map(|settings| normalize_permission_mode(Some(settings.permission_mode.as_str())))
        .unwrap_or_else(|_| DEFAULT_NATIVE_PERMISSION_MODE.to_string())
}

pub fn hook_matches(matcher: &str, tool_name: &str) -> bool {
    let matcher = matcher.trim();
    if matcher.is_empty() || matcher == "*" {
        return true;
    }
    matcher
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .any(|item| item.eq_ignore_ascii_case(tool_name))
}

pub fn normalize_native_hooks(hooks: Vec<NativeHook>) -> Vec<NativeHook> {
    let mut out = Vec::new();
    for (index, hook) in hooks.into_iter().take(MAX_NATIVE_HOOKS).enumerate() {
        let handler_type = normalize_hook_handler_type(&hook.handler_type).to_string();
        let command = hook.command.trim().to_string();
        let url = hook
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let agent_prompt = hook
            .agent_prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        // 每种处理器都要有自己的执行目标，否则视为无效条目。
        let valid = match handler_type.as_str() {
            HOOK_HANDLER_HTTP => url
                .as_deref()
                .is_some_and(|value| value.starts_with("http://") || value.starts_with("https://")),
            HOOK_HANDLER_AGENT => agent_prompt.is_some(),
            _ => !command.is_empty(),
        };
        if !valid {
            continue;
        }
        let Some(event) = normalize_hook_event(&hook.event) else {
            continue;
        };
        let matcher = {
            let matcher = hook.matcher.trim();
            if matcher.is_empty() {
                "*".to_string()
            } else {
                matcher.to_string()
            }
        };
        let timeout_secs = if (1..=MAX_NATIVE_HOOK_TIMEOUT_SECS).contains(&hook.timeout_secs) {
            hook.timeout_secs
        } else {
            DEFAULT_NATIVE_HOOK_TIMEOUT_SECS
        };
        let id = if hook.id.trim().is_empty() {
            format!("hook-{}", index + 1)
        } else {
            hook.id.trim().to_string()
        };
        out.push(NativeHook {
            id,
            event: event.to_string(),
            matcher,
            command,
            timeout_secs,
            enabled: hook.enabled,
            handler_type,
            url,
            agent_prompt,
            source: normalize_hook_source(&hook.source).to_string(),
        });
    }
    out
}

#[tauri::command]
pub async fn get_native_settings<R: Runtime>(app: AppHandle<R>) -> Result<NativeSettings, String> {
    load_native_settings(&app)
}

#[tauri::command]
pub async fn update_native_settings<R: Runtime>(
    app: AppHandle<R>,
    updates: UpdateNativeSettings,
) -> Result<NativeSettings, String> {
    merge_native_settings(&app, updates).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_zero_as_unlimited() {
        assert_eq!(normalize_native_max_turns(Some(0)), 0);
    }

    #[test]
    fn normalize_keeps_default_range() {
        assert_eq!(normalize_native_max_turns(Some(40)), 40);
        assert_eq!(normalize_native_max_turns(Some(500)), 500);
        assert_eq!(normalize_native_max_subagent_turns(Some(0)), 0);
        assert_eq!(normalize_native_max_subagent_turns(Some(20)), 20);
        assert_eq!(normalize_native_max_subagent_turns(Some(80)), 80);
        assert_eq!(normalize_native_max_subagent_turns(Some(500)), 500);
    }

    #[test]
    fn missing_confirm_flag_defaults_on() {
        let settings = normalize_settings(RawNativeSettings {
            max_turns: Some(40),
            ..RawNativeSettings::default()
        });
        assert_eq!(settings.permission_mode, DEFAULT_NATIVE_PERMISSION_MODE);
        assert_eq!(
            settings.artifact_retention_days,
            DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS
        );
        assert_eq!(
            settings.model_retry_max_retries,
            DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES
        );
        assert_eq!(
            settings.model_retry_base_delay_ms,
            DEFAULT_NATIVE_MODEL_RETRY_BASE_DELAY_MS
        );
        assert_eq!(
            settings.model_retry_max_delay_ms,
            DEFAULT_NATIVE_MODEL_RETRY_MAX_DELAY_MS
        );
        assert_eq!(
            settings.bash_default_timeout_secs,
            DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS
        );
        assert!(settings.shell_snapshot_enabled);
        assert!(settings.rg_sidecar_enabled);
        assert!(settings.hooks.is_empty());
        assert!(settings.global_prompt_template.is_empty());
        assert_eq!(
            settings.max_subagent_turns,
            DEFAULT_NATIVE_MAX_SUBAGENT_TURNS
        );
        assert_eq!(
            settings.max_concurrent_subagents,
            DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS
        );
        assert_eq!(settings.subagent_policy, DEFAULT_NATIVE_SUBAGENT_POLICY);
        assert_eq!(
            settings.context_window_tokens,
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            settings.rollout_token_budget,
            DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET
        );
        assert_eq!(
            settings.max_tool_output_tokens,
            DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS
        );
        assert_eq!(
            settings.permission_timeout_secs,
            DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS
        );
        assert_eq!(
            settings.subagent_budget_share_percent,
            DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT
        );
        assert!(settings.auto_checkpoint_after_tool_call);
        assert_eq!(
            settings.checkpoint_retention_days,
            DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS
        );
        assert!(settings.desktop_notifications);
        assert!(!settings.use_custom_context_window);
    }

    #[test]
    fn retry_settings_clamp_and_keep_max_above_base() {
        let settings = normalize_settings(RawNativeSettings {
            model_retry_max_retries: Some(99),
            model_retry_base_delay_ms: Some(5_000),
            model_retry_max_delay_ms: Some(1_000),
            model_retry_backoff_factor: Some(9.0),
            bash_default_timeout_secs: Some(0),
            artifact_retention_days: Some(-1),
            ..RawNativeSettings::default()
        });
        assert_eq!(
            settings.model_retry_max_retries,
            DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES
        );
        assert_eq!(settings.model_retry_base_delay_ms, 5_000);
        assert!(settings.model_retry_max_delay_ms >= settings.model_retry_base_delay_ms);
        assert_eq!(
            settings.model_retry_backoff_factor,
            DEFAULT_NATIVE_MODEL_RETRY_BACKOFF_FACTOR
        );
        assert_eq!(
            settings.bash_default_timeout_secs,
            DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS
        );
        assert_eq!(
            settings.artifact_retention_days,
            DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS
        );
        let retry = model_retry_config(&settings);
        assert_eq!(
            retry.max_retries,
            DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES as u32
        );
        assert_eq!(retry.base_delay_ms, 5_000);
    }

    #[test]
    fn legacy_confirm_flag_maps_to_permission_mode() {
        let from_false = normalize_settings(RawNativeSettings {
            confirm_high_risk: Some(false),
            ..RawNativeSettings::default()
        });
        assert_eq!(from_false.permission_mode, PERMISSION_MODE_YOLO);

        let from_true = normalize_settings(RawNativeSettings {
            confirm_high_risk: Some(true),
            ..RawNativeSettings::default()
        });
        assert_eq!(from_true.permission_mode, PERMISSION_MODE_DEFAULT);

        let from_none = normalize_settings(RawNativeSettings::default());
        assert_eq!(from_none.permission_mode, PERMISSION_MODE_DEFAULT);

        let explicit = normalize_settings(RawNativeSettings {
            permission_mode: Some(LEGACY_PERMISSION_MODE_AUTO_EDIT.to_string()),
            confirm_high_risk: Some(false),
            ..RawNativeSettings::default()
        });
        assert_eq!(explicit.permission_mode, PERMISSION_MODE_EDIT);
    }

    #[test]
    fn normalize_permission_mode_values() {
        assert_eq!(
            normalize_permission_mode(None),
            DEFAULT_NATIVE_PERMISSION_MODE
        );
        // 本项目旧名。
        assert_eq!(
            normalize_permission_mode(Some("confirm")),
            PERMISSION_MODE_DEFAULT
        );
        assert_eq!(
            normalize_permission_mode(Some("auto_edit")),
            PERMISSION_MODE_EDIT
        );
        assert_eq!(
            normalize_permission_mode(Some("full")),
            PERMISSION_MODE_YOLO
        );
        // 新名与 Claude Code 别名。
        assert_eq!(
            normalize_permission_mode(Some("yolo")),
            PERMISSION_MODE_YOLO
        );
        assert_eq!(
            normalize_permission_mode(Some("build")),
            PERMISSION_MODE_BUILD
        );
        assert_eq!(
            normalize_permission_mode(Some("auto")),
            PERMISSION_MODE_BUILD
        );
        assert_eq!(
            normalize_permission_mode(Some("acceptEdits")),
            PERMISSION_MODE_EDIT
        );
        assert_eq!(
            normalize_permission_mode(Some("bypassPermissions")),
            PERMISSION_MODE_YOLO
        );
        assert_eq!(
            normalize_permission_mode(Some("dontAsk")),
            PERMISSION_MODE_YOLO
        );
        assert_eq!(
            normalize_permission_mode(Some("plan")),
            PERMISSION_MODE_DEFAULT
        );
        assert_eq!(permission_mode_label_zh("confirm"), "变更前确认");
        assert_eq!(permission_mode_label_zh(PERMISSION_MODE_EDIT), "自动编辑");
        assert_eq!(permission_mode_label_zh(PERMISSION_MODE_BUILD), "自动构建");
        assert_eq!(permission_mode_label_zh(PERMISSION_MODE_YOLO), "完全访问");
        assert!(permission_mode_auto_approves_edits(PERMISSION_MODE_EDIT));
        assert!(permission_mode_auto_approves_edits(PERMISSION_MODE_BUILD));
        assert!(!permission_mode_auto_approves_edits(
            PERMISSION_MODE_DEFAULT
        ));
        assert!(permission_mode_auto_approves_build(PERMISSION_MODE_BUILD));
        assert!(!permission_mode_auto_approves_build(PERMISSION_MODE_EDIT));
        assert!(permission_mode_is_yolo("full"));
    }

    #[test]
    fn normalize_subagent_policy_values() {
        assert_eq!(
            normalize_subagent_policy(None),
            DEFAULT_NATIVE_SUBAGENT_POLICY
        );
        assert_eq!(
            normalize_subagent_policy(Some("balanced")),
            SUBAGENT_POLICY_BALANCED
        );
        assert_eq!(
            normalize_subagent_policy(Some("aggressive")),
            SUBAGENT_POLICY_AGGRESSIVE
        );
        assert_eq!(
            normalize_subagent_policy(Some("conservative")),
            SUBAGENT_POLICY_CONSERVATIVE
        );
        assert_eq!(
            normalize_subagent_policy(Some("yolo")),
            DEFAULT_NATIVE_SUBAGENT_POLICY
        );
    }

    #[test]
    fn normalize_concurrent_subagents_range() {
        assert_eq!(
            normalize_native_max_concurrent_subagents(None),
            DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS
        );
        assert_eq!(normalize_native_max_concurrent_subagents(Some(3)), 3);
        assert_eq!(normalize_native_max_concurrent_subagents(Some(1)), 1);
        assert_eq!(normalize_native_max_concurrent_subagents(Some(16)), 16);
        assert_eq!(
            normalize_native_max_concurrent_subagents(None),
            DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS
        );
        assert_eq!(
            normalize_native_max_concurrent_subagents(Some(0)),
            DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS
        );
        assert_eq!(
            normalize_native_max_concurrent_subagents(Some(17)),
            DEFAULT_NATIVE_MAX_CONCURRENT_SUBAGENTS
        );
    }

    #[test]
    fn normalize_falls_back_outside_range() {
        assert_eq!(normalize_native_max_turns(None), DEFAULT_NATIVE_MAX_TURNS);
        assert_eq!(
            normalize_native_max_turns(Some(-1)),
            DEFAULT_NATIVE_MAX_TURNS
        );
        assert_eq!(
            normalize_native_max_turns(Some(501)),
            DEFAULT_NATIVE_MAX_TURNS
        );
        assert_eq!(
            normalize_native_max_subagent_turns(None),
            DEFAULT_NATIVE_MAX_SUBAGENT_TURNS
        );
        assert_eq!(
            normalize_native_max_subagent_turns(Some(-1)),
            DEFAULT_NATIVE_MAX_SUBAGENT_TURNS
        );
        assert_eq!(
            normalize_native_max_subagent_turns(Some(501)),
            DEFAULT_NATIVE_MAX_SUBAGENT_TURNS
        );
    }

    #[test]
    fn activity_details_include_subagent_turns() {
        let defaults = default_settings();
        assert!(native_settings_activity_details(&defaults).contains("子 Agent 最大工具轮次：20"));
        let mut custom = default_settings();
        custom.max_subagent_turns = 0;
        assert!(native_settings_activity_details(&custom).contains("子 Agent 最大工具轮次：不限制"));
        custom.max_subagent_turns = 80;
        assert!(native_settings_activity_details(&custom).contains("子 Agent 最大工具轮次：80"));
    }

    #[test]
    fn normalize_context_window_and_tool_limits() {
        assert_eq!(
            normalize_native_context_window_tokens(Some(DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS)),
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            normalize_native_context_window_tokens(Some(MIN_NATIVE_CONTEXT_WINDOW_TOKENS)),
            MIN_NATIVE_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            normalize_native_context_window_tokens(Some(MIN_NATIVE_CONTEXT_WINDOW_TOKENS - 1)),
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(
            normalize_native_context_window_tokens(Some(MAX_NATIVE_CONTEXT_WINDOW_TOKENS + 1)),
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS
        );

        assert_eq!(normalize_native_rollout_token_budget(Some(0)), 0);
        assert_eq!(
            normalize_native_rollout_token_budget(Some(DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET)),
            DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET
        );
        assert_eq!(
            normalize_native_rollout_token_budget(Some(-1)),
            DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET
        );
        assert_eq!(
            normalize_native_rollout_token_budget(Some(MAX_NATIVE_ROLLOUT_TOKEN_BUDGET + 1)),
            DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET
        );

        assert_eq!(
            normalize_native_max_tool_output_tokens(Some(DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS)),
            DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS
        );
        assert_eq!(
            normalize_native_max_tool_output_tokens(Some(MIN_NATIVE_MAX_TOOL_OUTPUT_TOKENS - 1)),
            DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS
        );
        assert_eq!(
            normalize_native_max_tool_output_tokens(Some(MAX_NATIVE_MAX_TOOL_OUTPUT_TOKENS + 1)),
            DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS
        );

        assert_eq!(normalize_native_permission_timeout_secs(Some(0)), 0);
        assert_eq!(
            normalize_native_permission_timeout_secs(Some(DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS)),
            DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS
        );
        assert_eq!(
            normalize_native_permission_timeout_secs(Some(-1)),
            DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS
        );
        assert_eq!(normalize_native_subagent_budget_share_percent(Some(40)), 40);
        assert_eq!(
            normalize_native_subagent_budget_share_percent(Some(4)),
            DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT
        );
        assert_eq!(
            normalize_native_subagent_budget_share_percent(Some(100)),
            100
        );
        assert_eq!(normalize_native_checkpoint_retention_days(Some(0)), 0);
        assert_eq!(normalize_native_checkpoint_retention_days(Some(365)), 365);
        assert_eq!(
            normalize_native_checkpoint_retention_days(Some(366)),
            DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS
        );
    }

    #[test]
    fn missing_checkpoint_and_notification_settings_use_safe_defaults() {
        let settings = normalize_settings(RawNativeSettings::default());
        assert!(settings.auto_checkpoint_after_tool_call);
        assert_eq!(
            settings.checkpoint_retention_days,
            DEFAULT_NATIVE_CHECKPOINT_RETENTION_DAYS
        );
        assert!(settings.desktop_notifications);
    }

    #[test]
    fn missing_custom_context_window_defaults_off() {
        let from_none = normalize_settings(RawNativeSettings::default());
        assert!(!from_none.use_custom_context_window);

        let from_json: RawNativeSettings =
            serde_json::from_str(r#"{"context_window_tokens":256000}"#).unwrap();
        let settings = normalize_settings(from_json);
        assert!(!settings.use_custom_context_window);
        assert_eq!(settings.context_window_tokens, 256_000);

        let from_null: RawNativeSettings =
            serde_json::from_str(r#"{"use_custom_context_window":null}"#).unwrap();
        assert!(!normalize_settings(from_null).use_custom_context_window);

        let from_true: RawNativeSettings =
            serde_json::from_str(r#"{"use_custom_context_window":true}"#).unwrap();
        assert!(normalize_settings(from_true).use_custom_context_window);
    }

    #[test]
    fn resolve_session_context_window_tokens_respects_toggle() {
        assert_eq!(
            resolve_session_context_window_tokens(false, 256_000, Some(1_000_000)),
            1_000_000
        );
        assert_eq!(
            resolve_session_context_window_tokens(false, 256_000, None),
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as u32
        );
        assert_eq!(
            resolve_session_context_window_tokens(false, 256_000, Some(0)),
            DEFAULT_NATIVE_CONTEXT_WINDOW_TOKENS as u32
        );
        assert_eq!(
            resolve_session_context_window_tokens(true, 256_000, Some(1_000_000)),
            256_000
        );
        assert_eq!(
            resolve_session_context_window_tokens(true, 256_000, Some(128_000)),
            128_000
        );
        assert_eq!(
            resolve_session_context_window_tokens(true, 256_000, None),
            256_000
        );
    }

    #[test]
    fn normalize_hooks_and_matcher() {
        assert!(hook_matches("*", "Bash"));
        assert!(hook_matches("Bash, Write", "write"));
        assert!(!hook_matches("Read", "Bash"));
        let mut http = NativeHook::shell("h", "PermissionRequest", "Bash", "", 20, true);
        http.handler_type = "http".to_string();
        http.url = Some(" https://hooks.example.com/permission ".to_string());
        let mut bad_http = NativeHook::shell("bad", "Stop", "*", "", 20, true);
        bad_http.handler_type = "http".to_string();
        bad_http.url = Some("ftp://nope".to_string());
        let mut agent = NativeHook::shell("a", "stop", "*", "", 20, true);
        agent.handler_type = "agent".to_string();
        agent.agent_prompt = Some("判断是否完成".to_string());
        let hooks = normalize_native_hooks(vec![
            NativeHook::shell("", "pre_tool_use", "", " echo hi ", 0, true),
            // 未知事件直接丢弃。
            NativeHook::shell("nope", "nope", "", "echo x", 10, true),
            // command 处理器缺命令也丢弃。
            NativeHook::shell("x", HOOK_EVENT_POST_TOOL_USE, "ApplyPatch", "", 15, false),
            http,
            bad_http,
            agent,
        ]);
        assert_eq!(hooks.len(), 3);
        assert_eq!(hooks[0].id, "hook-1");
        assert_eq!(hooks[0].event, HOOK_EVENT_PRE_TOOL_USE);
        assert_eq!(hooks[0].matcher, "*");
        assert_eq!(hooks[0].timeout_secs, DEFAULT_NATIVE_HOOK_TIMEOUT_SECS);
        assert_eq!(hooks[0].command, "echo hi");
        assert_eq!(hooks[0].handler_type, HOOK_HANDLER_COMMAND);
        assert_eq!(hooks[1].event, HOOK_EVENT_PERMISSION_REQUEST);
        assert_eq!(hooks[1].handler_type, HOOK_HANDLER_HTTP);
        assert_eq!(
            hooks[1].url.as_deref(),
            Some("https://hooks.example.com/permission")
        );
        assert_eq!(hooks[2].event, HOOK_EVENT_STOP);
        assert_eq!(hooks[2].handler_type, HOOK_HANDLER_AGENT);
        assert_eq!(
            normalize_hook_event("SessionStart"),
            Some(HOOK_EVENT_SESSION_START)
        );
        assert_eq!(
            normalize_hook_event("PostToolUseFailure"),
            Some(HOOK_EVENT_POST_TOOL_USE_FAILURE)
        );
        assert_eq!(
            normalize_hook_event("UserPromptSubmit"),
            Some(HOOK_EVENT_USER_PROMPT_SUBMIT)
        );
        assert_eq!(normalize_hook_event("unknown"), None);
    }
}
