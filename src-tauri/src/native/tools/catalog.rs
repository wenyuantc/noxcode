use serde_json::json;

use crate::native::model::types::ToolSpec;

use super::contract::{
    builtin_contract, PatternSource, PermissionCapability, PreviewDirection, ResultBudget,
    ResultStrategy, RiskLevel, SideEffectScope, ToolContract, ToolTimeout,
};

/// 只读工具名（不含仅计划模式可见的 `AskQuestion`），供 explore 子 Agent 与
/// 计划模式白名单使用。顺序与 `tool_specs()` 一致。
pub fn read_only_tool_names() -> Vec<String> {
    tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .filter(|name| is_read_only_native_tool(name))
        .collect()
}

/// 计划模式（只读）下允许调用的内置工具，由契约的 `allowed_in_plan_mode` 决定。
pub fn is_read_only_native_tool(name: &str) -> bool {
    builtin_contract(name).is_some_and(|contract| contract.allowed_in_plan_mode)
}

fn budget(
    max_model_bytes: usize,
    strategy: ResultStrategy,
    preview: PreviewDirection,
) -> ResultBudget {
    ResultBudget {
        max_inline_bytes: 1_000_000,
        max_model_bytes,
        strategy,
        preview,
    }
}

#[allow(clippy::too_many_arguments)]
fn contract(
    name: &str,
    capability: &'static str,
    read_only: bool,
    destructive: bool,
    concurrent_safe: bool,
    side_effect_scope: SideEffectScope,
    risk_level: RiskLevel,
    needs_approval: bool,
    allowed_in_plan_mode: bool,
    permission: PermissionCapability,
    pattern_sources: &[PatternSource],
    result_budget: ResultBudget,
    timeout: ToolTimeout,
) -> ToolContract {
    ToolContract {
        name: name.to_string(),
        capability,
        read_only,
        destructive,
        concurrent_safe,
        side_effect_scope,
        risk_level,
        needs_approval,
        allowed_in_plan_mode,
        requires_user_interaction: false,
        permission,
        pattern_sources: pattern_sources.to_vec(),
        result_budget,
        timeout,
    }
}

