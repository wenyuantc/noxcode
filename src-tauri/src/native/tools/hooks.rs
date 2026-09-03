//! 生命周期钩子。
//!
//! 事件：`session_start` / `user_prompt_submit` / `pre_tool_use` / `post_tool_use` /
//! `post_tool_use_failure` / `permission_request` / `stop`。处理器：`command`（shell，
//! 载荷在环境变量 `NATIVE_HOOK_PAYLOAD`）、`http`（POST JSON）、`agent`（一次性只读子
//! Agent 判定）。处理器输出 JSON 时按下面的协议解释，否则当普通文本：
//!
//! ```json
//! { "decision": "allow" | "deny" | "ask" | "block", "reason": "...",
//!   "updated_input": { ... }, "additional_context": "...", "continue": true }
//! ```
//!
//! 兼容 Claude Code 的 `hookSpecificOutput.permissionDecision / updatedInput /
//! additionalContext`、`stopReason`。`command` 处理器退出码 2 等价于 `block`。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::db::models::NativeHook;
use crate::native::settings::{
    hook_matches, HOOK_EVENT_PERMISSION_REQUEST, HOOK_EVENT_POST_TOOL_USE,
    HOOK_EVENT_POST_TOOL_USE_FAILURE, HOOK_EVENT_PRE_TOOL_USE, HOOK_EVENT_SESSION_START,
    HOOK_EVENT_STOP, HOOK_EVENT_USER_PROMPT_SUBMIT, HOOK_HANDLER_AGENT, HOOK_HANDLER_HTTP,
};

use super::cancel::CancelFlag;
use super::local::{CommandStatus, LocalWorkspace};
use super::permission::NativeToolRiskKind;
use super::ssh::SshToolRuntime;

/// `agent` 处理器：拿到判定提示词与载荷 JSON，返回模型文本。
pub type HookAgentHandler = Arc<
    dyn Fn(String, String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// 钩子执行所需的依赖，从 `ToolCtx` 派生。
pub struct HookRuntime<'a> {
    pub workspace: &'a LocalWorkspace,
    pub ssh: Option<&'a SshToolRuntime>,
    pub cancel: &'a CancelFlag,
    pub hooks: &'a [NativeHook],
    pub extra_env: &'a [(String, String)],
    pub session_record_id: &'a str,
    pub agent_handler: Option<&'a HookAgentHandler>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Deny,
    Ask,
}

/// 单个钩子的解析结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutcome {
    pub hook_id: String,
    /// `block` / `deny` / 退出码 2：阻断当前动作。
    pub blocked: bool,
    pub reason: Option<String>,
    pub decision: Option<HookDecision>,
    pub updated_input: Option<Value>,
    pub additional_context: Option<String>,
    pub continue_run: Option<bool>,
    /// 钩子自身失败（超时、非零退出、HTTP 错误）；对非阻断事件只当警告。
    pub failure: Option<String>,
    pub output: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreToolHookResult {
    pub updated_arguments: Option<String>,
    pub additional_context: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostToolHookResult {
    pub warnings: Vec<String>,
    pub additional_context: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopHookResult {
    /// 钩子要求继续时的理由；`None` 表示可以结束。
    pub continue_reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPermissionDecision {
    pub decision: HookDecision,
    pub reason: Option<String>,
    pub hook_id: String,
}

fn hook_arguments_value(arguments: &str) -> Value {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(arguments.to_string()))
}

fn base_payload(rt: &HookRuntime<'_>, event: &str) -> Value {
    let workspace_path = rt
        .ssh
        .map(|runtime| runtime.root.clone())
        .unwrap_or_else(|| rt.workspace.root.to_string_lossy().into_owned());
    json!({
        "event": event,
        "session_record_id": rt.session_record_id,
        "workspace": workspace_path,
        "cwd": workspace_path,
        "remote": rt.ssh.is_some(),
    })
}

fn tool_payload(rt: &HookRuntime<'_>, event: &str, tool_name: &str, arguments: &str) -> Value {
    let mut payload = base_payload(rt, event);
    payload["tool_name"] = json!(tool_name);
    let args = hook_arguments_value(arguments);
    payload["arguments"] = args.clone();
    payload["tool_input"] = args;
    payload
}

/// 旧签名保留：预工具载荷（给现有单测与外部调用）。
pub fn hook_payload(event: &str, tool_name: &str, arguments: &str, workspace: &str) -> String {
    json!({
        "event": event,
        "tool_name": tool_name,
        "arguments": hook_arguments_value(arguments),
        "tool_input": hook_arguments_value(arguments),
        "workspace": workspace,
        "cwd": workspace,
    })
    .to_string()
}

fn matching_hooks<'a>(
    hooks: &'a [NativeHook],
    event: &str,
    tool_name: Option<&str>,
) -> Vec<&'a NativeHook> {
    hooks
        .iter()
        .filter(|hook| hook.enabled && hook.event == event)
        .filter(|hook| match tool_name {
            Some(name) => hook_matches(&hook.matcher, name),
            None => true,
        })
        .collect()
}

