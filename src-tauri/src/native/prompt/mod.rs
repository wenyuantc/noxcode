#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;

use tauri::{AppHandle, Runtime};

use crate::git::runner::git;
use crate::git::{GitTarget, IndexMode};
use crate::native::settings::{
    load_native_settings, normalize_subagent_policy, SUBAGENT_POLICY_AGGRESSIVE,
    SUBAGENT_POLICY_CONSERVATIVE,
};
use crate::native::subagents::{NativeSubagent, TOOL_MODE_ALL};
use crate::native::tools::ssh::SshToolRuntime;

const IDENTITY: &str = include_str!("identity.md");
const AGENTS_FILE_CAP: usize = 32_768;
const PROJECT_INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "Agents.md", "CLAUDE.md"];

#[derive(Debug, Default, Clone)]
pub struct NativeGitInfo {
    pub branch: String,
    pub status: String,
    pub log: String,
}

#[derive(Debug, Default, Clone)]
pub struct NativePromptParts {
    pub cwd: String,
    pub model: String,
    pub platform: String,
    pub git: Option<NativeGitInfo>,
    pub global_template: String,
    pub project_agents: String,
    pub profile_prompt: String,
    pub max_concurrent_subagents: u32,
    pub subagent_policy: String,
    pub identity_override: String,
    pub required_subagent_name: String,
    pub required_subagent_description: String,
    pub permission_mode: String,
    pub skills: String,
}

pub fn compose_system(parts: &NativePromptParts) -> String {
    let mut blocks = Vec::new();
    if parts.identity_override.trim().is_empty() {
        blocks.push(IDENTITY.trim().to_string());
    } else {
        blocks.push(parts.identity_override.trim().to_string());
    }
    let policy_block = subagent_policy_block(&parts.subagent_policy);
    if !policy_block.is_empty() {
        blocks.push(policy_block);
    }
    let context = workspace_context_block(parts);
    if !context.is_empty() {
        blocks.push(context);
    }
    if !parts.global_template.trim().is_empty() {
        blocks.push(format!("# 全局提示词\n{}", parts.global_template.trim()));
    }
    if !parts.project_agents.trim().is_empty() {
        blocks.push(format!(
            "# 项目指令（AGENTS.md / CLAUDE.md）\n{}",
            parts.project_agents.trim()
        ));
    }
    if !parts.skills.trim().is_empty() {
        blocks.push(parts.skills.trim().to_string());
    }
    if !parts.profile_prompt.trim().is_empty() {
        blocks.push(format!("# Agent 档案设定\n{}", parts.profile_prompt.trim()));
    }
    if !parts.required_subagent_name.trim().is_empty() {
        blocks.push(required_subagent_block(
            parts.required_subagent_name.trim(),
            parts.required_subagent_description.trim(),
        ));
    }
    blocks.join("\n\n")
}

pub fn required_subagent_block(name: &str, description: &str) -> String {
    let desc = if description.is_empty() {
        String::new()
    } else {
        format!("用途：{description}\n")
    };
    format!(
        "# 任务指定子智能体（必须遵守）\n本任务已指定子智能体 `{name}`。{desc}- 第一轮必须调用 Agent，且 subagent_type 必须是 `{name}`（不要用 explore 或 general 代替）。\n- Agent.prompt 必须自包含完整任务，子 Agent 看不到本对话。\n- 在该子 Agent 返回报告之前，不要自己 Read/Edit/Write 去做实现。\n- 子 Agent 返回后，你再核对、补漏或收尾。"
    )
}

pub fn wrap_prompt_for_required_subagent(prompt: &str, name: &str) -> String {
    format!(
        "【必须先委派】立即调用 Agent：subagent_type=`{name}`，description 用 3–5 个字概括任务，prompt 放入下面的完整任务说明。不要先自己摸底或改文件。\n\n{prompt}"
    )
}

pub fn workspace_context_block(parts: &NativePromptParts) -> String {
    let mut blocks = vec![environment_block(parts)];
    if let Some(git) = parts.git.as_ref() {
        let git_text = git_block(git);
        if !git_text.is_empty() {
            blocks.push(git_text);
        }
    }
    blocks.join("\n\n")
}