/// 内置工具契约。名字必须与 `tool_specs()` / `ask_question_spec()` 一致。
pub fn tool_contracts() -> Vec<ToolContract> {
    use PatternSource::{Command, Input, Path, ToolName};
    let mut contracts = vec![
        contract(
            "Read",
            "读取工作区内的文本文件或图片，不修改任何状态",
            true,
            false,
            true,
            SideEffectScope::None,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::Read,
            &[Path],
            budget(200_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "Write",
            "创建或覆盖工作区文件",
            false,
            false,
            false,
            SideEffectScope::Workspace,
            RiskLevel::Medium,
            true,
            false,
            PermissionCapability::Edit,
            &[Path],
            budget(100_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "Edit",
            "在文件中做精确字符串替换",
            false,
            false,
            false,
            SideEffectScope::Workspace,
            RiskLevel::Medium,
            true,
            false,
            PermissionCapability::Edit,
            &[Path],
            budget(100_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "ApplyPatch",
            "应用 Codex 风格多文件补丁",
            false,
            true,
            false,
            SideEffectScope::Workspace,
            RiskLevel::Medium,
            true,
            false,
            PermissionCapability::Edit,
            &[Input],
            budget(100_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(60_000),
        ),
        contract(
            "Bash",
            "在工作区执行 shell 命令，可能影响文件、Git、网络或系统状态",
            false,
            true,
            false,
            SideEffectScope::System,
            RiskLevel::High,
            true,
            false,
            PermissionCapability::Bash,
            &[Command],
            budget(30_000, ResultStrategy::Artifact, PreviewDirection::Tail),
            ToolTimeout {
                default_ms: 120_000,
                max_ms: 600_000,
                allow_call_override: true,
            },
        ),
        contract(
            "Glob",
            "按 glob 模式列出文件路径，不读取内容",
            true,
            false,
            true,
            SideEffectScope::None,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::Read,
            &[Path, Input],
            budget(50_000, ResultStrategy::Artifact, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "Grep",
            "用正则搜索文件内容",
            true,
            false,
            true,
            SideEffectScope::None,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::Read,
            &[Path, Input],
            budget(60_000, ResultStrategy::Artifact, PreviewDirection::Head),
            ToolTimeout::fixed(60_000),
        ),
        contract(
            "TodoRead",
            "读取当前会话的待办清单",
            true,
            false,
            true,
            SideEffectScope::None,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::TodoRead,
            &[ToolName],
            budget(20_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "TodoWrite",
            "替换当前会话的待办清单，用于跟踪多步任务进度",
            true,
            false,
            false,
            SideEffectScope::Session,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::TodoWrite,
            &[ToolName],
            budget(20_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "WebFetch",
            "抓取公开 URL 并转成可读文本",
            true,
            false,
            true,
            SideEffectScope::Network,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::WebFetch,
            &[Input],
            budget(80_000, ResultStrategy::Artifact, PreviewDirection::Head),
            ToolTimeout::fixed(45_000),
        ),
        contract(
            "WebSearch",
            "联网搜索",
            true,
            false,
            true,
            SideEffectScope::Network,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::WebSearch,
            &[Input],
            budget(20_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(45_000),
        ),
        contract(
            "Skill",
            "把本地技能说明加载进当前会话",
            true,
            false,
            true,
            SideEffectScope::Session,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::Skill,
            &[Input],
            budget(60_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ),
        contract(
            "Agent",
            "启动子 Agent；子 Agent 的工具调用单独受限与审批",
            true,
            false,
            false,
            SideEffectScope::Session,
            RiskLevel::Low,
            false,
            false,
            PermissionCapability::Subagent,
            &[Input],
            budget(64_000, ResultStrategy::Artifact, PreviewDirection::Head),
            ToolTimeout::none(),
        ),
    ];
    for name in ["AskUserQuestion", "AskQuestion"] {
        let mut ask = contract(
            name,
            "向用户提出多选澄清问题并等待回答",
            true,
            false,
            true,
            SideEffectScope::None,
            RiskLevel::Low,
            false,
            true,
            PermissionCapability::AskUser,
            &[ToolName],
            budget(20_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::none(),
        );
        ask.requires_user_interaction = true;
        contracts.push(ask);
    }
    contracts.push(contract(
        "EnterPlanMode",
        "进入只读计划模式",
        true,
        false,
        false,
        SideEffectScope::Session,
        RiskLevel::Low,
        false,
        false,
        PermissionCapability::AskUser,
        &[ToolName],
        budget(4_000, ResultStrategy::Truncate, PreviewDirection::Head),
        ToolTimeout::fixed(10_000),
    ));
    for (name, capability_text, permission) in [
        (
            "TaskOutput",
            "读取 / 等待后台子 Agent 任务的结果",
            PermissionCapability::Subagent,
        ),
        (
            "TaskStop",
            "停止后台子 Agent 任务",
            PermissionCapability::Subagent,
        ),
        (
            "SendMessage",
            "给运行中的后台子 Agent 追加指令",
            PermissionCapability::AgentMessageSend,
        ),
        (
            "RespondToCoordinator",
            "子 Agent 给父 Agent 留言",
            PermissionCapability::AgentMessageRespond,
        ),
    ] {
        contracts.push(contract(
            name,
            capability_text,
            true,
            false,
            false,
            SideEffectScope::Session,
            RiskLevel::Low,
            false,
            false,
            permission,
            &[ToolName, Input],
            budget(64_000, ResultStrategy::Artifact, PreviewDirection::Head),
            ToolTimeout::none(),
        ));
    }
    for (name, capability_text, read_only, needs_approval, plan_ok, permission) in [
        (
            "CronCreate",
            "创建定时自动化会话",
            false,
            true,
            false,
            PermissionCapability::AutomationWrite,
        ),
        (
            "CronList",
            "列出工作区自动化",
            true,
            false,
            true,
            PermissionCapability::AutomationRead,
        ),
        (
            "CronDelete",
            "删除自动化",
            false,
            true,
            false,
            PermissionCapability::AutomationWrite,
        ),
        (
            "Goal",
            "维护会话目标与进度清单",
            true,
            false,
            true,
            PermissionCapability::GoalRead,
        ),
        (
            "GoalRead",
            "读取会话目标",
            true,
            false,
            true,
            PermissionCapability::GoalRead,
        ),
        (
            "ReadSessionContext",
            "读取同工作区其它会话的上下文",
            true,
            false,
            true,
            PermissionCapability::SessionContextRead,
        ),
    ] {
        contracts.push(contract(
            name,
            capability_text,
            read_only,
            false,
            false,
            SideEffectScope::Session,
            if needs_approval {
                RiskLevel::Medium
            } else {
                RiskLevel::Low
            },
            needs_approval,
            plan_ok,
            permission,
            &[ToolName, Input],
            budget(40_000, ResultStrategy::Truncate, PreviewDirection::Head),
            ToolTimeout::fixed(30_000),
        ));
    }
    let mut exit = contract(
        "ExitPlanMode",
        "提交计划请求用户批准并退出计划模式",
        true,
        false,
        false,
        SideEffectScope::Session,
        RiskLevel::Low,
        false,
        true,
        PermissionCapability::AskUser,
        &[ToolName],
        budget(20_000, ResultStrategy::Truncate, PreviewDirection::Head),
        ToolTimeout::none(),
    );
    exit.requires_user_interaction = true;
    contracts.push(exit);
    contracts
}

pub fn ask_question_spec() -> ToolSpec {
    spec(
        "AskUserQuestion",
        "Ask the user blocking multiple-choice questions when a decision cannot be inferred from the repo (scope, trade-offs, destructive actions). Do not ask facts you can Read/Grep/Glob. At most 4 questions per call, each with up to 6 options; the user may also answer freely.",
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string"},
                            "options": {
                                "type": "array",
                                "items": {"type": "string"}
                            }
                        },
                        "required": ["prompt"]
                    }
                }
            },
            "required": ["questions"]
        }),
    )
}

pub fn enter_plan_mode_spec() -> ToolSpec {
    spec(
        "EnterPlanMode",
        "Switch to read-only planning before a non-trivial implementation: only Read/Glob/Grep/WebFetch/WebSearch/Skill/Todo/AskUserQuestion stay available. Explore, then call ExitPlanMode with the full plan to ask the user for approval.",
        json!({"type": "object", "properties": {}}),
    )
}

pub fn exit_plan_mode_spec() -> ToolSpec {
    spec(
        "ExitPlanMode",
        "Submit the finished plan for user approval and leave planning mode. Only call this after exploring enough to write a concrete plan; if the user rejects, revise and call again.",
        json!({
            "type": "object",
            "properties": {
                "plan": {"type": "string", "description": "Complete plan in Markdown: goal, scope, steps, verification, risks."}
            },
            "required": ["plan"]
        }),
    )
}

pub fn task_output_spec() -> ToolSpec {
    spec(
        "TaskOutput",
        "Read the result of a background Agent task started with run_in_background=true. With wait=true (default) it blocks until the task finishes or timeout_ms elapses, then returns the status and report.",
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "wait": {"type": "boolean", "description": "Block until finished (default true)."},
                "timeout_ms": {"type": "integer", "description": "Max wait in milliseconds (default 30000, max 600000)."}
            },
            "required": ["task_id"]
        }),
    )
}

pub fn task_stop_spec() -> ToolSpec {
    spec(
        "TaskStop",
        "Cancel a running background Agent task by task_id.",
        json!({
            "type": "object",
            "properties": {"task_id": {"type": "string"}},
            "required": ["task_id"]
        }),
    )
}

pub fn send_message_spec() -> ToolSpec {
    spec(
        "SendMessage",
        "Send an additional instruction to a running background Agent task; the child reads it before its next model call.",
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "message": {"type": "string"}
            },
            "required": ["task_id", "message"]
        }),
    )
}

