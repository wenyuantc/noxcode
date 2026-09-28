use super::*;

pub(super) fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|item| item.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn thinking_duration_seconds(elapsed_ms: u64) -> u32 {
    u32::try_from(elapsed_ms.saturating_add(500) / 1000)
        .unwrap_or(u32::MAX)
        .max(1)
}

pub(super) fn thinking_start_line(content: &str, seconds: u32) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("[思考] {seconds}秒\n{trimmed}"))
}

pub(super) fn tool_start_line(name: &str, arguments: &str) -> String {
    tool_start_line_ex(name, arguments, None, None)
}

pub(super) fn tool_start_line_ex(
    name: &str,
    arguments: &str,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    match name {
        "Read" => format!("[读取] {}", json_string(&args, "file_path")),
        "SQLiteQuery" => format!("[工具] SQLiteQuery {}", json_string(&args, "file_path")),
        "Write" => format!("[写入] {}", json_string(&args, "file_path")),
        "Edit" => format!("[编辑] {}", json_string(&args, "file_path")),
        "Bash" => format!("[命令] {}", json_string(&args, "command")),
        "Glob" => format!("[工具] Glob {}", json_string(&args, "pattern")),
        "Grep" => format!("[工具] Grep {}", json_string(&args, "pattern")),
        "TodoRead" => "[待办] 读取任务清单".to_string(),
        "TodoWrite" => format_todo_write_start(&args),
        "ApplyPatch" => "[补丁] 应用多文件补丁".to_string(),
        "Skill" => format!("[技能] {}", json_string(&args, "name")),
        "Agent" => format!("[子 Agent] {}", json_string(&args, "description")),
        "WebFetch" => format!("[工具] WebFetch {}", json_string(&args, "url")),
        "WebSearch" => format!("[工具] WebSearch {}", json_string(&args, "query")),
        "AskUserQuestion" | "AskQuestion" => {
            format!("[工具] 提问 {}", first_question_prompt(&args))
        }
        "EnterPlanMode" => "[工具] EnterPlanMode".to_string(),
        "ExitPlanMode" => "[工具] ExitPlanMode".to_string(),
        "TaskOutput" => format!("[工具] TaskOutput {}", json_string(&args, "task_id")),
        "TaskStop" => format!("[工具] TaskStop {}", json_string(&args, "task_id")),
        "SendMessage" => format!("[工具] SendMessage {}", json_string(&args, "task_id")),
        "RespondToCoordinator" => "[工具] RespondToCoordinator".to_string(),
        "CronCreate" => format!(
            "[工具] CronCreate {}",
            first_of(&args, &["name", "cron", "expression"])
        ),
        "CronList" => "[工具] CronList".to_string(),
        "CronDelete" => format!("[工具] CronDelete {}", first_of(&args, &["id"])),
        "Goal" => format!("[工具] Goal {}", first_of(&args, &["title", "action"])),
        "GoalRead" => "[工具] GoalRead".to_string(),
        "ReadSessionContext" => format!(
            "[工具] ReadSessionContext {}",
            first_of(&args, &["session_id", "query"])
        ),
        "Computer" => format!(
            "[电脑控制] {}",
            crate::native::tools::desktop::parse_computer_args(arguments)
                .map(|parsed| parsed.zh_brief())
                .unwrap_or_else(|_| json_string(&args, "action"))
        ),
        other => format_generic_tool_line(other, &args, mcp_server, mcp_tool),
    }
}

