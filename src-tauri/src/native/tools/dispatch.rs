use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::cancel::CancelFlag;
use super::contract::{resolve_builtin_contract, ToolContract};
use super::hooks::{
    run_permission_request_hooks, run_post_tool_failure_hooks, run_post_tool_hooks,
    run_pre_tool_hooks, HookAgentHandler, HookDecision, HookRuntime,
};
use super::local::{
    apply_edit_fuzzy, format_read, image_mime_type, FileFingerprint, LocalWorkspace,
    EDIT_STRATEGY_EXACT,
};
use super::mcp::SharedMcp;
use super::patch::{extract_patch_text, parse_patch, patch_counts, plan_mutations, FileMutation};
use super::paths::resolve_under_workspace;
use super::permission::{
    classify_native_tool_risk, suggest_rule, NativePermissionDecision, NativeToolRisk,
    NativeToolRiskKind, PermissionRuleSuggestion, PermissionRules, RuleDecision,
};
use super::question::{format_ask_question_result, parse_ask_question_args, PlanQuestionAnswer};
use super::ssh::SshToolRuntime;
use crate::native::model::types::{NativeImage, ToolCall};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::oneshot;

/// 会话内共享的权限规则（命令层追加规则后即时生效）。
pub type SharedPermissionRules = Arc<RwLock<PermissionRules>>;

/// 会话级数据库作用域：自动化 / 目标 / 跨会话上下文工具需要它。
#[derive(Clone)]
pub struct SessionScope {
    pub pool: sqlx::SqlitePool,
    pub workspace_id: Option<String>,
    pub channel_id: String,
    pub model: String,
    /// 目标变更时通知会话层（写事件流行）。
    pub on_goal: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

/// ExitPlanMode 提交给用户审批的计划。
#[derive(Debug, Clone)]
pub struct PlanApprovalPrompt {
    pub request_id: String,
    pub plan: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct PlanApprovalAnswer {
    pub approved: bool,
    #[serde(default)]
    pub feedback: String,
}

pub type PlanApprovalRequester =
    Arc<dyn Fn(PlanApprovalPrompt, oneshot::Sender<PlanApprovalAnswer>) + Send + Sync>;

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
    /// 「总是允许」时建议保存的规则。
    pub suggested_rule: Option<PermissionRuleSuggestion>,
}

pub type PermissionRequester =
    Arc<dyn Fn(PermissionPrompt, oneshot::Sender<NativePermissionDecision>) + Send + Sync>;

pub type QuestionRequester =
    Arc<dyn Fn(Vec<super::PlanQuestion>, oneshot::Sender<PlanQuestionAnswer>) + Send + Sync>;

pub type PermissionExpirer =
    Arc<dyn Fn(String) -> tauri::async_runtime::JoinHandle<()> + Send + Sync>;

pub type MutationHook = Arc<dyn Fn(&str) + Send + Sync>;

/// 已读文件登记：key 是本地解析后的绝对路径或 SSH 侧的原始路径；
/// value 是读取时的指纹（SSH 无法取到时为 `None`）。
pub type ReadFileRegistry = Arc<Mutex<HashMap<String, Option<FileFingerprint>>>>;

/// 工具执行上下文。`Clone` 得到的副本共享同一份可变状态（已读文件、待办、MCP、
/// 权限放行），因此同一轮里并行执行的工具可以各拿一份副本。
#[derive(Clone)]
pub struct ToolCtx {
    pub workspace: LocalWorkspace,
    pub ssh: Option<SshToolRuntime>,
    pub extra_env: Vec<(String, String)>,
    pub cancel: CancelFlag,
    pub read_files: ReadFileRegistry,
    pub todos: Arc<Mutex<Vec<TodoItem>>>,
    pub mcp: SharedMcp,
    pub allow_all_high_risk: Arc<std::sync::atomic::AtomicBool>,
    pub auto_approve_overwrite: bool,
    pub allowed_mcp_servers: Arc<Mutex<HashSet<String>>>,
    pub request_permission: Option<PermissionRequester>,
    pub expire_permission: Option<PermissionExpirer>,
    pub permission_timeout: Duration,
    pub request_question: Option<QuestionRequester>,
    pub request_plan_approval: Option<PlanApprovalRequester>,
    /// 只读（计划）模式开关；`EnterPlanMode` / `ExitPlanMode` 在运行中切换，所以用共享原子量。
    pub read_only: Arc<AtomicBool>,
    /// 是否处于计划模式（决定 ExitPlanMode 是否可见、会话层是否等待自动实施）。
    pub plan_mode: Arc<AtomicBool>,
    /// build 模式：不透明 shell 命令免确认。
    pub auto_approve_opaque_bash: bool,
    /// build 模式：带 `readOnlyHint` 的 MCP 工具免确认。
    pub auto_approve_readonly_mcp: bool,
    pub permission_rules: SharedPermissionRules,
    pub skills: Vec<crate::native::skills::NativeSkill>,
    pub hooks: Vec<crate::db::models::NativeHook>,
    /// `agent` 类型钩子的判定器（由会话层用当前模型构造）。
    pub hook_agent: Option<HookAgentHandler>,
    /// 钩子载荷里的会话 id。
    pub session_record_id: String,
    pub on_mutation: Option<MutationHook>,
    /// 父 Agent 的后台任务注册表（TaskOutput / TaskStop / SendMessage）。
    pub background: Option<Arc<crate::native::agent::background::BackgroundTaskRegistry>>,
    /// 后台子 Agent：父注册表 + 自己的 task_id（RespondToCoordinator）。
    pub coordinator: Option<(
        Arc<crate::native::agent::background::BackgroundTaskRegistry>,
        String,
    )>,
    /// 数据库作用域（自动化 / 目标 / ReadSessionContext）；测试与子 Agent 可为空。
    pub session_scope: Option<SessionScope>,
}

impl ToolCtx {
    pub fn new(workspace: LocalWorkspace) -> Self {
        Self {
            workspace,
            ssh: None,
            extra_env: Vec::new(),
            cancel: CancelFlag::new(),
            read_files: Arc::new(Mutex::new(HashMap::new())),
            todos: Arc::new(Mutex::new(Vec::new())),
            mcp: SharedMcp::empty(),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            auto_approve_overwrite: false,
            allowed_mcp_servers: Arc::new(Mutex::new(HashSet::new())),
            request_permission: None,
            expire_permission: None,
            permission_timeout: Duration::ZERO,
            request_question: None,
            request_plan_approval: None,
            read_only: Arc::new(AtomicBool::new(false)),
            plan_mode: Arc::new(AtomicBool::new(false)),
            auto_approve_opaque_bash: false,
            auto_approve_readonly_mcp: false,
            permission_rules: Arc::new(RwLock::new(PermissionRules::default())),
            skills: Vec::new(),
            hooks: Vec::new(),
            hook_agent: None,
            session_record_id: String::new(),
            on_mutation: None,
            background: None,
            coordinator: None,
            session_scope: None,
        }
    }