/// 解析处理器输出。JSON 对象按协议取字段，其余作为纯文本。
pub fn parse_hook_output(hook_id: &str, text: &str, exit_code: Option<i32>) -> HookOutcome {
    let mut outcome = HookOutcome {
        hook_id: hook_id.to_string(),
        output: text.trim().to_string(),
        ..HookOutcome::default()
    };
    if exit_code == Some(2) {
        outcome.blocked = true;
        outcome.reason = Some(if outcome.output.is_empty() {
            "钩子返回退出码 2".to_string()
        } else {
            outcome.output.clone()
        });
    }
    let Some(value) = extract_json_object(text) else {
        return outcome;
    };
    let specific = value.get("hookSpecificOutput");
    let decision_text = value
        .get("decision")
        .and_then(Value::as_str)
        .or_else(|| {
            specific
                .and_then(|item| item.get("permissionDecision"))
                .and_then(Value::as_str)
        })
        .map(|item| item.trim().to_ascii_lowercase());
    match decision_text.as_deref() {
        Some("allow") | Some("approve") => outcome.decision = Some(HookDecision::Allow),
        Some("deny") => {
            outcome.decision = Some(HookDecision::Deny);
            outcome.blocked = true;
        }
        Some("block") => outcome.blocked = true,
        Some("ask") => outcome.decision = Some(HookDecision::Ask),
        _ => {}
    }
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .or_else(|| value.get("stopReason").and_then(Value::as_str))
        .or_else(|| {
            specific
                .and_then(|item| item.get("permissionDecisionReason"))
                .and_then(Value::as_str)
        })
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    if reason.is_some() {
        outcome.reason = reason;
    }
    let updated = value
        .get("updated_input")
        .or_else(|| value.get("updatedInput"))
        .or_else(|| specific.and_then(|item| item.get("updatedInput")))
        .filter(|item| item.is_object())
        .cloned();
    if updated.is_some() {
        outcome.updated_input = updated;
    }
    let context = value
        .get("additional_context")
        .or_else(|| value.get("additionalContext"))
        .or_else(|| specific.and_then(|item| item.get("additionalContext")))
        .and_then(Value::as_str)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    if context.is_some() {
        outcome.additional_context = context;
    }
    if let Some(flag) = value.get("continue").and_then(Value::as_bool) {
        outcome.continue_run = Some(flag);
    }
    outcome
}

/// 从输出里找第一个顶层 JSON 对象（允许前后夹杂日志）。
fn extract_json_object(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value.is_object().then_some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Value>(&trimmed[start..=end])
        .ok()
        .filter(Value::is_object)
}

async fn run_command_handler(
    rt: &HookRuntime<'_>,
    hook: &NativeHook,
    payload: &str,
) -> Result<CommandStatus, String> {
    let timeout_ms = i64::from(hook.timeout_secs) * 1000;
    if let Some(ssh) = rt.ssh {
        let command = format!(
            "export NATIVE_HOOK_PAYLOAD={}; {}",
            crate::app::ssh::shell::shell_escape_single_quoted(payload),
            hook.command
        );
        match tokio::time::timeout(
            Duration::from_millis(timeout_ms.max(1) as u64),
            ssh.bash_with_status(&command),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Ok(CommandStatus {
                exit_code: -1,
                output: "Bash 超时".to_string(),
                timed_out: true,
            }),
        }
    } else {
        let mut env: Vec<(String, String)> = rt.extra_env.to_vec();
        env.push(("NATIVE_HOOK_PAYLOAD".to_string(), payload.to_string()));
        rt.workspace
            .bash_with_status(&hook.command, Some(timeout_ms), rt.cancel, &env)
            .await
    }
}