pub fn respond_to_coordinator_spec() -> ToolSpec {
    spec(
        "RespondToCoordinator",
        "Leave a short message for the parent agent that started you in the background (progress, blockers, questions). Keep working afterwards unless the task is done.",
        json!({
            "type": "object",
            "properties": {"message": {"type": "string"}},
            "required": ["message"]
        }),
    )
}

pub fn automation_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "CronCreate",
            "Schedule a recurring automation in this workspace: at each cron tick a new agent session runs the given prompt. Use standard 5-field cron (minute hour day month weekday) or @hourly/@daily/@weekly. Ask the user before creating one unless they explicitly requested it.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "cron": {"type": "string", "description": "e.g. \"0 9 * * mon-fri\""},
                    "prompt": {"type": "string", "description": "Self-contained task for the scheduled session"}
                },
                "required": ["name", "cron", "prompt"]
            }),
        ),
        spec(
            "CronList",
            "List the automations configured for this workspace with their next/last run times.",
            json!({"type": "object", "properties": {}}),
        ),
        spec(
            "CronDelete",
            "Delete an automation by id (see CronList).",
            json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        ),
        spec(
            "Goal",
            "Maintain the session goal shown to the user: action=set (new goal with optional checklist), update (change title/checklist/note), complete, clear. Checklist items are strings or {item, done}.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["set", "update", "complete", "clear"]},
                    "title": {"type": "string"},
                    "checklist": {"type": "array", "items": {}},
                    "note": {"type": "string"}
                },
                "required": ["action"]
            }),
        ),
        spec(
            "GoalRead",
            "Read the current session goal and checklist progress.",
            json!({"type": "object", "properties": {}}),
        ),
        spec(
            "ReadSessionContext",
            "Read other sessions of this workspace: without session_id it lists recent sessions (title, time, last reply); with session_id it returns that session's recent user/assistant exchange so you can continue earlier work.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": {"type": "string"},
                    "limit": {"type": "integer", "description": "Sessions to list or messages to include (default 10)."}
                }
            }),
        ),
    ]
}