    pub fn hook_runtime(&self) -> HookRuntime<'_> {
        HookRuntime {
            workspace: &self.workspace,
            ssh: self.ssh.as_ref(),
            cancel: &self.cancel,
            hooks: &self.hooks,
            extra_env: &self.extra_env,
            session_record_id: &self.session_record_id,
            agent_handler: self.hook_agent.as_ref(),
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only.load(Ordering::SeqCst)
    }

    pub fn set_read_only(&self, value: bool) {
        self.read_only.store(value, Ordering::SeqCst);
    }

    pub fn is_plan_mode(&self) -> bool {
        self.plan_mode.load(Ordering::SeqCst)
    }

    pub fn set_plan_mode(&self, value: bool) {
        self.plan_mode.store(value, Ordering::SeqCst);
    }

    /// 规则匹配用的工作区根：SSH 用远端路径，本地用工作区目录。
    pub fn rules_workspace_root(&self) -> PathBuf {
        match self.ssh.as_ref() {
            Some(ssh) => PathBuf::from(&ssh.root),
            None => self.workspace.root.clone(),
        }
    }

    pub fn permission_rules_snapshot(&self) -> PermissionRules {
        self.permission_rules
            .read()
            .map(|rules| rules.clone())
            .unwrap_or_default()
    }

    pub fn mark_read(&self, key: impl Into<String>, fingerprint: Option<FileFingerprint>) {
        if let Ok(mut files) = self.read_files.lock() {
            files.insert(key.into(), fingerprint);
        }
    }

    pub fn has_read(&self, key: &str) -> bool {
        self.read_files
            .lock()
            .map(|files| files.contains_key(key))
            .unwrap_or(false)
    }

    pub fn read_fingerprint(&self, key: &str) -> Option<FileFingerprint> {
        self.read_files
            .lock()
            .ok()
            .and_then(|files| files.get(key).copied().flatten())
    }

    pub fn todos_snapshot(&self) -> Vec<TodoItem> {
        self.todos
            .lock()
            .map(|todos| todos.clone())
            .unwrap_or_default()
    }

    /// 派生一份给子 Agent 的上下文：共享取消 / 权限 / MCP 放行状态，但已读文件
    /// 与待办清单是独立的。
    pub fn fork_for_child(&self) -> Self {
        let mut child = self.clone();
        child.read_files = Arc::new(Mutex::new(HashMap::new()));
        child.todos = Arc::new(Mutex::new(Vec::new()));
        child.mcp = SharedMcp::empty();
        child.read_only = Arc::new(AtomicBool::new(false));
        child.plan_mode = Arc::new(AtomicBool::new(false));
        child.request_question = None;
        child.request_plan_approval = None;
        child.on_mutation = None;
        child.background = None;
        child.coordinator = None;
        child
    }

    /// 解析工具契约：内置工具查注册表，MCP 工具按 annotations 生成。
    pub async fn contract_for(&self, name: &str) -> ToolContract {
        if let Some(contract) = super::contract::builtin_contract(name) {
            return contract.clone();
        }
        if let Some(contract) = self.mcp.contract_for(name).await {
            return contract;
        }
        resolve_builtin_contract(name)
    }
}

/// 工具结果：文本 + 可选图片（Read 图片文件时返回）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolOutput {
    pub text: String,
    pub images: Vec<NativeImage>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }
}

/// 兼容旧签名：只返回文本。
pub async fn execute_tool(ctx: &ToolCtx, name: &str, arguments: &str) -> Result<String, String> {
    let call = ToolCall {
        id: String::new(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    };
    execute_tool_call(ctx, &call)
        .await
        .map(|output| output.text)
}

pub async fn execute_tool_call(ctx: &ToolCtx, call: &ToolCall) -> Result<ToolOutput, String> {
    let name = call.name.as_str();
    let arguments = call.arguments.as_str();
    if ctx.cancel.is_cancelled() {
        return Err("已取消".to_string());
    }
    let contract = ctx.contract_for(name).await;
    if ctx.is_read_only() && !contract.allowed_in_plan_mode {
        return Err(format!("只读规划模式禁止调用工具 {name}"));
    }
    let pre = run_pre_tool_hooks(&ctx.hook_runtime(), name, arguments).await?;
    // 钩子可以改写参数（例如把危险命令替换成安全版本）。
    let arguments: &str = pre.updated_arguments.as_deref().unwrap_or(arguments);
    enforce_permissions(ctx, &contract, name, arguments).await?;
    let result = run_with_contract_timeout(ctx, &contract, name, arguments).await;
    match result {
        Ok(mut output) => {
            if matches!(name, "Write" | "Edit" | "ApplyPatch") {
                if let Some(on_mutation) = &ctx.on_mutation {
                    on_mutation(name);
                }
            }
            let post =
                run_post_tool_hooks(&ctx.hook_runtime(), name, arguments, &output.text).await;
            let mut extra_context: Vec<String> = pre.additional_context;
            extra_context.extend(post.additional_context);
            if !extra_context.is_empty() {
                output.text = format!(
                    "{}\n\n[钩子上下文]\n{}",
                    output.text,
                    extra_context.join("\n")
                );
            }
            if !post.warnings.is_empty() {
                output.text = format!(
                    "{}\n\n[钩子警告]\n{}",
                    output.text,
                    post.warnings.join("\n")
                );
            }
            Ok(output)
        }
        Err(error) => {
            let warnings =
                run_post_tool_failure_hooks(&ctx.hook_runtime(), name, arguments, &error).await;
            if warnings.is_empty() {
                Err(error)
            } else {
                Err(format!("{error}\n\n[钩子警告]\n{}", warnings.join("\n")))
            }
        }
    }
}

/// 按契约给工具执行加超时。需要等待用户的工具、Bash（自带超时）与 Agent 不包裹。
async fn run_with_contract_timeout(
    ctx: &ToolCtx,
    contract: &ToolContract,
    name: &str,
    arguments: &str,
) -> Result<ToolOutput, String> {
    let skip_outer_timeout = contract.requires_user_interaction
        || contract.timeout.is_unbounded()
        || contract.timeout.allow_call_override;
    if skip_outer_timeout {
        return dispatch(ctx, name, arguments).await;
    }
    let limit = Duration::from_millis(contract.timeout.default_ms.max(1));
    match tokio::time::timeout(limit, dispatch(ctx, name, arguments)).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "工具 {name} 执行超过 {} 秒已中止",
            limit.as_secs().max(1)
        )),
    }
}