async fn run_http_handler(hook: &NativeHook, payload: &Value) -> Result<(u16, String), String> {
    let url = hook
        .url
        .as_deref()
        .ok_or_else(|| "http 钩子缺少 url".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(hook.timeout_secs.max(1) as u64))
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("请求钩子失败: {error}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    Ok((status, text))
}

/// 执行一条钩子，返回统一结果；处理器失败记入 `failure`。
async fn run_one(rt: &HookRuntime<'_>, hook: &NativeHook, payload: &Value) -> HookOutcome {
    let payload_text = payload.to_string();
    match hook.handler_type.as_str() {
        HOOK_HANDLER_HTTP => match run_http_handler(hook, payload).await {
            Ok((status, text)) if (200..300).contains(&status) => {
                parse_hook_output(&hook.id, &text, None)
            }
            Ok((status, text)) => HookOutcome {
                hook_id: hook.id.clone(),
                failure: Some(format!("HTTP {status}: {}", text.trim())),
                output: text,
                ..HookOutcome::default()
            },
            Err(error) => HookOutcome {
                hook_id: hook.id.clone(),
                failure: Some(error),
                ..HookOutcome::default()
            },
        },
        HOOK_HANDLER_AGENT => {
            let Some(handler) = rt.agent_handler else {
                return HookOutcome {
                    hook_id: hook.id.clone(),
                    failure: Some("当前会话没有可用于 agent 钩子的模型".to_string()),
                    ..HookOutcome::default()
                };
            };
            let prompt = hook.agent_prompt.clone().unwrap_or_default();
            let call = handler(prompt, payload_text);
            match tokio::time::timeout(Duration::from_secs(hook.timeout_secs.max(1) as u64), call)
                .await
            {
                Ok(Ok(text)) => parse_hook_output(&hook.id, &text, None),
                Ok(Err(error)) => HookOutcome {
                    hook_id: hook.id.clone(),
                    failure: Some(error),
                    ..HookOutcome::default()
                },
                Err(_) => HookOutcome {
                    hook_id: hook.id.clone(),
                    failure: Some("agent 钩子超时".to_string()),
                    ..HookOutcome::default()
                },
            }
        }
        _ => match run_command_handler(rt, hook, &payload_text).await {
            Ok(status) if status.timed_out => HookOutcome {
                hook_id: hook.id.clone(),
                failure: Some("钩子超时".to_string()),
                output: status.output,
                ..HookOutcome::default()
            },
            Ok(status) => {
                let mut outcome =
                    parse_hook_output(&hook.id, &status.output, Some(status.exit_code));
                if status.exit_code != 0 && status.exit_code != 2 {
                    outcome.failure = Some(format!("退出码 {}", status.exit_code));
                }
                outcome
            }
            Err(error) => HookOutcome {
                hook_id: hook.id.clone(),
                failure: Some(error),
                ..HookOutcome::default()
            },
        },
    }
}

async fn run_event(
    rt: &HookRuntime<'_>,
    event: &str,
    tool_name: Option<&str>,
    payload: &Value,
) -> Vec<HookOutcome> {
    let mut outcomes = Vec::new();
    for hook in matching_hooks(rt.hooks, event, tool_name) {
        if rt.cancel.is_cancelled() {
            break;
        }
        outcomes.push(run_one(rt, hook, payload).await);
    }
    outcomes
}

fn failure_warning(outcome: &HookOutcome) -> Option<String> {
    outcome
        .failure
        .as_ref()
        .map(|failure| match outcome.output.trim() {
            "" => format!("钩子 {} 失败（{failure}）", outcome.hook_id),
            detail => format!("钩子 {}：{detail}", outcome.hook_id),
        })
}

/// 工具执行前：阻断返回 `Err`；可改写参数、补充上下文。
pub async fn run_pre_tool_hooks(
    rt: &HookRuntime<'_>,
    tool_name: &str,
    arguments: &str,
) -> Result<PreToolHookResult, String> {
    let payload = tool_payload(rt, HOOK_EVENT_PRE_TOOL_USE, tool_name, arguments);
    let mut result = PreToolHookResult::default();
    for outcome in run_event(rt, HOOK_EVENT_PRE_TOOL_USE, Some(tool_name), &payload).await {
        if let Some(failure) = &outcome.failure {
            if failure == "钩子超时" {
                return Err(format!("钩子超时：{}", outcome.hook_id));
            }
        }
        if outcome.blocked {
            let reason = outcome
                .reason
                .clone()
                .filter(|item| !item.trim().is_empty())
                .unwrap_or_else(|| outcome.hook_id.clone());
            return Err(format!("钩子阻断：{reason}"));
        }
        if let Some(updated) = &outcome.updated_input {
            result.updated_arguments = Some(updated.to_string());
        }
        if let Some(context) = &outcome.additional_context {
            result.additional_context.push(context.clone());
        }
    }
    Ok(result)
}

/// 工具成功后：失败只作警告，附加上下文回填给模型。
pub async fn run_post_tool_hooks(
    rt: &HookRuntime<'_>,
    tool_name: &str,
    arguments: &str,
    output: &str,
) -> PostToolHookResult {
    let mut payload = tool_payload(rt, HOOK_EVENT_POST_TOOL_USE, tool_name, arguments);
    payload["tool_output"] = json!(truncate_for_payload(output));
    let mut result = PostToolHookResult::default();
    for outcome in run_event(rt, HOOK_EVENT_POST_TOOL_USE, Some(tool_name), &payload).await {
        if let Some(warning) = failure_warning(&outcome) {
            result.warnings.push(warning);
        }
        if outcome.blocked {
            result.warnings.push(format!(
                "钩子 {}：{}",
                outcome.hook_id,
                outcome.reason.clone().unwrap_or_default()
            ));
        }
        if let Some(context) = &outcome.additional_context {
            result.additional_context.push(context.clone());
        }
    }
    result
}

/// 工具失败后：只收集警告，不影响错误本身。
pub async fn run_post_tool_failure_hooks(
    rt: &HookRuntime<'_>,
    tool_name: &str,
    arguments: &str,
    error: &str,
) -> Vec<String> {
    let mut payload = tool_payload(rt, HOOK_EVENT_POST_TOOL_USE_FAILURE, tool_name, arguments);
    payload["error"] = json!(truncate_for_payload(error));
    run_event(
        rt,
        HOOK_EVENT_POST_TOOL_USE_FAILURE,
        Some(tool_name),
        &payload,
    )
    .await
    .iter()
    .filter_map(failure_warning)
    .collect()
}

/// 权限确认前：第一个给出 allow / deny / ask 的钩子生效。
pub async fn run_permission_request_hooks(
    rt: &HookRuntime<'_>,
    tool_name: &str,
    arguments: &str,
    kind: NativeToolRiskKind,
    summary: &str,
) -> Option<HookPermissionDecision> {
    let mut payload = tool_payload(rt, HOOK_EVENT_PERMISSION_REQUEST, tool_name, arguments);
    payload["permission"] = json!({
        "kind": serde_json::to_value(kind).unwrap_or(Value::Null),
        "summary": summary,
    });
    for outcome in run_event(rt, HOOK_EVENT_PERMISSION_REQUEST, Some(tool_name), &payload).await {
        let decision = match outcome.decision {
            Some(decision) => decision,
            None if outcome.blocked => HookDecision::Deny,
            None => continue,
        };
        return Some(HookPermissionDecision {
            decision,
            reason: outcome.reason.clone(),
            hook_id: outcome.hook_id.clone(),
        });
    }
    None
}

/// 用户提交提示词时：阻断返回 `Err(reason)`，否则返回要附加的上下文。
pub async fn run_user_prompt_submit_hooks(
    rt: &HookRuntime<'_>,
    prompt: &str,
) -> Result<Vec<String>, String> {
    let mut payload = base_payload(rt, HOOK_EVENT_USER_PROMPT_SUBMIT);
    payload["prompt"] = json!(prompt);
    let mut context = Vec::new();
    for outcome in run_event(rt, HOOK_EVENT_USER_PROMPT_SUBMIT, None, &payload).await {
        if outcome.blocked {
            return Err(outcome
                .reason
                .clone()
                .unwrap_or_else(|| format!("钩子 {} 阻断了本次输入", outcome.hook_id)));
        }
        if let Some(extra) = &outcome.additional_context {
            context.push(extra.clone());
        } else if outcome.failure.is_none() && !outcome.output.is_empty() {
            // 纯文本输出视为附加上下文。
            if extract_json_object(&outcome.output).is_none() {
                context.push(outcome.output.clone());
            }
        }
    }
    Ok(context)
}

/// 会话开始：钩子输出（JSON 的 additional_context 或纯文本）注入系统提示尾段。
pub async fn run_session_start_hooks(rt: &HookRuntime<'_>) -> Vec<String> {
    let payload = base_payload(rt, HOOK_EVENT_SESSION_START);
    let mut context = Vec::new();
    for outcome in run_event(rt, HOOK_EVENT_SESSION_START, None, &payload).await {
        if let Some(extra) = &outcome.additional_context {
            context.push(extra.clone());
        } else if outcome.failure.is_none()
            && !outcome.output.is_empty()
            && extract_json_object(&outcome.output).is_none()
        {
            context.push(outcome.output.clone());
        }
    }
    context
}

/// 回合结束：`continue: true` 或 `block` 要求模型继续；只对主 Agent 生效。
pub async fn run_stop_hooks(rt: &HookRuntime<'_>, final_text: &str) -> StopHookResult {
    let mut payload = base_payload(rt, HOOK_EVENT_STOP);
    payload["final_text"] = json!(truncate_for_payload(final_text));
    let mut result = StopHookResult::default();
    for outcome in run_event(rt, HOOK_EVENT_STOP, None, &payload).await {
        if let Some(warning) = failure_warning(&outcome) {
            result.warnings.push(warning);
            continue;
        }
        let wants_continue = outcome.continue_run == Some(true) || outcome.blocked;
        if wants_continue && result.continue_reason.is_none() {
            result.continue_reason = Some(
                outcome
                    .reason
                    .clone()
                    .filter(|item| !item.trim().is_empty())
                    .unwrap_or_else(|| format!("钩子 {} 要求继续工作", outcome.hook_id)),
            );
        }
    }
    result
}

fn truncate_for_payload(text: &str) -> String {
    const LIMIT: usize = 20_000;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    let mut end = LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::settings::normalize_native_hooks;
    use std::path::PathBuf;

    fn temp_workspace() -> LocalWorkspace {
        let root = std::env::temp_dir().join(format!(
            "noxcode-hooks-{}",
            crate::native::artifacts::unique_suffix()
        ));
        std::fs::create_dir_all(&root).expect("mkdir");
        LocalWorkspace::new(root)
    }

    fn runtime<'a>(
        workspace: &'a LocalWorkspace,
        cancel: &'a CancelFlag,
        hooks: &'a [NativeHook],
    ) -> HookRuntime<'a> {
        HookRuntime {
            workspace,
            ssh: None,
            cancel,
            hooks,
            extra_env: &[],
            session_record_id: "sess-1",
            agent_handler: None,
        }
    }

    #[test]
    fn matching_respects_event_and_tool() {
        let hooks = normalize_native_hooks(vec![NativeHook::shell(
            "1",
            HOOK_EVENT_PRE_TOOL_USE,
            "Bash,ApplyPatch",
            "true",
            30,
            true,
        )]);
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_PRE_TOOL_USE, Some("Bash")).len(),
            1
        );
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_PRE_TOOL_USE, Some("Write")).len(),
            0
        );
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_POST_TOOL_USE, Some("Bash")).len(),
            0
        );
    }

    #[test]
    fn payload_parses_tool_arguments_as_json() {
        let payload = hook_payload(
            HOOK_EVENT_PRE_TOOL_USE,
            "Write",
            r#"{"file_path":"a.txt","content":"x"}"#,
            "/tmp/ws",
        );
        let value: Value = serde_json::from_str(&payload).expect("json");
        assert_eq!(value["event"], HOOK_EVENT_PRE_TOOL_USE);
        assert_eq!(value["tool_name"], "Write");
        assert_eq!(value["workspace"], "/tmp/ws");
        assert_eq!(value["arguments"]["file_path"], "a.txt");
        assert_eq!(value["tool_input"]["content"], "x");
        let raw = hook_payload(HOOK_EVENT_PRE_TOOL_USE, "Bash", "not-json", "/tmp/ws");
        let raw_value: Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(raw_value["arguments"], "not-json");
    }

    #[test]
    fn parses_protocol_and_claude_code_shapes() {
        let plain = parse_hook_output("h", "just text", Some(0));
        assert!(!plain.blocked);
        assert_eq!(plain.output, "just text");
        let exit2 = parse_hook_output("h", "nope", Some(2));
        assert!(exit2.blocked);
        assert_eq!(exit2.reason.as_deref(), Some("nope"));
        let deny = parse_hook_output(
            "h",
            r#"log line {"decision":"deny","reason":"no prod","additional_context":"ctx"}"#,
            Some(0),
        );
        assert_eq!(deny.decision, Some(HookDecision::Deny));
        assert!(deny.blocked);
        assert_eq!(deny.reason.as_deref(), Some("no prod"));
        assert_eq!(deny.additional_context.as_deref(), Some("ctx"));
        let cc = parse_hook_output(
            "h",
            r#"{"hookSpecificOutput":{"permissionDecision":"allow","permissionDecisionReason":"ok","updatedInput":{"command":"ls"}},"continue":false}"#,
            Some(0),
        );
        assert_eq!(cc.decision, Some(HookDecision::Allow));
        assert_eq!(cc.reason.as_deref(), Some("ok"));
        assert_eq!(
            cc.updated_input
                .as_ref()
                .and_then(|v| v["command"].as_str()),
            Some("ls")
        );
        assert_eq!(cc.continue_run, Some(false));
        let stop = parse_hook_output(
            "h",
            r#"{"continue":true,"stopReason":"tests missing"}"#,
            Some(0),
        );
        assert_eq!(stop.continue_run, Some(true));
        assert_eq!(stop.reason.as_deref(), Some("tests missing"));
    }

    #[tokio::test]
    async fn pre_hook_can_rewrite_input_and_post_hook_adds_context() {
        let workspace = temp_workspace();
        let cancel = CancelFlag::new();
        let hooks = normalize_native_hooks(vec![
            NativeHook::shell(
                "rewrite",
                HOOK_EVENT_PRE_TOOL_USE,
                "Bash",
                r#"printf '{"updated_input":{"command":"echo safe"},"additional_context":"pre-ctx"}'"#,
                60,
                true,
            ),
            NativeHook::shell(
                "post",
                HOOK_EVENT_POST_TOOL_USE,
                "*",
                // 载荷经环境变量传入；这里确认它是 JSON 对象后回填上下文。
                r#"case "$NATIVE_HOOK_PAYLOAD" in "{"*) printf '{"additional_context":"seen payload"}';; *) exit 1;; esac"#,
                60,
                true,
            ),
            NativeHook::shell("failing", HOOK_EVENT_POST_TOOL_USE, "*", "exit 3", 10, true),
        ]);
        let rt = runtime(&workspace, &cancel, &hooks);
        let pre = run_pre_tool_hooks(&rt, "Bash", r#"{"command":"rm -rf /"}"#)
            .await
            .expect("pre");
        assert_eq!(
            pre.updated_arguments.as_deref(),
            Some(r#"{"command":"echo safe"}"#)
        );
        assert_eq!(pre.additional_context, vec!["pre-ctx".to_string()]);
        let post = run_post_tool_hooks(&rt, "Bash", "{}", "output").await;
        assert_eq!(post.additional_context, vec!["seen payload".to_string()]);
        assert_eq!(post.warnings.len(), 1);
        assert!(post.warnings[0].contains("failing"));
        let _ = std::fs::remove_dir_all(&workspace.root);
    }

    #[tokio::test]
    async fn permission_prompt_stop_and_session_start_hooks() {
        let workspace = temp_workspace();
        let cancel = CancelFlag::new();
        let hooks = normalize_native_hooks(vec![
            NativeHook::shell(
                "perm",
                HOOK_EVENT_PERMISSION_REQUEST,
                "Bash",
                r#"printf '{"decision":"allow","reason":"trusted"}'"#,
                60,
                true,
            ),
            NativeHook::shell(
                "prompt",
                HOOK_EVENT_USER_PROMPT_SUBMIT,
                "*",
                "printf 'remember the style guide'",
                60,
                true,
            ),
            NativeHook::shell(
                "start",
                HOOK_EVENT_SESSION_START,
                "*",
                r#"printf '{"additional_context":"branch is main"}'"#,
                60,
                true,
            ),
            NativeHook::shell(
                "stop",
                HOOK_EVENT_STOP,
                "*",
                r#"printf '{"continue":true,"reason":"run the tests"}'"#,
                60,
                true,
            ),
        ]);
        let rt = runtime(&workspace, &cancel, &hooks);
        let decision = run_permission_request_hooks(
            &rt,
            "Bash",
            r#"{"command":"git push"}"#,
            NativeToolRiskKind::Push,
            "推送",
        )
        .await
        .expect("decision");
        assert_eq!(decision.decision, HookDecision::Allow);
        assert_eq!(decision.reason.as_deref(), Some("trusted"));
        assert!(run_permission_request_hooks(
            &rt,
            "Write",
            "{}",
            NativeToolRiskKind::Overwrite,
            "x"
        )
        .await
        .is_none());
        let prompt_context = run_user_prompt_submit_hooks(&rt, "hi")
            .await
            .expect("prompt");
        assert_eq!(prompt_context, vec!["remember the style guide".to_string()]);
        let start = run_session_start_hooks(&rt).await;
        assert_eq!(start, vec!["branch is main".to_string()]);
        let stop = run_stop_hooks(&rt, "done").await;
        assert_eq!(
            stop.continue_reason.as_deref(),
            Some("run the tests"),
            "warnings: {:?}",
            stop.warnings
        );
        let _ = std::fs::remove_dir_all(&workspace.root);
    }

    #[tokio::test]
    async fn user_prompt_hook_can_block_and_agent_hook_uses_handler() {
        let workspace = temp_workspace();
        let cancel = CancelFlag::new();
        let mut agent = NativeHook::shell("agent", HOOK_EVENT_STOP, "*", "", 60, true);
        agent.handler_type = HOOK_HANDLER_AGENT.to_string();
        agent.agent_prompt = Some("是否完成？".to_string());
        let hooks = normalize_native_hooks(vec![
            NativeHook::shell(
                "block",
                HOOK_EVENT_USER_PROMPT_SUBMIT,
                "*",
                "printf 'contains secret' >&2; exit 2",
                60,
                true,
            ),
            agent,
        ]);
        let handler: HookAgentHandler = Arc::new(|prompt, payload| {
            Box::pin(async move {
                assert!(prompt.contains("完成"));
                assert!(payload.contains("\"event\":\"stop\""));
                Ok(r#"{"continue":true,"reason":"agent says more"}"#.to_string())
            })
        });
        let rt = HookRuntime {
            workspace: &workspace,
            ssh: None,
            cancel: &cancel,
            hooks: &hooks,
            extra_env: &[],
            session_record_id: "sess-2",
            agent_handler: Some(&handler),
        };
        let blocked = run_user_prompt_submit_hooks(&rt, "leak").await.unwrap_err();
        assert!(blocked.contains("contains secret"));
        let stop = run_stop_hooks(&rt, "done").await;
        assert_eq!(stop.continue_reason.as_deref(), Some("agent says more"));
        let _ = std::fs::remove_dir_all(PathBuf::from(&workspace.root));
    }
}