fn environment_block(parts: &NativePromptParts) -> String {
    let permission_mode = if parts.permission_mode.trim().is_empty() {
        "confirm-high-risk"
    } else {
        parts.permission_mode.trim()
    };
    let mut lines = vec![
        "You have been invoked in the following environment:".to_string(),
        format!("- Working directory: {}", parts.cwd),
        format!("- Platform: {}", parts.platform),
        format!("- Date: {}", chrono::Local::now().format("%Y-%m-%d")),
        format!("- Permission mode: {permission_mode}"),
        format!(
            "- Max concurrent sub-agents: {}",
            parts.max_concurrent_subagents.max(1)
        ),
        format!(
            "- Sub-agent policy: {}",
            normalize_subagent_policy(Some(parts.subagent_policy.as_str()))
        ),
    ];
    if permission_mode == "plan" {
        lines.push(
            "- Plan mode: only Read/Glob/Grep/TodoRead/TodoWrite/WebFetch/WebSearch/Skill/AskQuestion. Do not edit files. If a user decision is required, call AskQuestion; if the plan is ready, output it and stop. The system will start implementation automatically after this plan turn."
                .to_string(),
        );
    }
    if !parts.model.trim().is_empty() {
        lines.push(format!(
            "- You are powered by the model named {}.",
            parts.model.trim()
        ));
    }
    lines.join("\n")
}

pub fn subagent_policy_block(policy: &str) -> String {
    let policy = normalize_subagent_policy(Some(policy));
    match policy.as_str() {
        SUBAGENT_POLICY_CONSERVATIVE => "# 子 Agent 策略（conservative）\n尽量自己用 Read / Glob / Grep / Edit 完成。仅当两块工作互不依赖、且各自需要较多探索或改动时，才在同一轮多次调用 Agent。单文件小改、后一步依赖前一步结果时不要委派。".to_string(),
        SUBAGENT_POLICY_AGGRESSIVE => "# 子 Agent 策略（aggressive）\n默认先拆。第一轮优先用 Agent 并行摸底（explore）或按块实现（general），不要自己先把 Glob/Read 做完再决定是否委派。只有改动明确只有一个已知文件时才跳过 Agent。同一轮尽量为独立工作流各调一次 Agent，不超过 Max concurrent sub-agents。".to_string(),
        _ => "# 子 Agent 策略（balanced）\n任务涉及两个以上模块/目录，或用户输入含多步执行计划时：同一轮至少委派 2 个 Agent（可并行的摸底用 explore，可并行的改动用 general）。仅当目标就是改一个已知文件、改动闭包很小，才自己 Read/Edit。不要把强依赖的步骤并行拆开。".to_string(),
    }
}

pub fn agent_tool_description(
    cap: u32,
    policy: &str,
    custom: &[NativeSubagent],
    required_type: Option<&str>,
) -> String {
    let policy = normalize_subagent_policy(Some(policy));
    let cap = cap.max(1);
    let policy_hint = match policy.as_str() {
        SUBAGENT_POLICY_CONSERVATIVE => {
            "Policy conservative: delegate only when two workstreams are independent and each needs substantial exploration or edits."
        }
        SUBAGENT_POLICY_AGGRESSIVE => {
            "Policy aggressive: default to spawning parallel Agents this turn unless the change is a single known file."
        }
        _ => {
            "Policy balanced: spawn at least two Agents in one turn when the task spans 2+ modules or includes a multi-step plan."
        }
    };
    let mut lines = vec![
        format!(
            "Delegate a self-contained subtask to a child agent. Multiple calls in one turn run in parallel (max {cap}). The child does not see this conversation. {policy_hint}"
        ),
        "When using the Agent tool, set subagent_type to one of the types below. If omitted, general is used.".to_string(),
    ];
    if let Some(required) = required_type.map(str::trim).filter(|item| !item.is_empty()) {
        lines.push(format!(
            "REQUIRED this turn: subagent_type={required}. Do not use explore or general instead of {required}."
        ));
    }
    lines.extend([
        "Available agent types:".to_string(),
        "- general: full tools including MCP; can edit files. (Tools: *)".to_string(),
        "- explore: read-only research. (Tools: Read, Glob, Grep, TodoRead, TodoWrite, WebFetch, WebSearch, Skill)".to_string(),
    ]);
    for item in custom {
        let tools = if item.tool_mode == TOOL_MODE_ALL {
            "*".to_string()
        } else if item.tools.is_empty() {
            "custom".to_string()
        } else {
            item.tools.join(", ")
        };
        lines.push(format!(
            "- {}: {} (Tools: {tools})",
            item.name, item.description
        ));
    }
    lines.join("\n")
}

fn git_block(git: &NativeGitInfo) -> String {
    let mut body = String::from("# Git context\n");
    if !git.branch.trim().is_empty() {
        body.push_str(&format!("Branch: {}\n", git.branch.trim()));
    }
    if !git.status.trim().is_empty() {
        body.push_str(&format!("Status:\n{}\n", git.status.trim()));
    }
    if !git.log.trim().is_empty() {
        body.push_str(&format!("Recent commits:\n{}\n", git.log.trim()));
    }
    if body.trim() == "# Git context" {
        String::new()
    } else {
        body
    }
}

fn cap_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= AGENTS_FILE_CAP {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(AGENTS_FILE_CAP).collect();
    format!("{prefix}…[truncated]")
}

