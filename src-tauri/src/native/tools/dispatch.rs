use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::cancel::CancelFlag;
use super::hooks::{run_post_tool_hooks, run_pre_tool_hooks};
use super::local::{apply_edit, format_read, LocalWorkspace};
use super::mcp::SharedMcp;
use super::patch::{extract_patch_text, parse_patch, patch_counts, plan_mutations, FileMutation};
use super::paths::resolve_under_workspace;
use super::permission::{
    classify_native_tool_risk, NativePermissionDecision, NativeToolRisk, NativeToolRiskKind,
};
use super::question::{format_ask_question_result, parse_ask_question_args, PlanQuestionAnswer};
use super::ssh::SshToolRuntime;
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Deserialize)]
pub struct TodoItem {
    #[serde(default)]
    pub id: String,
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub priority: String,
}

#[derive(Debug, Clone)]
pub struct PermissionPrompt {
    pub request_id: String,
    pub tool_name: String,
    pub kind: NativeToolRiskKind,
    pub summary: String,
    pub remote: bool,
    pub mcp_server_id: Option<String>,
}

pub type PermissionRequester =
    Arc<dyn Fn(PermissionPrompt, oneshot::Sender<NativePermissionDecision>) + Send + Sync>;

pub type QuestionRequester =
    Arc<dyn Fn(Vec<super::PlanQuestion>, oneshot::Sender<PlanQuestionAnswer>) + Send + Sync>;

pub type PermissionExpirer =
    Arc<dyn Fn(String) -> tauri::async_runtime::JoinHandle<()> + Send + Sync>;

pub type MutationHook = Arc<dyn Fn(&str) + Send + Sync>;

pub struct ToolCtx {
    pub workspace: LocalWorkspace,
    pub ssh: Option<SshToolRuntime>,
    pub cancel: CancelFlag,
    pub read_files: HashSet<String>,
    pub todos: Vec<TodoItem>,
    pub mcp: SharedMcp,
    pub allow_all_high_risk: Arc<std::sync::atomic::AtomicBool>,
    pub allowed_mcp_servers: Arc<Mutex<HashSet<String>>>,
    pub request_permission: Option<PermissionRequester>,
    pub expire_permission: Option<PermissionExpirer>,
    pub permission_timeout: Duration,
    pub request_question: Option<QuestionRequester>,
    pub read_only: bool,
    pub skills: Vec<crate::native::skills::NativeSkill>,
    pub hooks: Vec<crate::db::models::NativeHook>,
    pub on_mutation: Option<MutationHook>,
}

pub async fn execute_tool(
    ctx: &mut ToolCtx,
    name: &str,
    arguments: &str,
) -> Result<String, String> {
    if ctx.cancel.is_cancelled() {
        return Err("已取消".to_string());
    }
    if ctx.read_only && !super::is_read_only_native_tool(name) {
        return Err(format!("只读规划模式禁止调用工具 {name}"));
    }
    run_pre_tool_hooks(
        &ctx.workspace,
        ctx.ssh.as_ref(),
        &ctx.cancel,
        &ctx.hooks,
        name,
        arguments,
    )
    .await?;
    confirm_if_high_risk(ctx, name, arguments).await?;
    let result = match name {
        "Read" => call_read(ctx, arguments).await,
        "Write" => call_write(ctx, arguments).await,
        "Edit" => call_edit(ctx, arguments).await,
        "ApplyPatch" => call_apply_patch(ctx, arguments).await,
        "Glob" => call_glob(ctx, arguments).await,
        "Grep" => call_grep(ctx, arguments).await,
        "Bash" => call_bash(ctx, arguments).await,
        "TodoRead" => Ok(format_todos(&ctx.todos)),
        "TodoWrite" => call_todo_write(ctx, arguments),
        "WebFetch" => super::web::web_fetch(arguments).await,
        "WebSearch" => super::web::web_search(arguments).await,
        "AskQuestion" => call_ask_question(ctx, arguments).await,
        "Skill" => call_skill(ctx, arguments),
        other if ctx.mcp.has_tool(other).await => ctx.mcp.call(other, arguments).await,
        other => Err(format!("unknown tool: {other}")),
    };
    match result {
        Ok(output) => {
            if matches!(name, "Write" | "Edit" | "ApplyPatch") {
                if let Some(on_mutation) = &ctx.on_mutation {
                    on_mutation(name);
                }
            }
            let warnings = run_post_tool_hooks(
                &ctx.workspace,
                ctx.ssh.as_ref(),
                &ctx.cancel,
                &ctx.hooks,
                name,
                arguments,
            )
            .await;
            if warnings.is_empty() {
                Ok(output)
            } else {
                Ok(format!("{output}\n\n[钩子警告]\n{}", warnings.join("\n")))
            }
        }
        Err(error) => Err(error),
    }
}