pub fn tool_specs() -> Vec<ToolSpec> {
    let mut specs = core_tool_specs();
    specs.push(ask_question_spec());
    specs.push(enter_plan_mode_spec());
    specs.push(exit_plan_mode_spec());
    specs.push(task_output_spec());
    specs.push(task_stop_spec());
    specs.push(send_message_spec());
    specs.push(respond_to_coordinator_spec());
    specs.extend(automation_specs());
    specs
}

fn core_tool_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "Read",
            "Read a file from the workspace. Prefer this over cat in Bash.",
            json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "offset": {"type": "integer"},
                    "limit": {"type": "integer"}
                },
                "required": ["file_path"]
            }),
        ),
        spec(
            "Write",
            "Create or overwrite a file. Read an existing file before overwriting it.",
            json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["file_path", "content"]
            }),
        ),
        spec(
            "Edit",
            "Exact string replacement in a file. Read the file first. old_string must be unique unless replace_all is true.",
            json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean"}
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        ),
        spec(
            "Bash",
            "Run a shell command in the workspace. Prefer Read/Glob/Grep for file inspection.",
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout": {"type": "integer"},
                    "description": {"type": "string"}
                },
                "required": ["command"]
            }),
        ),
        spec(
            "Glob",
            "Find files by glob pattern, such as **/*.rs.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string"}
                },
                "required": ["pattern"]
            }),
        ),
        spec(
            "Grep",
            "Search file contents. Prefer this over grep/rg in Bash.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string"},
                    "glob": {"type": "string"},
                    "head_limit": {"type": "integer"}
                },
                "required": ["pattern"]
            }),
        ),
        spec(
            "TodoRead",
            "Read the current session todo list.",
            json!({"type": "object", "properties": {}}),
        ),
        spec(
            "TodoWrite",
            "Replace the session todo list. Keep at most one item in_progress.",
            json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string"},
                                "content": {"type": "string"},
                                "status": {"type": "string"},
                                "priority": {"type": "string"}
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        ),
        spec(
            "WebFetch",
            "Fetch a public http(s) URL, convert readable content to text, and optionally extract by prompt.",
            json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "prompt": {"type": "string"}
                },
                "required": ["url"]
            }),
        ),
        spec(
            "WebSearch",
            "Search the live web. Prefer a natural-language query. After answering, list Sources as markdown links.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "num_results": {"type": "integer"}
                },
                "required": ["query"]
            }),
        ),
        spec(
            "ApplyPatch",
            "Apply a Codex-style multi-file patch. Prefer this over many Edit/Write calls when changing several files. Include enough context in each hunk. Do not use this in plan mode.",
            json!({
                "type": "object",
                "properties": {
                    "patch": {"type": "string"}
                },
                "required": ["patch"]
            }),
        ),
        spec(
            "Skill",
            "Load a discovered skill's full SKILL.md and list extra files in its directory. Use after seeing the skill in the available-skills list. Prefer this over guessing skill contents.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                },
                "required": ["name"]
            }),
        ),
        spec(
            "Agent",
            "Delegate a self-contained subtask to a child agent. Multiple calls in one turn run in parallel up to the session cap (see Max concurrent sub-agents). The child does not see this conversation. Use explore for read-only research and general for edits. Do not use this for a single Read or Grep. Set run_in_background=true for long tasks: the call returns a task_id immediately; read the result later with TaskOutput, steer with SendMessage, cancel with TaskStop.",
            json!({
                "type": "object",
                "properties": {
                    "description": {"type": "string"},
                    "prompt": {"type": "string"},
                    "subagent_type": {"type": "string"},
                    "run_in_background": {"type": "boolean"}
                },
                "required": ["description", "prompt"]
            }),
        ),
    ]
}

