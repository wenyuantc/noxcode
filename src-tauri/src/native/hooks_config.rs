//! 工作区级钩子文件。
//!
//! - `.noxcode/hooks.json`：`{ "hooks": [ NativeHook... ] }`（或裸数组），字段与设置页一致。
//! - `.claude/settings.json` / `.claude/settings.local.json` 的 `hooks` 段（Claude Code 格式）：
//!   `{ "hooks": { "PreToolUse": [ { "matcher": "Bash|Write", "hooks": [ { "type": "command",
//!   "command": "...", "timeout": 30 } ] } ] } }`。`type: prompt` 映射为 `agent` 处理器。
//!
//! 工作区钩子只对本地工作区生效，来源标记为 `workspace`，排在全局钩子之后执行。

use std::path::Path;

use serde_json::Value;

use crate::db::models::NativeHook;
use crate::native::settings::{
    normalize_hook_event, normalize_native_hooks, HOOK_HANDLER_AGENT, HOOK_HANDLER_COMMAND,
    HOOK_HANDLER_HTTP, HOOK_SOURCE_WORKSPACE,
};

pub const WORKSPACE_HOOKS_FILE: &str = ".noxcode/hooks.json";
const CLAUDE_SETTINGS_FILES: &[&str] = &[".claude/settings.json", ".claude/settings.local.json"];

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn parse_native_hooks_file(value: &Value) -> Vec<NativeHook> {
    let items = match value {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map
            .get("hooks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<NativeHook>(item).ok())
        .collect()
}

/// Claude Code 的 matcher 是正则（`Edit|Write`）；这里只支持工具名的或运算与 `*`。
fn convert_claude_matcher(matcher: Option<&str>) -> String {
    let trimmed = matcher.map(str::trim).unwrap_or("");
    if trimmed.is_empty() || trimmed == "*" || trimmed == ".*" {
        return "*".to_string();
    }
    trimmed
        .trim_matches(|ch| ch == '^' || ch == '$' || ch == '(' || ch == ')')
        .split('|')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn parse_claude_settings_hooks(value: &Value, prefix: &str) -> Vec<NativeHook> {
    let Some(map) = value.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (event_name, groups) in map {
        let Some(event) = normalize_hook_event(event_name) else {
            continue;
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let matcher = convert_claude_matcher(group.get("matcher").and_then(Value::as_str));
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (handler_index, handler) in handlers.iter().enumerate() {
                let kind = handler
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(HOOK_HANDLER_COMMAND);
                let timeout_secs = handler
                    .get("timeout")
                    .and_then(Value::as_i64)
                    .map(|value| value.clamp(1, 120) as i32)
                    .unwrap_or(30);
                let id = format!("{prefix}-{event}-{group_index}-{handler_index}");
                let mut hook =
                    NativeHook::shell(id, event, matcher.clone(), "", timeout_secs, true);
                hook.source = HOOK_SOURCE_WORKSPACE.to_string();
                match kind {
                    "prompt" => {
                        hook.handler_type = HOOK_HANDLER_AGENT.to_string();
                        hook.agent_prompt = handler
                            .get("prompt")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                    "http" => {
                        hook.handler_type = HOOK_HANDLER_HTTP.to_string();
                        hook.url = handler
                            .get("url")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                    _ => {
                        hook.handler_type = HOOK_HANDLER_COMMAND.to_string();
                        hook.command = handler
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                    }
                }
                out.push(hook);
            }
        }
    }
    out
}

/// 读取工作区目录下的钩子；文件不存在或格式错误返回空。
pub fn load_workspace_hooks(workspace_root: &Path) -> Vec<NativeHook> {
    let mut hooks = Vec::new();
    if let Some(value) = read_json(&workspace_root.join(WORKSPACE_HOOKS_FILE)) {
        for (index, mut hook) in parse_native_hooks_file(&value).into_iter().enumerate() {
            if hook.id.trim().is_empty() {
                hook.id = format!("ws-{}", index + 1);
            }
            hook.source = HOOK_SOURCE_WORKSPACE.to_string();
            hooks.push(hook);
        }
    }
    for file in CLAUDE_SETTINGS_FILES {
        if let Some(value) = read_json(&workspace_root.join(file)) {
            let prefix = if file.contains("local") {
                "claude-local"
            } else {
                "claude"
            };
            hooks.extend(parse_claude_settings_hooks(&value, prefix));
        }
    }
    normalize_native_hooks(hooks)
}

/// 全局钩子先执行，再执行工作区钩子；同 id 以工作区为准。
pub fn merge_hooks(global: Vec<NativeHook>, workspace: Vec<NativeHook>) -> Vec<NativeHook> {
    let mut merged: Vec<NativeHook> = global
        .into_iter()
        .filter(|item| !workspace.iter().any(|ws| ws.id == item.id))
        .collect();
    merged.extend(workspace);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::settings::{HOOK_EVENT_PRE_TOOL_USE, HOOK_EVENT_STOP};

    fn temp_root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "noxcode-hooks-config-{}",
            crate::native::artifacts::unique_suffix()
        ));
        std::fs::create_dir_all(root.join(".noxcode")).expect("mkdir");
        std::fs::create_dir_all(root.join(".claude")).expect("mkdir");
        root
    }

    #[test]
    fn loads_native_and_claude_code_shapes() {
        let root = temp_root();
        std::fs::write(
            root.join(WORKSPACE_HOOKS_FILE),
            r#"{"hooks":[{"id":"","event":"stop","matcher":"*","command":"echo done","timeout_secs":5,"enabled":true}]}"#,
        )
        .expect("write");
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Edit|Write","hooks":[{"type":"command","command":"./lint.sh","timeout":10},{"type":"prompt","prompt":"检查是否安全"}]}],"Unknown":[]}}"#,
        )
        .expect("write");
        let hooks = load_workspace_hooks(&root);
        assert_eq!(hooks.len(), 3);
        assert!(hooks
            .iter()
            .all(|hook| hook.source == HOOK_SOURCE_WORKSPACE));
        let stop = hooks
            .iter()
            .find(|hook| hook.event == HOOK_EVENT_STOP)
            .expect("stop");
        assert_eq!(stop.id, "ws-1");
        assert_eq!(stop.command, "echo done");
        let pre: Vec<&NativeHook> = hooks
            .iter()
            .filter(|hook| hook.event == HOOK_EVENT_PRE_TOOL_USE)
            .collect();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0].matcher, "Edit, Write");
        assert_eq!(pre[0].command, "./lint.sh");
        assert_eq!(pre[0].timeout_secs, 10);
        assert_eq!(pre[1].handler_type, HOOK_HANDLER_AGENT);
        assert_eq!(pre[1].agent_prompt.as_deref(), Some("检查是否安全"));
        let merged = merge_hooks(
            vec![NativeHook::shell(
                "g",
                HOOK_EVENT_STOP,
                "*",
                "echo g",
                5,
                true,
            )],
            hooks.clone(),
        );
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].id, "g");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_or_invalid_files_yield_nothing() {
        let root = temp_root();
        std::fs::write(root.join(WORKSPACE_HOOKS_FILE), "not json").expect("write");
        assert!(load_workspace_hooks(&root).is_empty());
        assert_eq!(convert_claude_matcher(None), "*");
        assert_eq!(convert_claude_matcher(Some("^(Bash|Read)$")), "Bash, Read");
        let _ = std::fs::remove_dir_all(root);
    }
}