async fn dispatch(ctx: &ToolCtx, name: &str, arguments: &str) -> Result<ToolOutput, String> {
    match name {
        "Read" => call_read(ctx, arguments).await,
        "Write" => call_write(ctx, arguments).await.map(ToolOutput::text),
        "Edit" => call_edit(ctx, arguments).await.map(ToolOutput::text),
        "ApplyPatch" => call_apply_patch(ctx, arguments).await.map(ToolOutput::text),
        "Glob" => call_glob(ctx, arguments).await.map(ToolOutput::text),
        "Grep" => call_grep(ctx, arguments).await.map(ToolOutput::text),
        "Bash" => call_bash(ctx, arguments).await.map(ToolOutput::text),
        "TodoRead" => Ok(ToolOutput::text(format_todos(&ctx.todos_snapshot()))),
        "TodoWrite" => call_todo_write(ctx, arguments).map(ToolOutput::text),
        "WebFetch" => super::web::web_fetch(arguments).await.map(ToolOutput::text),
        "WebSearch" => super::web::web_search(arguments)
            .await
            .map(ToolOutput::text),
        "AskQuestion" | "AskUserQuestion" => call_ask_question(ctx, arguments)
            .await
            .map(ToolOutput::text),
        "EnterPlanMode" => call_enter_plan_mode(ctx).map(ToolOutput::text),
        "ExitPlanMode" => call_exit_plan_mode(ctx, arguments)
            .await
            .map(ToolOutput::text),
        "TaskOutput" => call_task_output(ctx, arguments).await.map(ToolOutput::text),
        "TaskStop" => call_task_stop(ctx, arguments).map(ToolOutput::text),
        "SendMessage" => call_send_message(ctx, arguments)
            .await
            .map(ToolOutput::text),
        "RespondToCoordinator" => call_respond_to_coordinator(ctx, arguments).map(ToolOutput::text),
        "CronCreate" => call_cron_create(ctx, arguments).await.map(ToolOutput::text),
        "CronList" => call_cron_list(ctx).await.map(ToolOutput::text),
        "CronDelete" => call_cron_delete(ctx, arguments).await.map(ToolOutput::text),
        "Goal" => call_goal(ctx, arguments).await.map(ToolOutput::text),
        "GoalRead" => call_goal_read(ctx).await.map(ToolOutput::text),
        "ReadSessionContext" => call_read_session_context(ctx, arguments)
            .await
            .map(ToolOutput::text),
        "Skill" => call_skill(ctx, arguments).map(ToolOutput::text),
        other if ctx.mcp.has_tool(other).await => {
            ctx.mcp.call(other, arguments).await.map(ToolOutput::text)
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

async fn call_ask_question(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
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

fn session_scope(ctx: &ToolCtx) -> Result<&SessionScope, String> {
    ctx.session_scope
        .as_ref()
        .ok_or_else(|| "当前会话没有数据库作用域，无法使用该工具".to_string())
}

fn scope_workspace(scope: &SessionScope) -> Result<&str, String> {
    scope
        .workspace_id
        .as_deref()
        .ok_or_else(|| "当前会话没有绑定工作区".to_string())
}

async fn call_cron_create(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    let workspace_id = scope_workspace(scope)?;
    let args = parse_args(arguments)?;
    let created = crate::native::scheduler::create_automation(
        &scope.pool,
        crate::native::scheduler::CreateNativeAutomation {
            workspace_id: workspace_id.to_string(),
            name: string_arg(&args, "name")?,
            prompt: string_arg(&args, "prompt")?,
            cron: string_arg(&args, "cron")?,
            channel_id: Some(scope.channel_id.clone()),
            model: Some(scope.model.clone()),
            enabled: Some(true),
        },
    )
    .await?;
    Ok(format!(
        "已创建自动化 {}「{}」，cron=`{}`，下次运行：{}",
        created.id,
        created.name,
        created.cron,
        created.next_run_at.as_deref().unwrap_or("-")
    ))
}

async fn call_cron_list(ctx: &ToolCtx) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    let workspace_id = scope_workspace(scope)?;
    let items = crate::native::scheduler::list_automations(&scope.pool, Some(workspace_id)).await?;
    Ok(crate::native::scheduler::format_automation_list(&items))
}

async fn call_cron_delete(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    let workspace_id = scope_workspace(scope)?;
    let args = parse_args(arguments)?;
    let id = string_arg(&args, "id")?;
    let automation = crate::native::scheduler::get_automation(&scope.pool, &id).await?;
    if automation.workspace_id != workspace_id {
        return Err("只能删除当前工作区的自动化".to_string());
    }
    crate::native::scheduler::delete_automation(&scope.pool, &id).await?;
    Ok(format!("已删除自动化 {id}「{}」", automation.name))
}

async fn call_goal(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    if ctx.session_record_id.trim().is_empty() {
        return Err("当前会话没有记录 id，无法保存目标".to_string());
    }
    let args = parse_args(arguments)?;
    let action = string_arg(&args, "action")?;
    let title = args.get("title").and_then(Value::as_str);
    let note = args.get("note").and_then(Value::as_str);
    let checklist = crate::native::goals::parse_checklist(args.get("checklist"));
    let goal = crate::native::goals::apply_goal_action(
        &scope.pool,
        &ctx.session_record_id,
        scope.workspace_id.as_deref(),
        &action,
        title,
        checklist,
        note,
    )
    .await?;
    match goal {
        Some(goal) => {
            if let Some(on_goal) = &scope.on_goal {
                on_goal(goal.line());
            }
            Ok(goal.describe())
        }
        None => {
            if let Some(on_goal) = &scope.on_goal {
                on_goal(format!(
                    "{}{{\"cleared\":true}}",
                    crate::native::goals::GOAL_LINE_PREFIX
                ));
            }
            Ok("已清除会话目标。".to_string())
        }
    }
}

async fn call_goal_read(ctx: &ToolCtx) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    match crate::native::goals::current_goal(&scope.pool, &ctx.session_record_id).await? {
        Some(goal) => Ok(goal.describe()),
        None => Ok("当前没有设置目标。用 Goal(action=set) 设置。".to_string()),
    }
}

async fn call_read_session_context(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let scope = session_scope(ctx)?;
    let args = parse_args(arguments)?;
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 60) as usize;
    match args
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        Some(session_id) => {
            crate::native::goals::session_digest(&scope.pool, session_id, limit).await
        }
        None => {
            let workspace_id = scope_workspace(scope)?;
            let items = crate::native::goals::list_recent_sessions(
                &scope.pool,
                workspace_id,
                &ctx.session_record_id,
                limit,
            )
            .await?;
            Ok(crate::native::goals::format_session_list(&items))
        }
    }
}