async fn call_ask_question(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let questions = parse_ask_question_args(arguments)?;
    let Some(requester) = ctx.request_question.clone() else {
        return Err("当前不是计划提问阶段，不能向用户提问".to_string());
    };
    let (tx, rx) = oneshot::channel();
    requester(questions.clone(), tx);
    let answer = tokio::select! {
        biased;
        _ = async {
            loop {
                if ctx.cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        } => return Err("已取消".to_string()),
        result = rx => result.map_err(|_| "已取消".to_string())?,
    };
    if ctx.cancel.is_cancelled() {
        return Err("已取消".to_string());
    }
    if !answer.skipped && answer.answers.len() != questions.len() {
        return Err("回答数量与问题数量不一致".to_string());
    }
    if !answer.skipped && answer.answers.iter().any(|item| item.trim().is_empty()) {
        return Err("每个问题都需要回答".to_string());
    }
    Ok(format_ask_question_result(&questions, &answer))
}

async fn confirm_if_high_risk(
    ctx: &mut ToolCtx,
    name: &str,
    arguments: &str,
) -> Result<(), String> {
    let exists = match name {
        "Write" => write_target_exists(ctx, arguments).await,
        _ => None,
    };
    let is_mcp = ctx.mcp.has_tool(name).await || name.starts_with("mcp_");
    match classify_native_tool_risk(name, arguments, exists, is_mcp) {
        NativeToolRisk::Low => Ok(()),
        NativeToolRisk::High { kind, summary } => {
            request_permission(ctx, name, kind, summary).await
        }
    }
}

async fn write_target_exists(ctx: &ToolCtx, arguments: &str) -> Option<bool> {
    let path = serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("file_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
        })?;
    if ctx.ssh.is_some() {
        return Some(ctx.read_files.contains(&path));
    }
    ctx.workspace
        .resolve(&path)
        .ok()
        .map(|resolved| resolved.exists())
}