fn spec(name: &str, description: &str, parameters: serde_json::Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_exclude_writers() {
        assert!(is_read_only_native_tool("Read"));
        assert!(is_read_only_native_tool("Grep"));
        assert!(is_read_only_native_tool("TodoWrite"));
        assert!(is_read_only_native_tool("AskQuestion"));
        assert!(is_read_only_native_tool("Skill"));
        assert!(!is_read_only_native_tool("Write"));
        assert!(!is_read_only_native_tool("Edit"));
        assert!(!is_read_only_native_tool("Bash"));
        assert!(!is_read_only_native_tool("ApplyPatch"));
        assert!(!is_read_only_native_tool("Agent"));
        let names = read_only_tool_names();
        assert_eq!(
            names,
            vec![
                "Read",
                "Glob",
                "Grep",
                "TodoRead",
                "TodoWrite",
                "WebFetch",
                "WebSearch",
                "Skill",
                "AskUserQuestion",
                "ExitPlanMode",
                "CronList",
                "Goal",
                "GoalRead",
                "ReadSessionContext"
            ]
        );
        assert!(is_read_only_native_tool("ExitPlanMode"));
        assert!(!is_read_only_native_tool("EnterPlanMode"));
    }

    #[test]
    fn every_spec_has_a_contract_and_vice_versa() {
        let specs: Vec<String> = tool_specs().into_iter().map(|item| item.name).collect();
        let contracts: Vec<String> = tool_contracts().into_iter().map(|item| item.name).collect();
        for name in &specs {
            assert!(contracts.contains(name), "contract missing for {name}");
        }
        for name in &contracts {
            assert!(
                specs.contains(name) || name == "AskQuestion",
                "spec missing for {name}"
            );
        }
        assert!(specs.contains(&"AskUserQuestion".to_string()));
        assert!(specs.contains(&"EnterPlanMode".to_string()));
        assert!(specs.contains(&"ExitPlanMode".to_string()));
        let bash = builtin_contract("Bash").expect("bash contract");
        assert_eq!(bash.side_effect_scope, SideEffectScope::System);
        assert_eq!(bash.risk_level, RiskLevel::High);
        assert!(!bash.allowed_in_plan_mode);
        assert_eq!(bash.result_budget.strategy, ResultStrategy::Artifact);
        assert_eq!(bash.result_budget.preview, PreviewDirection::Tail);
    }

    #[test]
    fn includes_core_tools() {
        let names: Vec<_> = tool_specs().into_iter().map(|item| item.name).collect();
        for expected in [
            "Read",
            "Write",
            "Edit",
            "Bash",
            "Glob",
            "Grep",
            "TodoRead",
            "TodoWrite",
            "WebFetch",
            "WebSearch",
            "ApplyPatch",
            "Skill",
            "Agent",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
        assert!(!names.contains(&"AskQuestion".to_string()));
        assert!(names.contains(&"AskUserQuestion".to_string()));
    }
}