fn background_registry(
    ctx: &ToolCtx,
) -> Result<&Arc<crate::native::agent::background::BackgroundTaskRegistry>, String> {
    ctx.background
        .as_ref()
        .ok_or_else(|| "当前会话没有后台任务能力（只有主 Agent 可以管理后台任务）".to_string())
}

async fn call_task_output(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let registry = background_registry(ctx)?;
    let args = parse_args(arguments)?;
    let task_id = string_arg(&args, "task_id")?;
    let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(true);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_i64)
        .unwrap_or(30_000)
        .clamp(0, 600_000) as u64;
    if registry.get(&task_id).is_none() {
        let known: Vec<String> = registry.list().iter().map(|task| task.id.clone()).collect();
        return Err(format!(
            "未知任务：{task_id}。当前任务：{}",
            if known.is_empty() {
                "（无）".to_string()
            } else {
                known.join("、")
            }
        ));
    }
    if wait {
        let waiter = registry.wait(&task_id, Duration::from_millis(timeout_ms));
        tokio::select! {
            biased;
            _ = async {
                loop {
                    if ctx.cancel.is_cancelled() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => return Err("已取消".to_string()),
            _ = waiter => {}
        }
    }
    registry
        .describe(&task_id)
        .ok_or_else(|| format!("未知任务：{task_id}"))
}

fn call_task_stop(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let registry = background_registry(ctx)?;
    let args = parse_args(arguments)?;
    let task_id = string_arg(&args, "task_id")?;
    match registry.stop(&task_id) {
        Some(true) => Ok(format!("已停止任务 {task_id}")),
        Some(false) => Ok(format!("任务 {task_id} 早已结束，无需停止")),
        None => Err(format!("未知任务：{task_id}")),
    }
}

async fn call_send_message(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let registry = background_registry(ctx)?;
    let args = parse_args(arguments)?;
    let task_id = string_arg(&args, "task_id")?;
    let message = string_arg(&args, "message")?;
    registry.send_message(&task_id, &message).await?;
    Ok(format!(
        "已把消息发给任务 {task_id}，它会在下一次模型调用前读到。"
    ))
}

fn call_respond_to_coordinator(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let Some((registry, task_id)) = ctx.coordinator.as_ref() else {
        return Err("当前不是后台子 Agent，没有可回复的父 Agent".to_string());
    };
    let args = parse_args(arguments)?;
    let message = string_arg(&args, "message")?;
    registry.push_inbox(task_id, &message);
    Ok("已留言给父 Agent。请继续完成任务，除非任务已完成。".to_string())
}

fn call_enter_plan_mode(ctx: &ToolCtx) -> Result<String, String> {
    if ctx.is_plan_mode() {
        return Ok("已经处于计划模式（只读）。".to_string());
    }
    ctx.set_read_only(true);
    ctx.set_plan_mode(true);
    Ok("已进入计划模式：只能使用只读工具摸底。计划写好后调用 ExitPlanMode 提交，等用户批准再实施。".to_string())
}

async fn call_exit_plan_mode(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    if !ctx.is_plan_mode() {
        return Err("当前不在计划模式，无需 ExitPlanMode".to_string());
    }
    let args = parse_args(arguments)?;
    let plan = string_arg(&args, "plan")?;
    let Some(requester) = ctx.request_plan_approval.clone() else {
        // 无交互通道（无头会话）时视为自动批准。
        ctx.set_read_only(false);
        ctx.set_plan_mode(false);
        return Ok("计划已记录（无人值守，自动批准），进入实施。".to_string());
    };
    let (tx, rx) = oneshot::channel();
    requester(
        PlanApprovalPrompt {
            request_id: uuid::Uuid::new_v4().to_string(),
            plan: plan.clone(),
        },
        tx,
    );
    let answer = tokio::select! {
        biased;
        _ = async {
            loop {
                if ctx.cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => return Err("已取消".to_string()),
        result = rx => result.map_err(|_| "已取消".to_string())?,
    };
    if answer.approved {
        ctx.set_read_only(false);
        ctx.set_plan_mode(false);
        Ok(if answer.feedback.trim().is_empty() {
            "用户已批准计划，进入实施。".to_string()
        } else {
            format!(
                "用户已批准计划并补充：{}\n进入实施。",
                answer.feedback.trim()
            )
        })
    } else if answer.feedback.trim().is_empty() {
        Ok("用户未批准计划。请根据对话调整计划后再次调用 ExitPlanMode，或用 AskUserQuestion 澄清。".to_string())
    } else {
        Ok(format!(
            "用户未批准计划，反馈：{}\n请据此修改计划后再次调用 ExitPlanMode。",
            answer.feedback.trim()
        ))
    }
}

/// 规则层 → 风险分类 → 模式默认。
async fn enforce_permissions(
    ctx: &ToolCtx,
    contract: &ToolContract,
    name: &str,
    arguments: &str,
) -> Result<(), String> {
    let root = ctx.rules_workspace_root();
    let decision = ctx
        .permission_rules
        .read()
        .map(|rules| rules.evaluate(contract, name, arguments, Some(&root)))
        .unwrap_or(RuleDecision::NoMatch);
    match decision {
        RuleDecision::Deny(rule) => {
            return Err(format!(
                "权限规则拒绝：{} 命中 deny 规则 `{}`",
                contract.permission.as_str(),
                rule.pattern
            ));
        }
        RuleDecision::Allow(_) => return Ok(()),
        RuleDecision::Ask(rule) => {
            return request_permission(
                ctx,
                contract,
                name,
                arguments,
                NativeToolRiskKind::Rule,
                format!(
                    "规则 `{}` 要求确认：{}",
                    rule.pattern,
                    call_brief(name, arguments)
                ),
            )
            .await;
        }
        RuleDecision::NoMatch => {}
    }
    let exists = match name {
        "Write" => write_target_exists(ctx, arguments).await,
        _ => None,
    };
    let is_mcp = ctx.mcp.has_tool(name).await || name.starts_with("mcp_");
    // 自动化写操作没有启发式分类，直接按契约要求确认。
    if contract.needs_approval
        && contract.permission == super::contract::PermissionCapability::AutomationWrite
    {
        return request_permission(
            ctx,
            contract,
            name,
            arguments,
            NativeToolRiskKind::Automation,
            format!("自动化变更：{}", call_brief(name, arguments)),
        )
        .await;
    }
    match classify_native_tool_risk(name, arguments, exists, is_mcp) {
        NativeToolRisk::Low => Ok(()),
        NativeToolRisk::High { kind, summary } => {
            if kind == NativeToolRiskKind::Opaque && ctx.auto_approve_opaque_bash {
                return Ok(());
            }
            if kind == NativeToolRiskKind::Mcp
                && ctx.auto_approve_readonly_mcp
                && contract.read_only
            {
                return Ok(());
            }
            request_permission(ctx, contract, name, arguments, kind, summary).await
        }
    }
}

fn call_brief(name: &str, arguments: &str) -> String {
    let args = serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null);
    for key in ["command", "file_path", "path", "url", "query"] {
        if let Some(value) = args.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                let shown: String = trimmed.chars().take(160).collect();
                return format!("{name} {shown}");
            }
        }
    }
    name.to_string()
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
        return Some(ctx.has_read(&path));
    }
    ctx.workspace
        .resolve(&path)
        .ok()
        .map(|resolved| resolved.exists())
}

