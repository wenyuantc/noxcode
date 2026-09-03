use std::time::Duration;

use crate::db::models::NativeHook;
use crate::native::settings::{hook_matches, HOOK_EVENT_POST_TOOL_USE, HOOK_EVENT_PRE_TOOL_USE};

use super::cancel::CancelFlag;
use super::local::{CommandStatus, LocalWorkspace};
use super::ssh::SshToolRuntime;

pub async fn run_pre_tool_hooks(
    workspace: &LocalWorkspace,
    ssh: Option<&SshToolRuntime>,
    cancel: &CancelFlag,
    hooks: &[NativeHook],
    tool_name: &str,
    arguments: &str,
) -> Result<(), String> {
    for hook in matching_hooks(hooks, HOOK_EVENT_PRE_TOOL_USE, tool_name) {
        let status = run_one(
            workspace,
            ssh,
            cancel,
            hook,
            HOOK_EVENT_PRE_TOOL_USE,
            tool_name,
            arguments,
        )
        .await?;
        if status.timed_out {
            return Err(format!("钩子超时：{}", hook.id));
        }
        if status.exit_code == 2 {
            let reason = status.output.trim();
            return Err(if reason.is_empty() {
                format!("钩子阻断：{}", hook.id)
            } else {
                format!("钩子阻断：{reason}")
            });
        }
    }
    Ok(())
}

pub async fn run_post_tool_hooks(
    workspace: &LocalWorkspace,
    ssh: Option<&SshToolRuntime>,
    cancel: &CancelFlag,
    hooks: &[NativeHook],
    tool_name: &str,
    arguments: &str,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for hook in matching_hooks(hooks, HOOK_EVENT_POST_TOOL_USE, tool_name) {
        match run_one(
            workspace,
            ssh,
            cancel,
            hook,
            HOOK_EVENT_POST_TOOL_USE,
            tool_name,
            arguments,
        )
        .await
        {
            Ok(status) if status.timed_out || status.exit_code != 0 => {
                let detail = status.output.trim();
                warnings.push(if detail.is_empty() {
                    format!("钩子 {} 失败（退出码 {}）", hook.id, status.exit_code)
                } else {
                    format!("钩子 {}：{detail}", hook.id)
                });
            }
            Ok(_) => {}
            Err(error) => warnings.push(format!("钩子 {}：{error}", hook.id)),
        }
    }
    warnings
}

fn hook_arguments_value(arguments: &str) -> serde_json::Value {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(trimmed)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()))
}

fn hook_payload(event: &str, tool_name: &str, arguments: &str, workspace: &str) -> String {
    serde_json::json!({
        "event": event,
        "tool_name": tool_name,
        "arguments": hook_arguments_value(arguments),
        "workspace": workspace,
    })
    .to_string()
}

fn matching_hooks<'a>(
    hooks: &'a [NativeHook],
    event: &str,
    tool_name: &str,
) -> Vec<&'a NativeHook> {
    hooks
        .iter()
        .filter(|hook| {
            hook.enabled && hook.event == event && hook_matches(&hook.matcher, tool_name)
        })
        .collect()
}

async fn run_one(
    workspace: &LocalWorkspace,
    ssh: Option<&SshToolRuntime>,
    cancel: &CancelFlag,
    hook: &NativeHook,
    event: &str,
    tool_name: &str,
    arguments: &str,
) -> Result<CommandStatus, String> {
    let workspace_path = ssh
        .map(|runtime| runtime.root.clone())
        .unwrap_or_else(|| workspace.root.to_string_lossy().into_owned());
    let payload = hook_payload(event, tool_name, arguments, &workspace_path);
    let timeout_ms = i64::from(hook.timeout_secs) * 1000;
    if let Some(ssh) = ssh {
        let command = format!(
            "export NATIVE_HOOK_PAYLOAD={}; {}",
            crate::app::ssh::shell::shell_escape_single_quoted(&payload),
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
        workspace
            .bash_with_status(
                &hook.command,
                Some(timeout_ms),
                cancel,
                &[("NATIVE_HOOK_PAYLOAD".to_string(), payload)],
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::settings::normalize_native_hooks;

    #[test]
    fn matching_respects_event_and_tool() {
        let hooks = normalize_native_hooks(vec![NativeHook {
            id: "1".to_string(),
            event: HOOK_EVENT_PRE_TOOL_USE.to_string(),
            matcher: "Bash,ApplyPatch".to_string(),
            command: "true".to_string(),
            timeout_secs: 30,
            enabled: true,
        }]);
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_PRE_TOOL_USE, "Bash").len(),
            1
        );
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_PRE_TOOL_USE, "Write").len(),
            0
        );
        assert_eq!(
            matching_hooks(&hooks, HOOK_EVENT_POST_TOOL_USE, "Bash").len(),
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
        let value: serde_json::Value = serde_json::from_str(&payload).expect("json");
        assert_eq!(value["event"], HOOK_EVENT_PRE_TOOL_USE);
        assert_eq!(value["tool_name"], "Write");
        assert_eq!(value["workspace"], "/tmp/ws");
        assert_eq!(value["arguments"]["file_path"], "a.txt");
        assert_eq!(value["arguments"]["content"], "x");
        let raw = hook_payload(HOOK_EVENT_PRE_TOOL_USE, "Bash", "not-json", "/tmp/ws");
        let raw_value: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(raw_value["arguments"], "not-json");
    }
}