pub fn read_local_project_agents(cwd: &str) -> String {
    let root = std::path::Path::new(cwd);
    let mut seen = HashSet::new();
    let mut chunks = Vec::new();
    for name in PROJECT_INSTRUCTION_FILES {
        let path = root.join(name);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let capped = cap_text(&text);
        if capped.is_empty() || !seen.insert(capped.clone()) {
            continue;
        }
        chunks.push(format!("## {name}\n{capped}"));
    }
    chunks.join("\n\n")
}

pub async fn read_ssh_project_agents(ssh: &SshToolRuntime) -> String {
    let mut seen = HashSet::new();
    let mut chunks = Vec::new();
    for name in PROJECT_INSTRUCTION_FILES {
        let Ok(text) = ssh.read(name).await else {
            continue;
        };
        if text.trim() == "(no output)" {
            continue;
        }
        let capped = cap_text(&text);
        if capped.is_empty() || !seen.insert(capped.clone()) {
            continue;
        }
        chunks.push(format!("## {name}\n{capped}"));
    }
    chunks.join("\n\n")
}

async fn git_text(target: &GitTarget, args: &[&str]) -> Option<String> {
    let output = git(target, args, &IndexMode::ReadOnly).await.ok()?;
    if !output.success() {
        return None;
    }
    Some(output.stdout_lossy().trim().to_string())
}

pub(crate) async fn detect_git(target: &GitTarget) -> Option<NativeGitInfo> {
    let branch = git_text(target, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    Some(NativeGitInfo {
        branch,
        status: git_text(target, &["status", "-sb"])
            .await
            .unwrap_or_default(),
        log: git_text(target, &["log", "-5", "--oneline"])
            .await
            .unwrap_or_default(),
    })
}

pub fn load_global_template<R: Runtime>(app: &AppHandle<R>) -> String {
    load_native_settings(app)
        .map(|settings| settings.global_prompt_template.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_includes_identity_global_project_and_profile() {
        let text = compose_system(&NativePromptParts {
            cwd: "/repo".to_string(),
            model: "demo".to_string(),
            platform: "macos".to_string(),
            git: Some(NativeGitInfo {
                branch: "main".to_string(),
                status: "## main".to_string(),
                log: "abc init".to_string(),
            }),
            global_template: "全局规则".to_string(),
            project_agents: "## AGENTS.md\n用 2 空格缩进".to_string(),
            profile_prompt: "角色：reviewer".to_string(),
            max_concurrent_subagents: 5,
            subagent_policy: "aggressive".to_string(),
            identity_override: String::new(),
            required_subagent_name: String::new(),
            required_subagent_description: String::new(),
            permission_mode: String::new(),
            skills: String::new(),
        });
        assert!(text.contains("内置编程 Agent"));
        assert!(text.contains("confirm-high-risk"));
        assert!(text.contains("Max concurrent sub-agents: 5"));
        assert!(text.contains("Sub-agent policy: aggressive"));
        assert!(text.contains("子 Agent 策略（aggressive）"));
        assert!(text.contains("explore"));
        assert!(text.contains("general"));
        assert!(text.contains("Working directory: /repo"));
        assert!(text.contains("Branch: main"));
        assert!(text.contains("全局规则"));
        assert!(text.contains("用 2 空格缩进"));
        assert!(text.contains("# Agent 档案设定"));
        assert!(text.contains("角色：reviewer"));
        let with_skills = compose_system(&NativePromptParts {
            skills: "# 可用技能\n- `demo`：desc（全局）".to_string(),
            ..NativePromptParts::default()
        });
        assert!(with_skills.contains("可用技能"));
        let overridden = compose_system(&NativePromptParts {
            cwd: "/repo".to_string(),
            identity_override: "你是审查员".to_string(),
            ..NativePromptParts::default()
        });
        assert!(overridden.contains("你是审查员"));
        assert!(!overridden.contains("内置编程 Agent"));
    }

    #[test]
    fn balanced_policy_asks_for_two_agents_on_multi_step_work() {
        let block = subagent_policy_block("balanced");
        assert!(block.contains("至少委派 2 个 Agent"));
        let custom = NativeSubagent {
            id: "1".to_string(),
            name: "code-reviewer".to_string(),
            description: "Review diffs".to_string(),
            model_mode: "inherit".to_string(),
            channel_id: None,
            model: None,
            tool_mode: "custom".to_string(),
            tools: vec!["Read".to_string(), "Grep".to_string()],
            system_prompt: String::new(),
            inject_agents_md: true,
            scope: "all".to_string(),
            workspace_ids: Vec::new(),
        };
        let with_custom =
            agent_tool_description(3, "balanced", std::slice::from_ref(&custom), None);
        assert!(with_custom.contains("code-reviewer"));
        assert!(with_custom.contains("Read, Grep"));
    }
}