async fn request_permission(
    ctx: &mut ToolCtx,
    name: &str,
    kind: NativeToolRiskKind,
    summary: String,
) -> Result<(), String> {
    let mcp_server_id = if kind == NativeToolRiskKind::Mcp {
        ctx.mcp.server_id_for_tool(name).await
    } else {
        None
    };
    if kind != NativeToolRiskKind::Mcp
        && ctx
            .allow_all_high_risk
            .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(());
    }
    if kind == NativeToolRiskKind::Mcp {
        if let Some(server_id) = mcp_server_id.as_deref() {
            if ctx
                .allowed_mcp_servers
                .lock()
                .map(|allowed| allowed.contains(server_id))
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
    }
    let Some(requester) = ctx.request_permission.clone() else {
        // Setting off, or tests that skip the UI channel.
        return Ok(());
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let prompt = PermissionPrompt {
        request_id: request_id.clone(),
        tool_name: name.to_string(),
        kind,
        summary: if ctx.ssh.is_some() {
            format!("远程工作区 · {summary}")
        } else {
            summary
        },
        remote: ctx.ssh.is_some(),
        mcp_server_id,
    };
    let (tx, rx) = oneshot::channel();
    requester(prompt, tx);
    let timeout = ctx.permission_timeout;
    enum PermissionWait {
        Cancelled,
        TimedOut,
        Decision(NativePermissionDecision),
    }
    let wait = tokio::select! {
        biased;
        _ = async {
            loop {
                if ctx.cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        } => PermissionWait::Cancelled,
        _ = async {
            if timeout.is_zero() {
                std::future::pending::<()>().await;
            } else {
                tokio::time::sleep(timeout).await;
            }
        } => PermissionWait::TimedOut,
        result = rx => PermissionWait::Decision(result.unwrap_or(NativePermissionDecision::Deny)),
    };
    let decision = match wait {
        PermissionWait::Cancelled => NativePermissionDecision::Deny,
        PermissionWait::TimedOut => {
            if let Some(expire) = &ctx.expire_permission {
                let _ = expire(request_id).await;
            }
            return Err("确认超时，已按拒绝处理".to_string());
        }
        PermissionWait::Decision(decision) => decision,
    };
    match decision {
        NativePermissionDecision::AllowSession if kind == NativeToolRiskKind::Mcp => {
            allow_mcp_server(ctx, name).await;
            Ok(())
        }
        NativePermissionDecision::AllowSession => {
            ctx.allow_all_high_risk
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        NativePermissionDecision::AllowServer => {
            allow_mcp_server(ctx, name).await;
            Ok(())
        }
        NativePermissionDecision::AllowOnce => Ok(()),
        NativePermissionDecision::Deny => Err("用户不允许该高风险操作".to_string()),
    }
}

async fn allow_mcp_server(ctx: &ToolCtx, tool_name: &str) {
    if let Some(server_id) = ctx.mcp.server_id_for_tool(tool_name).await {
        if let Ok(mut allowed) = ctx.allowed_mcp_servers.lock() {
            allowed.insert(server_id);
        }
    }
}

fn call_skill(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let name = string_arg(&args, "name")?;
    let skill = crate::native::skills::find_skill(&ctx.skills, &name)?;
    Ok(crate::native::skills::render_skill(
        skill,
        ctx.ssh.is_some(),
    ))
}

async fn call_apply_patch(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let patch = extract_patch_text(arguments)?;
    let actions = parse_patch(&patch)?;
    let mut cache: HashMap<String, Option<String>> = HashMap::new();
    let mut needed = Vec::new();
    for action in &actions {
        match action {
            crate::native::tools::patch::PatchAction::Add { path, .. }
            | crate::native::tools::patch::PatchAction::Delete { path }
            | crate::native::tools::patch::PatchAction::Update { path, .. } => {
                needed.push(path.clone());
            }
        }
        if let crate::native::tools::patch::PatchAction::Update {
            move_to: Some(dest),
            ..
        } = action
        {
            needed.push(dest.clone());
        }
    }
    for path in needed {
        if cache.contains_key(&path) {
            continue;
        }
        let content = load_patch_file(ctx, &path).await?;
        cache.insert(path, content);
    }
    let mutations = plan_mutations(&actions, |path| Ok(cache.get(path).cloned().flatten()))?;
    let mut notes = Vec::new();
    for mutation in mutations {
        match mutation {
            FileMutation::Write { path, content } => {
                if let Some(ssh) = ctx.ssh.as_ref() {
                    ssh.write(&path, &content).await?;
                    ctx.read_files.insert(path.clone());
                } else {
                    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
                    ctx.workspace.write_file(&path, &content)?;
                    ctx.read_files
                        .insert(resolved.to_string_lossy().into_owned());
                }
                notes.push(format!("wrote {path}"));
            }
            FileMutation::Delete { path } => {
                if let Some(ssh) = ctx.ssh.as_ref() {
                    ssh.delete(&path).await?;
                    ctx.read_files.insert(path.clone());
                } else {
                    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
                    ctx.workspace.delete_file(&path)?;
                    ctx.read_files
                        .insert(resolved.to_string_lossy().into_owned());
                }
                notes.push(format!("deleted {path}"));
            }
        }
    }
    Ok(format!(
        "{}\n{}",
        patch_counts(&actions).summary(),
        notes.join("\n")
    ))
}

async fn load_patch_file(ctx: &ToolCtx, path: &str) -> Result<Option<String>, String> {
    if let Some(ssh) = ctx.ssh.as_ref() {
        return match ssh.read(path).await {
            Ok(text) if text.trim() == "(no output)" => Ok(Some(String::new())),
            Ok(text) => Ok(Some(text)),
            Err(_) => Ok(None),
        };
    }
    let resolved = match resolve_under_workspace(&ctx.workspace.root, path) {
        Ok(path) => path,
        Err(error) => return Err(error),
    };
    if !resolved.is_file() {
        return Ok(None);
    }
    fs::read_to_string(&resolved)
        .map(Some)
        .map_err(|error| format!("读取失败: {error}"))
}

fn parse_args(arguments: &str) -> Result<Value, String> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(arguments).map_err(|error| format!("工具参数不是合法 JSON: {error}"))
}

async fn call_read(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let offset = args.get("offset").and_then(Value::as_i64);
    let limit = args.get("limit").and_then(Value::as_i64);
    if let Some(ssh) = ctx.ssh.as_ref() {
        let raw = ssh.read(&path).await?;
        ctx.read_files.insert(path.clone());
        return Ok(format_read(&raw, offset, limit));
    }
    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
    let output = ctx.workspace.read_file(&path, offset, limit)?;
    ctx.read_files
        .insert(resolved.to_string_lossy().into_owned());
    Ok(output)
}

async fn call_write(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let content = string_arg(&args, "content")?;
    if let Some(ssh) = ctx.ssh.as_ref() {
        if !ctx.read_files.contains(&path) {
            return Err(
                "File has not been read yet. Read it first before writing to it.".to_string(),
            );
        }
        let output = ssh.write(&path, &content).await?;
        ctx.read_files.insert(path);
        return Ok(output);
    }
    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
    if resolved.exists()
        && !ctx
            .read_files
            .contains(&resolved.to_string_lossy().into_owned())
    {
        return Err("File has not been read yet. Read it first before writing to it.".to_string());
    }
    let output = ctx.workspace.write_file(&path, &content)?;
    ctx.read_files
        .insert(resolved.to_string_lossy().into_owned());
    Ok(output)
}

async fn call_edit(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let old = string_arg(&args, "old_string")?;
    let new = string_arg(&args, "new_string")?;
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(ssh) = ctx.ssh.as_ref() {
        if !ctx.read_files.contains(&path) {
            return Err("File has not been read yet. Read it first before editing.".to_string());
        }
        let original = ssh.read(&path).await?;
        let updated = apply_edit(&original, &old, &new, replace_all)?;
        ssh.write(&path, &updated).await?;
        return Ok(format!("Edited {path}"));
    }
    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
    if !ctx
        .read_files
        .contains(&resolved.to_string_lossy().into_owned())
    {
        return Err("File has not been read yet. Read it first before editing.".to_string());
    }
    let original =
        std::fs::read_to_string(&resolved).map_err(|error| format!("读取失败: {error}"))?;
    let updated = apply_edit(&original, &old, &new, replace_all)?;
    ctx.workspace.write_file(&path, &updated)?;
    Ok(format!("Edited {}", resolved.display()))
}

async fn call_glob(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let pattern = string_arg(&args, "pattern")?;
    let path = args.get("path").and_then(Value::as_str);
    if let Some(ssh) = ctx.ssh.as_ref() {
        let listing = ssh.glob().await?;
        let hits: Vec<_> = listing
            .lines()
            .filter(|line| super::glob::glob_match(&pattern, line.trim()))
            .take(100)
            .map(ToOwned::to_owned)
            .collect();
        return Ok(if hits.is_empty() {
            "No files found".to_string()
        } else {
            hits.join("\n")
        });
    }
    ctx.workspace.glob_files(&pattern, path)
}

async fn call_grep(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let pattern = string_arg(&args, "pattern")?;
    let path = args.get("path").and_then(Value::as_str);
    let glob = args.get("glob").and_then(Value::as_str);
    let head_limit = args.get("head_limit").and_then(Value::as_i64);
    if let Some(ssh) = ctx.ssh.as_ref() {
        return ssh.grep(&pattern, path).await;
    }
    ctx.workspace.grep_files(&pattern, path, glob, head_limit)
}

async fn call_bash(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let command = string_arg(&args, "command")?;
    let timeout = args.get("timeout").and_then(Value::as_i64);
    if let Some(ssh) = ctx.ssh.as_ref() {
        return ssh.bash(&command).await;
    }
    ctx.workspace.bash(&command, timeout, &ctx.cancel).await
}

fn call_todo_write(ctx: &mut ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let todos: Vec<TodoItem> =
        serde_json::from_value(args.get("todos").cloned().unwrap_or(Value::Null))
            .map_err(|_| "todos 必须是数组".to_string())?;
    ctx.todos = todos
        .into_iter()
        .enumerate()
        .map(|(index, mut item)| {
            if item.id.trim().is_empty() {
                item.id = format!("{}", index + 1);
            }
            if item.priority.trim().is_empty() {
                item.priority = "medium".to_string();
            }
            item
        })
        .collect();
    Ok(format_todos(&ctx.todos))
}

fn format_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "(no todos)".to_string();
    }
    todos
        .iter()
        .map(|item| format!("- [{}] {} ({})", item.status, item.content, item.priority))
        .collect::<Vec<_>>()
        .join("\n")
}

