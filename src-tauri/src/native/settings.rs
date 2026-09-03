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
pub const DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 40;
const MIN_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 5;
const MAX_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT: i32 = 100;
pub const SUBAGENT_POLICY_CONSERVATIVE: &str = "conservative";
pub const SUBAGENT_POLICY_BALANCED: &str = "balanced";
pub const SUBAGENT_POLICY_AGGRESSIVE: &str = "aggressive";
pub const DEFAULT_NATIVE_SUBAGENT_POLICY: &str = SUBAGENT_POLICY_CONSERVATIVE;
pub const PERMISSION_MODE_CONFIRM: &str = "confirm";
pub const PERMISSION_MODE_AUTO_EDIT: &str = "auto_edit";
pub const PERMISSION_MODE_FULL: &str = "full";
pub const DEFAULT_NATIVE_PERMISSION_MODE: &str = PERMISSION_MODE_CONFIRM;
const MAX_NATIVE_HOOKS: usize = 32;
const DEFAULT_NATIVE_HOOK_TIMEOUT_SECS: i32 = 30;
const MAX_NATIVE_HOOK_TIMEOUT_SECS: i32 = 120;
pub const HOOK_EVENT_PRE_TOOL_USE: &str = "pre_tool_use";
pub const HOOK_EVENT_POST_TOOL_USE: &str = "post_tool_use";

#[derive(Debug, Default, Deserialize, Serialize)]
struct RawNativeSettings {
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
    rollout_token_budget: Option<i64>,
    #[serde(default)]
    max_tool_output_tokens: Option<i32>,
    #[serde(default)]
    permission_timeout_secs: Option<i32>,
    #[serde(default)]
    subagent_budget_share_percent: Option<i32>,
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

pub fn normalize_permission_mode(value: Option<&str>) -> String {
    match value.map(str::trim).unwrap_or("") {
        PERMISSION_MODE_AUTO_EDIT => PERMISSION_MODE_AUTO_EDIT.to_string(),
        PERMISSION_MODE_FULL => PERMISSION_MODE_FULL.to_string(),
        PERMISSION_MODE_CONFIRM => PERMISSION_MODE_CONFIRM.to_string(),
        _ => DEFAULT_NATIVE_PERMISSION_MODE.to_string(),
    }
}

pub fn permission_mode_label_zh(mode: &str) -> &'static str {
    match mode {
        PERMISSION_MODE_AUTO_EDIT => "自动编辑",
        PERMISSION_MODE_FULL => "完全访问",
        _ => "变更前确认",
    }
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
        Some(false) => PERMISSION_MODE_FULL.to_string(),
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
        rollout_token_budget: DEFAULT_NATIVE_ROLLOUT_TOKEN_BUDGET,
        max_tool_output_tokens: DEFAULT_NATIVE_MAX_TOOL_OUTPUT_TOKENS,
        permission_timeout_secs: DEFAULT_NATIVE_PERMISSION_TIMEOUT_SECS,
        subagent_budget_share_percent: DEFAULT_NATIVE_SUBAGENT_BUDGET_SHARE_PERCENT,
        hooks: Vec::new(),
        global_prompt_template: String::new(),
    }
}