fn format_generic_tool_line(
    name: &str,
    args: &Value,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> String {
    let extra = compact_args(args);
    if let (Some(server), Some(tool)) = (mcp_server, mcp_tool) {
        return if extra.is_empty() {
            format!("[MCP工具] {server} / {tool}")
        } else {
            format!("[MCP工具] {server} / {tool} {extra}")
        };
    }
    if name.starts_with("mcp_") {
        return if extra.is_empty() {
            format!("[MCP工具] {name}")
        } else {
            format!("[MCP工具] {name} {extra}")
        };
    }
    if extra.is_empty() {
        format!("[工具] {name}")
    } else {
        format!("[工具] {name} {extra}")
    }
}

pub(super) fn tool_event_title(line: &str) -> String {
    let first = line.split('\n').next().unwrap_or(line).trim();
    let Some(rest) = first.strip_prefix('[') else {
        return first.to_string();
    };
    let Some(end) = rest.find(']') else {
        return first.to_string();
    };
    let label = rest[..end].trim();
    let after = rest[end + 1..].trim();
    if after.is_empty() {
        label.to_string()
    } else {
        format!("{label} {after}")
    }
}

pub(super) fn tool_args_summary(name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    match name {
        "Read" | "SQLiteQuery" | "Write" | "Edit" => {
            json_opt(&args, "file_path").unwrap_or_default()
        }
        "Bash" => json_opt(&args, "command").unwrap_or_default(),
        "Glob" | "Grep" => json_opt(&args, "pattern").unwrap_or_default(),
        "Skill" => json_opt(&args, "name").unwrap_or_default(),
        "Agent" => json_opt(&args, "description").unwrap_or_default(),
        "WebFetch" => json_opt(&args, "url").unwrap_or_default(),
        "WebSearch" => json_opt(&args, "query").unwrap_or_default(),
        "TaskOutput" | "TaskStop" | "SendMessage" => json_opt(&args, "task_id").unwrap_or_default(),
        "Computer" => crate::native::tools::desktop::parse_computer_args(arguments)
            .map(|parsed| parsed.zh_brief())
            .unwrap_or_else(|_| json_string(&args, "action")),
        _ => compact_args(&args),
    }
}

fn first_question_prompt(args: &Value) -> String {
    args.get("questions")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("prompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, 80))
        .unwrap_or_else(|| "(unknown)".to_string())
}

fn first_of(args: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = json_opt(args, key) {
            return value;
        }
    }
    "(unknown)".to_string()
}

fn json_opt(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
}

fn compact_args(args: &Value) -> String {
    match args {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        Value::Object(map) => {
            if let Some((_, Value::String(text))) = map
                .iter()
                .find(|(_, value)| value.as_str().is_some_and(|item| !item.trim().is_empty()))
            {
                return truncate_chars(text.trim(), 120);
            }
            truncate_chars(&args.to_string(), 120)
        }
        other => truncate_chars(&other.to_string(), 120),
    }
}

fn format_todo_write_start(args: &Value) -> String {
    let Some(todos) = args.get("todos").and_then(Value::as_array) else {
        return "[待办] 更新任务清单".to_string();
    };
    if todos.is_empty() {
        return "[待办] (空)".to_string();
    }
    let lines: Vec<String> = todos.iter().filter_map(format_todo_item_line).collect();
    if lines.is_empty() {
        return "[待办] 更新任务清单".to_string();
    }
    format!("[待办]\n{}", lines.join("\n"))
}

fn format_todo_item_line(item: &Value) -> Option<String> {
    let content = item
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("pending");
    let priority = item
        .get("priority")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("medium");
    Some(format!(
        "- [{}] {} ({})",
        status,
        truncate_chars(content, 200),
        priority
    ))
}

pub(super) fn tool_result_line(name: &str, output: &str) -> String {
    match name {
        "TodoWrite" if is_todo_list_output(output) => {
            format!("[工具结果] 已更新 {} 项", count_todo_item_lines(output))
        }
        _ => format!("[工具结果]\n{}", cap_tool_result_display(output)),
    }
}

pub(super) fn cap_tool_result_display(output: &str) -> String {
    let trimmed = output.trim_end();
    let line_count = trimmed.lines().count();
    let char_count = trimmed.chars().count();
    if line_count <= TOOL_RESULT_DISPLAY_MAX_LINES && char_count <= TOOL_RESULT_DISPLAY_MAX_CHARS {
        return trimmed.to_string();
    }
    let mut prefix = String::new();
    let mut used_chars = 0usize;
    for line in trimmed.lines().take(TOOL_RESULT_DISPLAY_MAX_LINES) {
        let extra = usize::from(!prefix.is_empty());
        let line_chars = line.chars().count();
        if used_chars + extra + line_chars > TOOL_RESULT_DISPLAY_MAX_CHARS {
            let remaining = TOOL_RESULT_DISPLAY_MAX_CHARS.saturating_sub(used_chars + extra);
            if remaining > 0 {
                if extra == 1 {
                    prefix.push('\n');
                }
                prefix.extend(line.chars().take(remaining));
            }
            break;
        }
        if extra == 1 {
            prefix.push('\n');
        }
        prefix.push_str(line);
        used_chars += extra + line_chars;
    }
    format!("{prefix}\n…（已截断，共 {line_count} 行 / {char_count} 字）")
}

fn is_todo_list_output(output: &str) -> bool {
    let trimmed = output.trim();
    trimmed == "(no todos)" || count_todo_item_lines(output) > 0
}

fn count_todo_item_lines(output: &str) -> usize {
    output
        .lines()
        .filter(|line| line.trim_start().starts_with("- ["))
        .count()
}

fn json_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(unknown)".to_string())
}

fn truncate_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{prefix}…")
}