async fn request_permission(
    ctx: &ToolCtx,
    contract: &ToolContract,
    name: &str,
    arguments: &str,
    kind: NativeToolRiskKind,
    summary: String,
) -> Result<(), String> {
    let mcp_server_id = if kind == NativeToolRiskKind::Mcp {
        ctx.mcp.server_id_for_tool(name).await
    } else {
        None
    };
    // ask 规则显式要求确认，不受 yolo / 会话放行影响。
    let rule_forced = kind == NativeToolRiskKind::Rule;
    if !rule_forced
        && kind != NativeToolRiskKind::Mcp
        && ctx.allow_all_high_risk.load(Ordering::SeqCst)
    {
        return Ok(());
    }
    if kind == NativeToolRiskKind::Overwrite && ctx.auto_approve_overwrite {
        return Ok(());
    }
    // permission_request 钩子可以代替用户直接给出 allow / deny；ask 则继续弹窗。
    if let Some(hook_decision) =
        run_permission_request_hooks(&ctx.hook_runtime(), name, arguments, kind, &summary).await
    {
        match hook_decision.decision {
            HookDecision::Allow => return Ok(()),
            HookDecision::Deny => {
                return Err(match hook_decision.reason {
                    Some(reason) => format!("钩子 {} 拒绝：{reason}", hook_decision.hook_id),
                    None => format!("钩子 {} 拒绝了该操作", hook_decision.hook_id),
                });
            }
            HookDecision::Ask => {}
        }
    }
    let suggested_rule = suggest_rule(contract, name, arguments, Some(&ctx.rules_workspace_root()));
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
        suggested_rule: suggested_rule.clone(),
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
            ctx.allow_all_high_risk.store(true, Ordering::SeqCst);
            Ok(())
        }
        NativePermissionDecision::AllowServer => {
            allow_mcp_server(ctx, name).await;
            Ok(())
        }
        NativePermissionDecision::AllowAlways => {
            // 持久化由会话层完成；这里先把规则塞进内存，本会话内立刻生效。
            if let Some(suggestion) = suggested_rule {
                if let Ok(mut rules) = ctx.permission_rules.write() {
                    rules.push(
                        super::permission::RuleEffect::Allow,
                        super::permission::PermissionRule {
                            id: uuid::Uuid::new_v4().to_string(),
                            capability: suggestion.capability,
                            pattern: suggestion.pattern,
                            source: suggestion.source,
                            scope: super::permission::RuleScope::Workspace,
                            note: String::new(),
                        },
                    );
                }
            }
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

async fn call_apply_patch(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
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
                    ctx.mark_read(path.clone(), None);
                } else {
                    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
                    ctx.workspace.write_file(&path, &content)?;
                    ctx.mark_read(
                        resolved.to_string_lossy().into_owned(),
                        Some(FileFingerprint::of_content(&resolved, content.as_bytes())),
                    );
                }
                notes.push(format!("wrote {path}"));
            }
            FileMutation::Delete { path } => {
                if let Some(ssh) = ctx.ssh.as_ref() {
                    ssh.delete(&path).await?;
                    ctx.mark_read(path.clone(), None);
                } else {
                    let resolved = resolve_under_workspace(&ctx.workspace.root, &path)?;
                    ctx.workspace.delete_file(&path)?;
                    ctx.mark_read(resolved.to_string_lossy().into_owned(), None);
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

async fn call_read(ctx: &ToolCtx, arguments: &str) -> Result<ToolOutput, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let offset = args.get("offset").and_then(Value::as_i64);
    let limit = args.get("limit").and_then(Value::as_i64);
    if let Some(ssh) = ctx.ssh.as_ref() {
        let raw = ssh.read(&path).await?;
        ctx.mark_read(path.clone(), None);
        return Ok(ToolOutput::text(format_read(&raw, offset, limit)));
    }
    let resolved = ctx.workspace.resolve_for_read(&path)?;
    if image_mime_type(&resolved).is_some() {
        let image = ctx.workspace.read_image(&path)?;
        ctx.mark_read(
            resolved.to_string_lossy().into_owned(),
            FileFingerprint::of_path(&resolved),
        );
        let text = format!(
            "[图片] {}（{}，{} 字节）已作为图片附件提供给模型。",
            image.name,
            image.mime_type,
            image.data_base64.len() * 3 / 4
        );
        return Ok(ToolOutput {
            text,
            images: vec![image],
        });
    }
    let output = ctx.workspace.read_file(&path, offset, limit)?;
    ctx.mark_read(
        resolved.to_string_lossy().into_owned(),
        FileFingerprint::of_path(&resolved),
    );
    Ok(ToolOutput::text(output))
}

/// 本地写入 / 编辑前的新鲜度校验：文件存在则必须先 Read，且读后未被别人改过。
fn ensure_fresh_for_mutation(
    ctx: &ToolCtx,
    resolved: &std::path::Path,
    verb: &str,
) -> Result<(), String> {
    if !resolved.exists() {
        return Ok(());
    }
    let key = resolved.to_string_lossy().into_owned();
    if !ctx.has_read(&key) {
        return Err(format!(
            "File has not been read yet. Read it first before {verb}."
        ));
    }
    if let Some(recorded) = ctx.read_fingerprint(&key) {
        if let Some(current) = FileFingerprint::of_path(resolved) {
            if current != recorded {
                return Err(format!(
                    "文件自上次 Read 后已被修改（{}），请重新 Read 再{}。",
                    resolved.display(),
                    if verb == "writing to it" {
                        "写入"
                    } else {
                        "编辑"
                    }
                ));
            }
        }
    }
    Ok(())
}

async fn call_write(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let content = string_arg(&args, "content")?;
    if let Some(ssh) = ctx.ssh.as_ref() {
        if !ctx.has_read(&path) {
            return Err(
                "File has not been read yet. Read it first before writing to it.".to_string(),
            );
        }
        let output = ssh.write(&path, &content).await?;
        ctx.mark_read(path, None);
        return Ok(output);
    }
    let resolved = ctx.workspace.resolve_for_write(&path)?;
    ensure_fresh_for_mutation(ctx, &resolved, "writing to it")?;
    let output = ctx.workspace.write_file(&path, &content)?;
    ctx.mark_read(
        resolved.to_string_lossy().into_owned(),
        Some(FileFingerprint::of_content(&resolved, content.as_bytes())),
    );
    Ok(output)
}

async fn call_edit(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let path = string_arg(&args, "file_path")?;
    let old = string_arg(&args, "old_string")?;
    let new = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| "new_string 不能为空".to_string())?
        .to_string();
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(ssh) = ctx.ssh.as_ref() {
        if !ctx.has_read(&path) {
            return Err("File has not been read yet. Read it first before editing.".to_string());
        }
        let original = ssh.read(&path).await?;
        let outcome = apply_edit_fuzzy(&original, &old, &new, replace_all)?;
        ssh.write(&path, &outcome.content).await?;
        return Ok(edit_summary(&path, outcome.strategy, outcome.replacements));
    }
    let resolved = ctx.workspace.resolve_for_write(&path)?;
    if !resolved.exists() {
        return Err(match super::local::similar_filename_hint(&resolved) {
            Some(hint) => format!("文件不存在: {path}（{hint}）"),
            None => format!("文件不存在: {path}"),
        });
    }
    ensure_fresh_for_mutation(ctx, &resolved, "editing")?;
    let original =
        std::fs::read_to_string(&resolved).map_err(|error| format!("读取失败: {error}"))?;
    let outcome = apply_edit_fuzzy(&original, &old, &new, replace_all)?;
    ctx.workspace.write_file(&path, &outcome.content)?;
    ctx.mark_read(
        resolved.to_string_lossy().into_owned(),
        Some(FileFingerprint::of_content(
            &resolved,
            outcome.content.as_bytes(),
        )),
    );
    Ok(edit_summary(
        &resolved.display().to_string(),
        outcome.strategy,
        outcome.replacements,
    ))
}