fn string_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{key} 不能为空"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn deny_keeps_existing_file() {
        let root = std::env::temp_dir().join(format!(
            "codex-ai-perm-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("mkdir");
        let path = root.join("keep.txt");
        std::fs::write(&path, "original").expect("write");
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: Some(Arc::new(
                |_prompt, tx: oneshot::Sender<NativePermissionDecision>| {
                    let _ = tx.send(NativePermissionDecision::Deny);
                },
            )),
            request_question: None,
            read_only: false,
            skills: Vec::new(),
            hooks: Vec::new(),
            on_mutation: None,
        };
        let err = execute_tool(
            &mut ctx,
            "Write",
            r#"{"file_path":"keep.txt","content":"changed"}"#,
        )
        .await
        .expect_err("denied");
        assert!(err.contains("不允许"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "original");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn read_only_rejects_write_without_mutating() {
        let root = std::env::temp_dir().join(format!(
            "codex-ai-readonly-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("mkdir");
        let path = root.join("keep.txt");
        std::fs::write(&path, "original").expect("write");
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: None,
            request_question: None,
            read_only: true,
            skills: Vec::new(),
            hooks: Vec::new(),
            on_mutation: None,
        };
        let err = execute_tool(
            &mut ctx,
            "Write",
            r#"{"file_path":"keep.txt","content":"changed"}"#,
        )
        .await
        .expect_err("read-only write");
        assert!(err.contains("只读规划模式禁止调用工具 Write"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "original");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ask_question_without_channel_errors() {
        let root = std::env::temp_dir().join("codex-ai-ask-none");
        let _ = std::fs::create_dir_all(&root);
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: None,
            request_question: None,
            read_only: true,
            skills: Vec::new(),
            hooks: Vec::new(),
            on_mutation: None,
        };
        let err = execute_tool(
            &mut ctx,
            "AskQuestion",
            r#"{"questions":[{"prompt":"用哪个？"}]}"#,
        )
        .await
        .expect_err("no channel");
        assert!(err.contains("计划提问"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn ask_question_returns_user_answers() {
        let root = std::env::temp_dir().join("codex-ai-ask-ok");
        let _ = std::fs::create_dir_all(&root);
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: None,
            request_question: Some(Arc::new(|_questions, tx| {
                let _ = tx.send(PlanQuestionAnswer {
                    skipped: false,
                    answers: vec!["用 A".to_string()],
                });
            })),
            read_only: true,
            skills: Vec::new(),
            hooks: Vec::new(),
            on_mutation: None,
        };
        let result = execute_tool(
            &mut ctx,
            "AskQuestion",
            r#"{"questions":[{"prompt":"用哪个？"}]}"#,
        )
        .await
        .expect("ask");
        assert!(result.contains("用 A"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn apply_patch_writes_and_pre_hook_can_block() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("codex-ai-patch-{stamp}"));
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("a.txt"), "hello\n").expect("write");
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: None,
            request_question: None,
            read_only: false,
            skills: Vec::new(),
            hooks: Vec::new(),
            on_mutation: None,
        };
        let patch = r#"{"patch":"*** Begin Patch\n*** Update File: a.txt\n@@\n-hello\n+hello world\n*** Add File: b.txt\n+new\n*** End Patch"}"#;
        let result = execute_tool(&mut ctx, "ApplyPatch", patch)
            .await
            .expect("patch");
        assert!(result.contains("新增 1"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).expect("a"),
            "hello world\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).expect("b"),
            "new\n"
        );

        ctx.hooks = vec![crate::db::models::NativeHook {
            id: "block".to_string(),
            event: crate::native::settings::HOOK_EVENT_PRE_TOOL_USE.to_string(),
            matcher: "Write".to_string(),
            command: "printf 'nope' >&2; exit 2".to_string(),
            timeout_secs: 10,
            enabled: true,
        }];
        let err = execute_tool(&mut ctx, "Write", r#"{"file_path":"c.txt","content":"x"}"#)
            .await
            .expect_err("blocked");
        assert!(err.contains("钩子阻断"));
        assert!(!root.join("c.txt").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn skill_tool_loads_named_skill() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("codex-ai-skill-tool-{stamp}"));
        std::fs::create_dir_all(&root).expect("mkdir");
        let mut ctx = ToolCtx {
            workspace: LocalWorkspace::new(root.clone()),
            ssh: None,
            cancel: CancelFlag::new(),
            read_files: HashSet::new(),
            todos: Vec::new(),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_permission: None,
            request_question: None,
            read_only: true,
            skills: vec![crate::native::skills::NativeSkill {
                name: "demo".to_string(),
                description: "desc".to_string(),
                source: crate::native::skills::SkillSource::Global,
                dir: "/skills/demo".to_string(),
                skill_md_path: "/skills/demo/SKILL.md".to_string(),
                body: "hello skill".to_string(),
                extra_files: vec!["notes.md".to_string()],
            }],
            hooks: Vec::new(),
            on_mutation: None,
        };
        let result = execute_tool(&mut ctx, "Skill", r#"{"name":"demo"}"#)
            .await
            .expect("skill");
        assert!(result.contains("hello skill"));
        assert!(result.contains("notes.md"));
        let err = execute_tool(&mut ctx, "Skill", r#"{"name":"missing"}"#)
            .await
            .expect_err("missing");
        assert!(err.contains("未找到"));
        let blocked = execute_tool(
            &mut ctx,
            "ApplyPatch",
            r#"{"patch":"*** Begin Patch\n*** Add File: x.txt\n+hi\n*** End Patch"}"#,
        )
        .await
        .expect_err("plan");
        assert!(blocked.contains("只读"));
        let _ = std::fs::remove_dir_all(root);
    }
}