fn normalize_settings(raw: RawNativeSettings) -> NativeSettings {
    NativeSettings {
        max_turns: normalize_native_max_turns(raw.max_turns),
        max_subagent_turns: normalize_native_max_subagent_turns(raw.max_subagent_turns),
        permission_mode: resolve_permission_mode(raw.permission_mode, raw.confirm_high_risk),
        max_concurrent_subagents: normalize_native_max_concurrent_subagents(
            raw.max_concurrent_subagents,
        ),
        subagent_policy: normalize_subagent_policy(raw.subagent_policy.as_deref()),
        context_window_tokens: normalize_native_context_window_tokens(raw.context_window_tokens),
        rollout_token_budget: normalize_native_rollout_token_budget(raw.rollout_token_budget),
        max_tool_output_tokens: normalize_native_max_tool_output_tokens(raw.max_tool_output_tokens),
        permission_timeout_secs: normalize_native_permission_timeout_secs(
            raw.permission_timeout_secs,
        ),
        subagent_budget_share_percent: normalize_native_subagent_budget_share_percent(
            raw.subagent_budget_share_percent,
        ),
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
    let raw = RawNativeSettings {
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
        "{}；{}；权限模式：{}；确认超时：{}；同轮子 Agent 上限：{}；子 Agent 策略：{}；子 Agent 预算占比：{}%；上下文窗口：{} token；会话预算：{}；单条工具结果：{} token；钩子：{} 条",
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
        if settings.rollout_token_budget == 0 {
            "不限制".to_string()
        } else {
            format!("{} token", settings.rollout_token_budget)
        },
        settings.max_tool_output_tokens,
        settings.hooks.len()
    )
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
        let command = hook.command.trim().to_string();
        if command.is_empty() {
            continue;
        }
        let event = match hook.event.trim() {
            HOOK_EVENT_POST_TOOL_USE => HOOK_EVENT_POST_TOOL_USE.to_string(),
            _ => HOOK_EVENT_PRE_TOOL_USE.to_string(),
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
            event,
            matcher,
            command,
            timeout_secs,
            enabled: hook.enabled,
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
            max_subagent_turns: None,
            permission_mode: None,
            confirm_high_risk: None,
            max_concurrent_subagents: None,
            subagent_policy: None,
            context_window_tokens: None,
            rollout_token_budget: None,
            max_tool_output_tokens: None,
            permission_timeout_secs: None,
            subagent_budget_share_percent: None,
            hooks: None,
            global_prompt_template: None,
        });
        assert_eq!(settings.permission_mode, DEFAULT_NATIVE_PERMISSION_MODE);
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
    }

    #[test]
    fn legacy_confirm_flag_maps_to_permission_mode() {
        let from_false = normalize_settings(RawNativeSettings {
            confirm_high_risk: Some(false),
            ..RawNativeSettings::default()
        });
        assert_eq!(from_false.permission_mode, PERMISSION_MODE_FULL);

        let from_true = normalize_settings(RawNativeSettings {
            confirm_high_risk: Some(true),
            ..RawNativeSettings::default()
        });
        assert_eq!(from_true.permission_mode, PERMISSION_MODE_CONFIRM);

        let from_none = normalize_settings(RawNativeSettings::default());
        assert_eq!(from_none.permission_mode, PERMISSION_MODE_CONFIRM);

        let explicit = normalize_settings(RawNativeSettings {
            permission_mode: Some(PERMISSION_MODE_AUTO_EDIT.to_string()),
            confirm_high_risk: Some(false),
            ..RawNativeSettings::default()
        });
        assert_eq!(explicit.permission_mode, PERMISSION_MODE_AUTO_EDIT);
    }

    #[test]
    fn normalize_permission_mode_values() {
        assert_eq!(
            normalize_permission_mode(None),
            DEFAULT_NATIVE_PERMISSION_MODE
        );
        assert_eq!(
            normalize_permission_mode(Some("confirm")),
            PERMISSION_MODE_CONFIRM
        );
        assert_eq!(
            normalize_permission_mode(Some("auto_edit")),
            PERMISSION_MODE_AUTO_EDIT
        );
        assert_eq!(
            normalize_permission_mode(Some("full")),
            PERMISSION_MODE_FULL
        );
        assert_eq!(
            normalize_permission_mode(Some("yolo")),
            DEFAULT_NATIVE_PERMISSION_MODE
        );
        assert_eq!(
            permission_mode_label_zh(PERMISSION_MODE_CONFIRM),
            "变更前确认"
        );
        assert_eq!(
            permission_mode_label_zh(PERMISSION_MODE_AUTO_EDIT),
            "自动编辑"
        );
        assert_eq!(permission_mode_label_zh(PERMISSION_MODE_FULL), "完全访问");
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
    }

    #[test]
    fn normalize_hooks_and_matcher() {
        assert!(hook_matches("*", "Bash"));
        assert!(hook_matches("Bash, Write", "write"));
        assert!(!hook_matches("Read", "Bash"));
        let hooks = normalize_native_hooks(vec![
            NativeHook {
                id: String::new(),
                event: "nope".to_string(),
                matcher: String::new(),
                command: " echo hi ".to_string(),
                timeout_secs: 0,
                enabled: true,
            },
            NativeHook {
                id: "x".to_string(),
                event: HOOK_EVENT_POST_TOOL_USE.to_string(),
                matcher: "ApplyPatch".to_string(),
                command: String::new(),
                timeout_secs: 15,
                enabled: false,
            },
        ]);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].id, "hook-1");
        assert_eq!(hooks[0].event, HOOK_EVENT_PRE_TOOL_USE);
        assert_eq!(hooks[0].matcher, "*");
        assert_eq!(hooks[0].timeout_secs, DEFAULT_NATIVE_HOOK_TIMEOUT_SECS);
        assert_eq!(hooks[0].command, "echo hi");
    }
}