fn edit_summary(path: &str, strategy: &str, replacements: usize) -> String {
    if strategy == EDIT_STRATEGY_EXACT {
        if replacements > 1 {
            format!("Edited {path}（替换 {replacements} 处）")
        } else {
            format!("Edited {path}")
        }
    } else {
        format!("Edited {path}（匹配策略：{strategy}，替换 {replacements} 处）")
    }
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
    ctx.workspace
        .bash(&command, timeout, &ctx.cancel, &ctx.extra_env)
        .await
}

fn call_todo_write(ctx: &ToolCtx, arguments: &str) -> Result<String, String> {
    let args = parse_args(arguments)?;
    let todos: Vec<TodoItem> =
        serde_json::from_value(args.get("todos").cloned().unwrap_or(Value::Null))
            .map_err(|_| "todos 必须是数组".to_string())?;
    let normalized: Vec<TodoItem> = todos
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
    let rendered = format_todos(&normalized);
    if let Ok(mut todos) = ctx.todos.lock() {
        *todos = normalized;
    }
    Ok(rendered)
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
    use tokio::sync::oneshot;

    fn temp_root(prefix: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("mkdir");
        root
    }

    fn ctx_for(root: &std::path::Path) -> ToolCtx {
        ToolCtx::new(LocalWorkspace::new(root.to_path_buf()))
    }

    fn deny_requester() -> PermissionRequester {
        Arc::new(|_prompt, tx: oneshot::Sender<NativePermissionDecision>| {
            let _ = tx.send(NativePermissionDecision::Deny);
        })
    }

    #[tokio::test]
    async fn deny_keeps_existing_file() {
        let root = temp_root("codex-ai-perm");
        let path = root.join("keep.txt");
        std::fs::write(&path, "original").expect("write");
        let mut ctx = ctx_for(&root);
        ctx.request_permission = Some(deny_requester());
        let err = execute_tool(
            &ctx,
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
        let root = temp_root("codex-ai-readonly");
        let path = root.join("keep.txt");
        std::fs::write(&path, "original").expect("write");
        let ctx = ctx_for(&root);
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        ctx.set_read_only(true);
        let err = execute_tool(
            &ctx,
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
        let root = temp_root("codex-ai-ask-none");
        let ctx = ctx_for(&root);
        ctx.set_read_only(true);
        let err = execute_tool(
            &ctx,
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
        let root = temp_root("codex-ai-ask-ok");
        let mut ctx = ctx_for(&root);
        ctx.set_read_only(true);
        ctx.request_question = Some(Arc::new(|_questions, tx| {
            let _ = tx.send(PlanQuestionAnswer {
                skipped: false,
                answers: vec!["用 A".to_string()],
            });
        }));
        let result = execute_tool(
            &ctx,
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
        let root = temp_root("codex-ai-patch");
        std::fs::write(root.join("a.txt"), "hello\n").expect("write");
        let mut ctx = ctx_for(&root);
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let patch = r#"{"patch":"*** Begin Patch\n*** Update File: a.txt\n@@\n-hello\n+hello world\n*** Add File: b.txt\n+new\n*** End Patch"}"#;
        let result = execute_tool(&ctx, "ApplyPatch", patch)
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

        ctx.hooks = vec![crate::db::models::NativeHook::shell(
            "block",
            crate::native::settings::HOOK_EVENT_PRE_TOOL_USE,
            "Write",
            "printf 'nope' >&2; exit 2",
            10,
            true,
        )];
        let err = execute_tool(&ctx, "Write", r#"{"file_path":"c.txt","content":"x"}"#)
            .await
            .expect_err("blocked");
        assert!(err.contains("钩子阻断"));
        assert!(!root.join("c.txt").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn skill_tool_loads_named_skill() {
        let root = temp_root("codex-ai-skill-tool");
        let mut ctx = ctx_for(&root);
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        ctx.set_read_only(true);
        ctx.skills = vec![crate::native::skills::NativeSkill {
            name: "demo".to_string(),
            description: "desc".to_string(),
            source: crate::native::skills::SkillSource::Global,
            dir: "/skills/demo".to_string(),
            skill_md_path: "/skills/demo/SKILL.md".to_string(),
            body: "hello skill".to_string(),
            extra_files: vec!["notes.md".to_string()],
            allowed_tools: Vec::new(),
            argument_hint: None,
            when_to_use: None,
            plugin: None,
        }];
        let result = execute_tool(&ctx, "Skill", r#"{"name":"demo"}"#)
            .await
            .expect("skill");
        assert!(result.contains("hello skill"));
        assert!(result.contains("notes.md"));
        let err = execute_tool(&ctx, "Skill", r#"{"name":"missing"}"#)
            .await
            .expect_err("missing");
        assert!(err.contains("未找到"));
        let blocked = execute_tool(
            &ctx,
            "ApplyPatch",
            r#"{"patch":"*** Begin Patch\n*** Add File: x.txt\n+hi\n*** End Patch"}"#,
        )
        .await
        .expect_err("plan");
        assert!(blocked.contains("只读"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn auto_edit_allows_overwrite_but_still_prompts_delete() {
        let root = temp_root("codex-ai-auto-edit");
        let path = root.join("keep.txt");
        std::fs::write(&path, "original").expect("write");
        let mut ctx = ctx_for(&root);
        ctx.auto_approve_overwrite = true;
        ctx.request_permission = Some(deny_requester());
        execute_tool(&ctx, "Read", r#"{"file_path":"keep.txt"}"#)
            .await
            .expect("read");
        execute_tool(
            &ctx,
            "Write",
            r#"{"file_path":"keep.txt","content":"changed"}"#,
        )
        .await
        .expect("overwrite allowed");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "changed");
        let err = execute_tool(&ctx, "Bash", r#"{"command":"rm keep.txt"}"#)
            .await
            .expect_err("delete still confirmed");
        assert!(err.contains("不允许"));
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn edit_rejects_stale_read_and_reports_strategy() {
        let root = temp_root("codex-ai-fresh");
        let path = root.join("code.rs");
        std::fs::write(&path, "fn a() {\n    one();\n}\n").expect("write");
        let ctx = ctx_for(&root);
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        execute_tool(&ctx, "Read", r#"{"file_path":"code.rs"}"#)
            .await
            .expect("read");
        // 外部修改后再编辑应被拒绝。
        std::fs::write(&path, "fn a() {\n    one();\n}\n// touched\n").expect("touch");
        let err = execute_tool(
            &ctx,
            "Edit",
            r#"{"file_path":"code.rs","old_string":"one();","new_string":"two();"}"#,
        )
        .await
        .expect_err("stale");
        assert!(err.contains("已被修改"), "{err}");
        // 重新读取后，带行号的 old_string 也能命中并报告策略。
        execute_tool(&ctx, "Read", r#"{"file_path":"code.rs"}"#)
            .await
            .expect("read again");
        let result = execute_tool(
            &ctx,
            "Edit",
            r#"{"file_path":"code.rs","old_string":"     2\t    one();","new_string":"    two();"}"#,
        )
        .await
        .expect("edit");
        assert!(result.contains("line_number_prefix_stripped"), "{result}");
        assert!(std::fs::read_to_string(&path)
            .expect("read")
            .contains("two();"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn read_image_returns_attachment_and_missing_file_hints() {
        let root = temp_root("codex-ai-image");
        std::fs::write(root.join("shot.png"), [0x89u8, b'P', b'N', b'G', 1, 2]).expect("write");
        let ctx = ctx_for(&root);
        let call = ToolCall {
            id: "1".to_string(),
            name: "Read".to_string(),
            arguments: r#"{"file_path":"shot.png"}"#.to_string(),
        };
        let output = execute_tool_call(&ctx, &call).await.expect("image");
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime_type, "image/png");
        assert!(output.text.contains("[图片]"));
        let err = execute_tool(&ctx, "Read", r#"{"file_path":"shoot.png"}"#)
            .await
            .expect_err("missing");
        assert!(err.contains("shot.png"), "{err}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn plan_mode_tools_toggle_read_only_and_auto_approve_headless() {
        let root = temp_root("codex-ai-plan-tools");
        std::fs::write(root.join("a.txt"), "x").expect("write");
        let ctx = ctx_for(&root);
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let err = execute_tool(&ctx, "ExitPlanMode", r#"{"plan":"p"}"#)
            .await
            .expect_err("not in plan mode");
        assert!(err.contains("不在计划模式"));
        let entered = execute_tool(&ctx, "EnterPlanMode", "{}")
            .await
            .expect("enter");
        assert!(entered.contains("计划模式"));
        assert!(ctx.is_read_only() && ctx.is_plan_mode());
        let blocked = execute_tool(&ctx, "Write", r#"{"file_path":"a.txt","content":"y"}"#)
            .await
            .expect_err("blocked in plan mode");
        assert!(blocked.contains("只读规划模式"));
        // 无审批通道时视为自动批准。
        let exited = execute_tool(&ctx, "ExitPlanMode", r#"{"plan":"1. 改 a.txt"}"#)
            .await
            .expect("exit");
        assert!(exited.contains("进入实施"));
        assert!(!ctx.is_read_only() && !ctx.is_plan_mode());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn exit_plan_mode_waits_for_user_and_honors_rejection() {
        let root = temp_root("codex-ai-plan-approval");
        let mut ctx = ctx_for(&root);
        ctx.set_read_only(true);
        ctx.set_plan_mode(true);
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        ctx.request_plan_approval = Some(Arc::new(move |prompt, tx| {
            sink.lock().expect("lock").push(prompt.plan.clone());
            let approve = prompt.plan.contains("v2");
            let _ = tx.send(PlanApprovalAnswer {
                approved: approve,
                feedback: if approve {
                    String::new()
                } else {
                    "先补测试".to_string()
                },
            });
        }));
        let rejected = execute_tool(&ctx, "ExitPlanMode", r#"{"plan":"v1"}"#)
            .await
            .expect("rejected");
        assert!(rejected.contains("未批准"));
        assert!(rejected.contains("先补测试"));
        assert!(ctx.is_plan_mode());
        let approved = execute_tool(&ctx, "ExitPlanMode", r#"{"plan":"v2"}"#)
            .await
            .expect("approved");
        assert!(approved.contains("已批准"));
        assert!(!ctx.is_plan_mode() && !ctx.is_read_only());
        assert_eq!(seen.lock().expect("lock").len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn permission_rules_deny_allow_and_ask_override_mode() {
        use super::super::contract::{PatternSource, PermissionCapability};
        use super::super::permission::{PermissionRule, RuleEffect, RuleScope};
        let root = temp_root("codex-ai-rules");
        std::fs::write(root.join("keep.txt"), "original").expect("write");
        let ctx = ctx_for(&root);
        // yolo：默认全放行。
        ctx.allow_all_high_risk
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let rule = |capability, pattern: &str, source| PermissionRule {
            id: pattern.to_string(),
            capability,
            pattern: pattern.to_string(),
            source,
            scope: RuleScope::Workspace,
            note: String::new(),
        };
        {
            let mut rules = ctx.permission_rules.write().expect("rules");
            rules.push(
                RuleEffect::Deny,
                rule(PermissionCapability::Bash, "rm*", PatternSource::Command),
            );
            rules.push(
                RuleEffect::Ask,
                rule(
                    PermissionCapability::Bash,
                    "git push*",
                    PatternSource::Command,
                ),
            );
        }
        let denied = execute_tool(&ctx, "Bash", r#"{"command":"rm keep.txt"}"#)
            .await
            .expect_err("deny rule");
        assert!(denied.contains("权限规则拒绝"), "{denied}");
        assert!(root.join("keep.txt").exists());
        // ask 规则即便在 yolo 下也要弹确认；这里的确认器一律拒绝。
        let asked = Arc::new(Mutex::new(Vec::<PermissionPrompt>::new()));
        let sink = asked.clone();
        let mut ctx = ctx;
        ctx.request_permission = Some(Arc::new(move |prompt, tx| {
            sink.lock().expect("lock").push(prompt);
            let _ = tx.send(NativePermissionDecision::Deny);
        }));
        let err = execute_tool(&ctx, "Bash", r#"{"command":"git push origin main"}"#)
            .await
            .expect_err("ask rule");
        assert!(err.contains("不允许"));
        let prompts = asked.lock().expect("lock").clone();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].kind, NativeToolRiskKind::Rule);
        assert_eq!(
            prompts[0]
                .suggested_rule
                .as_ref()
                .map(|item| item.pattern.as_str()),
            Some("git push*")
        );
        // allow 规则让本来要确认的覆盖直接通过。
        ctx.allow_all_high_risk
            .store(false, std::sync::atomic::Ordering::SeqCst);
        {
            let mut rules = ctx.permission_rules.write().expect("rules");
            rules.push(
                RuleEffect::Allow,
                rule(PermissionCapability::Edit, "keep.txt", PatternSource::Path),
            );
        }
        execute_tool(&ctx, "Read", r#"{"file_path":"keep.txt"}"#)
            .await
            .expect("read");
        execute_tool(
            &ctx,
            "Write",
            r#"{"file_path":"keep.txt","content":"changed"}"#,
        )
        .await
        .expect("allow rule skips prompt");
        assert_eq!(asked.lock().expect("lock").len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn always_allow_decision_adds_in_memory_rule() {
        let root = temp_root("codex-ai-always-allow");
        let mut ctx = ctx_for(&root);
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        ctx.request_permission = Some(Arc::new(move |_prompt, tx| {
            *counter.lock().expect("lock") += 1;
            let _ = tx.send(NativePermissionDecision::AllowAlways);
        }));
        // 第一次弹确认并选择「总是允许」；命令本身在临时目录里会失败，这里只关心审批。
        let _ = execute_tool(&ctx, "Bash", r#"{"command":"git push origin a"}"#).await;
        assert_eq!(*calls.lock().expect("lock"), 1);
        let rules = ctx.permission_rules_snapshot();
        assert_eq!(rules.allow.len(), 1);
        assert_eq!(rules.allow[0].pattern, "git push*");
        // 第二次命中 allow 规则，不再弹确认。
        let _ = execute_tool(&ctx, "Bash", r#"{"command":"git push origin b"}"#).await;
        assert_eq!(*calls.lock().expect("lock"), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn build_mode_auto_approves_opaque_bash_only() {
        let root = temp_root("codex-ai-build-mode");
        let mut ctx = ctx_for(&root);
        ctx.auto_approve_overwrite = true;
        ctx.auto_approve_opaque_bash = true;
        ctx.auto_approve_readonly_mcp = true;
        ctx.request_permission = Some(deny_requester());
        let opaque = execute_tool(&ctx, "Bash", r#"{"command":"echo $(pwd)"}"#)
            .await
            .expect("opaque auto-approved in build mode");
        assert!(opaque.contains(&root.to_string_lossy().to_string()));
        let delete = execute_tool(&ctx, "Bash", r#"{"command":"rm -rf nothing"}"#)
            .await
            .expect_err("delete still asks");
        assert!(delete.contains("不允许"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cloned_contexts_share_read_state_and_todos() {
        let root = temp_root("codex-ai-shared");
        std::fs::write(root.join("a.txt"), "x").expect("write");
        let ctx = ctx_for(&root);
        let twin = ctx.clone();
        execute_tool(&twin, "Read", r#"{"file_path":"a.txt"}"#)
            .await
            .expect("read");
        assert!(ctx.has_read(&root.join("a.txt").to_string_lossy()));
        execute_tool(
            &twin,
            "TodoWrite",
            r#"{"todos":[{"content":"do","status":"pending"}]}"#,
        )
        .await
        .expect("todo");
        assert_eq!(ctx.todos_snapshot().len(), 1);
        let child = ctx.fork_for_child();
        assert!(child.todos_snapshot().is_empty());
        assert!(!child.has_read(&root.join("a.txt").to_string_lossy()));
        let _ = std::fs::remove_dir_all(root);
    }
}
