#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::native::model::types::NativeImage;
use crate::native::permission_rules::SharedPermissionRules;
use crate::native::tools::dispatch::PlanApprovalAnswer;
use crate::native::tools::file_access::FileAccessSelection;
use crate::native::tools::permission::{
    NativePermissionDecision, NativeToolRiskKind, PermissionRule, PermissionRuleSuggestion,
    RuleEffect, RuleScope,
};
use crate::native::tools::question::{PlanQuestion, PlanQuestionAnswer};
use crate::native::tools::CancelFlag;

#[derive(Debug, Clone)]
pub struct NativeSessionInfo {
    pub profile_id: String,
    pub channel_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub session_record_id: String,
}

#[derive(Debug)]
pub enum NativeFollowup {
    Input {
        text: String,
        images: Vec<NativeImage>,
    },
    /// `/compact [指令]`：在等待输入或下一次模型调用前压缩上下文。
    Compact(NativeCompactionRequest),
    Finish,
}

#[derive(Debug)]
pub struct NativeCompactionRequest {
    pub instructions: Option<String>,
    pending: Arc<AtomicUsize>,
}

impl NativeCompactionRequest {
    pub fn new(instructions: Option<String>, pending: Arc<AtomicUsize>) -> Self {
        pending.fetch_add(1, Ordering::SeqCst);
        Self {
            instructions,
            pending,
        }
    }
}

impl Drop for NativeCompactionRequest {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

impl NativeFollowup {
    pub fn input(text: impl Into<String>) -> Self {
        Self::Input {
            text: text.into(),
            images: Vec::new(),
        }
    }

    pub fn input_with_images(text: impl Into<String>, images: Vec<NativeImage>) -> Self {
        Self::Input {
            text: text.into(),
            images,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub request_id: String,
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub tool_name: String,
    pub kind: NativeToolRiskKind,
    pub summary: String,
    pub remote: bool,
    pub mcp_server_id: Option<String>,
    pub suggested_rule: Option<PermissionRuleSuggestion>,
    pub file_access: Option<crate::native::tools::file_access::FileAccessPrompt>,
    pub allow_once_only: bool,
}

pub struct PendingPermission {
    pub request: PermissionRequest,
    pub reply: oneshot::Sender<NativePermissionDecision>,
}

#[derive(Debug, Clone)]
pub struct PlanApprovalRequest {
    pub request_id: String,
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub plan: String,
}

pub struct PendingPlanApproval {
    pub request: PlanApprovalRequest,
    pub reply: oneshot::Sender<PlanApprovalAnswer>,
}

#[derive(Debug, Clone)]
pub struct PlanQuestionRequest {
    pub request_id: String,
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub questions: Vec<PlanQuestion>,
}

pub struct PendingPlanQuestion {
    pub request: PlanQuestionRequest,
    pub reply: oneshot::Sender<PlanQuestionAnswer>,
}

pub struct NativeLiveSession {
    pub info: NativeSessionInfo,
    pub runtime: Option<crate::db::models::NativeSessionRuntime>,
    pub plan_mode: Arc<AtomicBool>,
    pub background: Option<Arc<crate::native::agent::background::BackgroundTaskRegistry>>,
    pub closing: bool,
    pub cancel: CancelFlag,
    pub followup_tx: mpsc::Sender<NativeFollowup>,
    pub input_queue: Arc<crate::native::input_queue::NativeInputQueue>,
    pub join: JoinHandle<()>,
    pub allow_all_high_risk: Arc<AtomicBool>,
    pub working: Arc<AtomicBool>,
    pub pending_compactions: Arc<AtomicUsize>,
    /// 与 `ToolCtx` 共享的规则；「总是允许」写入后即时生效。
    pub permission_rules: SharedPermissionRules,
    /// 权限规则存储根；SSH 使用本机按工作区隔离的目录。
    pub workspace_root: Option<std::path::PathBuf>,
    pub pending_permission: VecDeque<PendingPermission>,
    pub pending_question: VecDeque<PendingPlanQuestion>,
    pub pending_plan_approval: VecDeque<PendingPlanApproval>,
}

impl NativeLiveSession {
    pub fn runtime_snapshot(&self) -> Option<crate::db::models::NativeSessionRuntime> {
        self.runtime.clone().map(|mut runtime| {
            runtime.plan_mode = self.plan_mode.load(Ordering::SeqCst);
            runtime
        })
    }
}

#[derive(Default)]
pub struct NativeAgentManager {
    sessions: HashMap<String, NativeLiveSession>,
    operation_locks: HashMap<String, Weak<tokio::sync::Mutex<()>>>,
}

impl NativeAgentManager {
    pub fn new() -> Self {
        Self::default()
    }

    // Serialize archive/resume/input without holding the manager lock during session startup.
    pub(crate) fn session_operation_lock(
        &mut self,
        session_id: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        if let Some(lock) = self.operation_locks.get(session_id).and_then(Weak::upgrade) {
            return lock;
        }
        self.operation_locks
            .retain(|_, lock| lock.strong_count() > 0);
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        self.operation_locks
            .insert(session_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    pub(crate) fn session_is_busy(&self, session_id: &str) -> bool {
        self.get_session(session_id).is_some_and(|session| {
            session.closing
                // Keep compact requests busy across queue handoff and execution.
                || session.pending_compactions.load(Ordering::SeqCst) > 0
                || session.input_queue.is_busy(&session.working)
                || session.followup_tx.capacity() < session.followup_tx.max_capacity()
                || !session.pending_permission.is_empty()
                || !session.pending_question.is_empty()
                || !session.pending_plan_approval.is_empty()
                || session.background.as_ref().is_some_and(|registry| {
                    registry
                        .list()
                        .iter()
                        .any(|task| !task.status().is_finished())
                })
        })
    }

    pub fn add_session(&mut self, session: NativeLiveSession) {
        self.sessions
            .insert(session.info.session_record_id.clone(), session);
    }

    pub fn remove_session(&mut self, session_record_id: &str) -> Option<NativeLiveSession> {
        self.sessions.remove(session_record_id)
    }

    pub fn get_session(&self, session_record_id: &str) -> Option<&NativeLiveSession> {
        self.sessions.get(session_record_id)
    }

    pub fn get_session_mut(&mut self, session_record_id: &str) -> Option<&mut NativeLiveSession> {
        self.sessions.get_mut(session_record_id)
    }

    pub fn refresh_permission_rules(&self, config_dir: &std::path::Path) {
        for session in self.sessions.values() {
            let effective = crate::native::permission_rules::load_effective_rules(
                config_dir,
                session.workspace_root.as_deref(),
            );
            if let Ok(mut rules) = session.permission_rules.write() {
                *rules = effective;
            }
        }
    }

    pub fn save_permission_rules(
        &self,
        config_dir: &std::path::Path,
        session_id: &str,
        request_id: &str,
        selections: Option<&[FileAccessSelection]>,
        scope: Option<RuleScope>,
    ) -> Result<(), String> {
        let session = self
            .get_session(session_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let pending = session
            .pending_permission
            .front()
            .filter(|pending| {
                pending.request.request_id == request_id
                    && !pending.reply.is_closed()
                    && !session.cancel.is_cancelled()
            })
            .ok_or_else(|| "权限确认请求已过期".to_string())?;
        if pending.request.allow_once_only {
            return Err("计划模式下的 Bash 命令仅允许本次授权，不能保存白名单".to_string());
        }
        let root = session.workspace_root.as_deref();
        let scope = scope.unwrap_or(if root.is_some() {
            RuleScope::Workspace
        } else {
            RuleScope::Global
        });
        let rules = if let Some(access) = &pending.request.file_access {
            let rules = access.rules_for_selection(
                selections.ok_or_else(|| "请选择白名单范围".to_string())?,
                scope,
            )?;
            if access.target == crate::native::tools::file_access::PermissionTarget::Local {
                for rule in &rules {
                    let current = crate::native::tools::paths::resolve_local_path(
                        std::path::Path::new("/"),
                        &rule.pattern,
                    )?;
                    if current.to_string_lossy() != rule.pattern {
                        return Err("授权目标已经改变，请重新确认".to_string());
                    }
                }
            }
            rules
        } else {
            let suggestion = pending
                .request
                .suggested_rule
                .as_ref()
                .ok_or_else(|| "该请求不支持始终允许".to_string())?;
            vec![PermissionRule {
                id: String::new(),
                capability: suggestion.capability,
                pattern: suggestion.pattern.clone(),
                source: suggestion.source,
                scope,
                note: "由权限确认对话框保存".to_string(),
                external_path: None,
            }]
        };
        crate::native::permission_rules::add_rules(config_dir, root, RuleEffect::Allow, rules)?;
        self.refresh_permission_rules(config_dir);
        Ok(())
    }

    pub fn begin_finish(
        &mut self,
        session_record_id: &str,
    ) -> Result<Option<mpsc::Sender<NativeFollowup>>, String> {
        let Some(session) = self.sessions.get_mut(session_record_id) else {
            return Ok(None);
        };
        if !session.closing
            && (session.input_queue.is_busy(&session.working)
                || session.followup_tx.capacity() < session.followup_tx.max_capacity())
        {
            return Err("Agent 正在工作，请先停止当前回合".to_string());
        }
        session.closing = true;
        Ok(Some(session.followup_tx.clone()))
    }

    pub fn deny_pending_permission(&mut self, session_record_id: &str) {
        if let Some(session) = self.sessions.get_mut(session_record_id) {
            while let Some(pending) = session.pending_permission.pop_front() {
                let _ = pending.reply.send(NativePermissionDecision::Deny);
            }
            session.pending_question.clear();
            while let Some(pending) = session.pending_plan_approval.pop_front() {
                let _ = pending.reply.send(PlanApprovalAnswer {
                    approved: false,
                    feedback: "会话已停止".to_string(),
                });
            }
        }
    }

    pub fn enqueue_plan_approval(
        &mut self,
        session_record_id: &str,
        pending: PendingPlanApproval,
    ) -> Result<bool, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        if pending.reply.is_closed() || session.cancel.is_cancelled() || session.closing {
            return Err("计划审批请求已失效".to_string());
        }
        let should_emit = session.pending_plan_approval.is_empty();
        session.pending_plan_approval.push_back(pending);
        Ok(should_emit)
    }

    pub fn resolve_plan_approval(
        &mut self,
        session_record_id: &str,
        request_id: &str,
        answer: PlanApprovalAnswer,
    ) -> Result<Option<PlanApprovalRequest>, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        if session.cancel.is_cancelled() || session.closing {
            return Err("会话已停止，不能批准计划".to_string());
        }
        let pending = session
            .pending_plan_approval
            .pop_front()
            .ok_or_else(|| "没有待批准的计划".to_string())?;
        if pending.request.request_id != request_id {
            session.pending_plan_approval.push_front(pending);
            return Err("计划批准请求已过期".to_string());
        }
        pending
            .reply
            .send(answer)
            .map_err(|_| "计划批准通道已关闭".to_string())?;
        Ok(session
            .pending_plan_approval
            .front()
            .map(|item| item.request.clone()))
    }

    pub fn expire_plan_approval(
        &mut self,
        session_record_id: &str,
        request_id: &str,
    ) -> Option<PlanApprovalRequest> {
        let session = self.sessions.get_mut(session_record_id)?;
        let was_front = session
            .pending_plan_approval
            .front()
            .is_some_and(|pending| pending.request.request_id == request_id);
        session
            .pending_plan_approval
            .retain(|pending| pending.request.request_id != request_id);
        was_front
            .then(|| {
                session
                    .pending_plan_approval
                    .front()
                    .map(|pending| pending.request.clone())
            })
            .flatten()
    }

    pub fn enqueue_permission(
        &mut self,
        session_record_id: &str,
        pending: PendingPermission,
    ) -> Result<bool, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let should_emit = session.pending_permission.is_empty();
        session.pending_permission.push_back(pending);
        Ok(should_emit)
    }

    pub fn resolve_permission(
        &mut self,
        session_record_id: &str,
        request_id: &str,
        decision: NativePermissionDecision,
    ) -> Result<Option<PermissionRequest>, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let pending = session
            .pending_permission
            .pop_front()
            .ok_or_else(|| "没有待确认的高风险操作".to_string())?;
        if pending.request.request_id != request_id {
            session.pending_permission.push_front(pending);
            return Err("权限确认请求已过期".to_string());
        }
        if pending.request.allow_once_only
            && !matches!(
                decision,
                NativePermissionDecision::AllowOnce | NativePermissionDecision::Deny
            )
        {
            session.pending_permission.push_front(pending);
            return Err("计划模式下的 Bash 命令只能本次允许或拒绝".to_string());
        }
        if pending.request.file_access.is_some()
            && matches!(
                decision,
                NativePermissionDecision::AllowSession | NativePermissionDecision::AllowServer
            )
        {
            session.pending_permission.push_front(pending);
            return Err("文件访问请使用本次允许或始终允许".to_string());
        }
        if decision == NativePermissionDecision::AllowSession
            && pending.request.kind != NativeToolRiskKind::Mcp
            && pending.request.tool_name != "WorkspaceHooks"
        {
            session
                .allow_all_high_risk
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if session.allow_all_high_risk.load(Ordering::SeqCst) {
            if let Some(runtime) = &mut session.runtime {
                runtime.permission_mode = crate::native::settings::PERMISSION_MODE_YOLO.to_string();
            }
        }
        pending
            .reply
            .send(decision)
            .map_err(|_| "权限确认通道已关闭".to_string())?;
        Ok(session
            .pending_permission
            .front()
            .map(|item| item.request.clone()))
    }

    pub fn expire_permission(
        &mut self,
        session_record_id: &str,
        request_id: &str,
    ) -> Result<Option<PermissionRequest>, String> {
        self.resolve_permission(
            session_record_id,
            request_id,
            NativePermissionDecision::Deny,
        )
    }

    pub fn enqueue_question(
        &mut self,
        session_record_id: &str,
        pending: PendingPlanQuestion,
    ) -> Result<bool, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let should_emit = session.pending_question.is_empty();
        session.pending_question.push_back(pending);
        Ok(should_emit)
    }

    pub fn resolve_question(
        &mut self,
        session_record_id: &str,
        request_id: &str,
        answer: PlanQuestionAnswer,
    ) -> Result<Option<PlanQuestionRequest>, String> {
        let session = self
            .sessions
            .get_mut(session_record_id)
            .ok_or_else(|| "没有运行中的内置 Agent 会话".to_string())?;
        let pending = session
            .pending_question
            .pop_front()
            .ok_or_else(|| "没有待回答的计划提问".to_string())?;
        if pending.request.request_id != request_id {
            session.pending_question.push_front(pending);
            return Err("计划提问已过期".to_string());
        }
        pending
            .reply
            .send(answer)
            .map_err(|_| "计划提问通道已关闭".to_string())?;
        Ok(session
            .pending_question
            .front()
            .map(|item| item.request.clone()))
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn has_profile_processes(&self, profile_id: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.info.profile_id == profile_id)
    }

    pub fn get_profile_processes(&self, profile_id: &str) -> Vec<NativeSessionInfo> {
        self.sessions
            .values()
            .filter(|session| session.info.profile_id == profile_id)
            .map(|session| session.info.clone())
            .collect()
    }

    pub fn has_channel_processes(&self, channel_id: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.info.channel_id == channel_id)
    }

    pub fn get_workspace_processes(&self, workspace_id: &str) -> Vec<NativeSessionInfo> {
        self.sessions
            .values()
            .filter(|session| session.info.workspace_id.as_deref() == Some(workspace_id))
            .map(|session| session.info.clone())
            .collect()
    }

    pub fn has_workspace_processes(&self, workspace_id: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.info.workspace_id.as_deref() == Some(workspace_id))
    }

    pub fn has_working_workspace_processes(&self, workspace_id: &str) -> bool {
        self.sessions.values().any(|session| {
            session.info.workspace_id.as_deref() == Some(workspace_id)
                && session.working.load(Ordering::SeqCst)
        })
    }

    pub fn cancel_all(&mut self) {
        for session in self.sessions.values() {
            session.cancel.cancel();
        }
    }

    pub fn take_all(&mut self) -> Vec<NativeLiveSession> {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        for id in &ids {
            self.deny_pending_permission(id);
        }
        ids.into_iter()
            .filter_map(|id| self.remove_session(&id))
            .collect()
    }
}

pub async fn shutdown_all_sessions(manager: &tokio::sync::Mutex<NativeAgentManager>) {
    let sessions = manager.lock().await.take_all();
    for mut session in sessions {
        session.input_queue.close();
        if session.working.load(Ordering::SeqCst) {
            session.cancel.cancel();
        }
        let _ = session.followup_tx.try_send(NativeFollowup::Finish);
        if tokio::time::timeout(std::time::Duration::from_secs(30), &mut session.join)
            .await
            .is_err()
        {
            session.cancel.cancel();
            session.join.abort();
            let _ = session.join.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_snapshot_reads_the_runner_plan_state() {
        let mut session = live_session("plan-state");
        session.runtime = Some(crate::db::models::NativeSessionRuntime {
            ai_channel_id: "ch".into(),
            model: "model".into(),
            reasoning_effort: None,
            permission_mode: "yolo".into(),
            plan_mode: false,
        });
        let root = tempfile::tempdir().unwrap();
        let mut ctx = crate::native::tools::dispatch::ToolCtx::new(
            crate::native::tools::local::LocalWorkspace::new(root.path().into()),
        );
        ctx.plan_mode = session.plan_mode.clone();
        ctx.set_plan_mode(true);
        assert!(session.runtime_snapshot().unwrap().plan_mode);
        assert!(ctx.is_read_only());
        ctx.set_plan_mode(false);
        assert!(!session.runtime_snapshot().unwrap().plan_mode);
        session.join.await.unwrap();
    }

    #[tokio::test]
    async fn stale_cancelled_and_expired_plan_requests_cannot_be_approved() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("plan"));
        let (reply, rx) = oneshot::channel();
        manager
            .enqueue_plan_approval(
                "plan",
                PendingPlanApproval {
                    request: PlanApprovalRequest {
                        request_id: "current".into(),
                        profile_id: String::new(),
                        workspace_id: None,
                        session_kind: "plan".into(),
                        plan: "plan".into(),
                    },
                    reply,
                },
            )
            .unwrap();
        let approved = || PlanApprovalAnswer {
            approved: true,
            feedback: String::new(),
        };
        assert!(manager
            .resolve_plan_approval("plan", "old", approved())
            .is_err());
        assert_eq!(
            manager
                .get_session("plan")
                .unwrap()
                .pending_plan_approval
                .len(),
            1
        );
        manager.expire_plan_approval("plan", "current");
        assert!(rx.await.is_err());
        assert!(manager
            .resolve_plan_approval("plan", "current", approved())
            .is_err());
        manager.get_session("plan").unwrap().cancel.cancel();
        let (reply, _rx) = oneshot::channel();
        assert!(manager
            .enqueue_plan_approval(
                "plan",
                PendingPlanApproval {
                    request: PlanApprovalRequest {
                        request_id: "late".into(),
                        profile_id: String::new(),
                        workspace_id: None,
                        session_kind: "plan".into(),
                        plan: "plan".into()
                    },
                    reply,
                }
            )
            .is_err());
        assert!(manager
            .resolve_plan_approval("plan", "late", approved())
            .is_err());
    }

    #[tokio::test]
    async fn plan_bash_request_cannot_save_rules_or_enable_session_access() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("plan"));
        let (mut request, mut reply) = pending("bash", "Bash");
        request.request.allow_once_only = true;
        manager.enqueue_permission("plan", request).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(manager
            .save_permission_rules(dir.path(), "plan", "bash", None, None)
            .is_err());
        for decision in [
            NativePermissionDecision::AllowAlways,
            NativePermissionDecision::AllowSession,
            NativePermissionDecision::AllowServer,
        ] {
            assert!(manager
                .resolve_permission("plan", "bash", decision)
                .is_err());
            assert!(reply.try_recv().is_err());
            assert!(!manager
                .get_session("plan")
                .unwrap()
                .allow_all_high_risk
                .load(Ordering::SeqCst));
        }
        manager
            .resolve_permission("plan", "bash", NativePermissionDecision::AllowOnce)
            .unwrap();
        assert_eq!(reply.await.unwrap(), NativePermissionDecision::AllowOnce);
    }

    #[tokio::test]
    async fn tracks_profile_and_workspace_sessions() {
        let mut manager = NativeAgentManager::new();
        let (tx, _rx) = mpsc::channel(1);
        manager.add_session(NativeLiveSession {
            info: NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch-1".to_string(),
                workspace_id: Some("ws-1".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: "sess-1".to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel: CancelFlag::new(),
            followup_tx: tx,
            input_queue: Arc::new(crate::native::input_queue::NativeInputQueue::new("s1")),
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(true)),
            pending_compactions: Arc::default(),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_plan_approval: VecDeque::new(),
        });
        assert!(manager.has_channel_processes("ch-1"));
        assert!(manager.has_workspace_processes("ws-1"));
        assert!(manager.has_working_workspace_processes("ws-1"));
        assert!(!manager.has_workspace_processes("ws-other"));
        assert_eq!(manager.len(), 1);
        manager.add_session(live_session("sess-2"));
        assert_eq!(manager.get_workspace_processes("ws-1").len(), 2);
        assert_eq!(manager.len(), 2);
        manager.cancel_all();
        manager.remove_session("sess-1");
        manager.remove_session("sess-2");
        assert_eq!(manager.len(), 0);
    }

    fn live_session(id: &str) -> NativeLiveSession {
        let (tx, _rx) = mpsc::channel(1);
        NativeLiveSession {
            info: NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch-1".to_string(),
                workspace_id: Some("ws-1".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: id.to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel: CancelFlag::new(),
            followup_tx: tx,
            input_queue: Arc::new(crate::native::input_queue::NativeInputQueue::new(id)),
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            pending_compactions: Arc::default(),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_plan_approval: VecDeque::new(),
        }
    }

    #[tokio::test]
    async fn compaction_stays_busy_after_dequeue_until_the_request_finishes() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id) VALUES ('compact')")
            .execute(&pool)
            .await
            .unwrap();
        let mut manager = NativeAgentManager::new();
        let mut session = live_session("compact");
        let (tx, mut rx) = mpsc::channel(1);
        session.followup_tx = tx;
        session
            .followup_tx
            .try_send(NativeFollowup::Compact(NativeCompactionRequest::new(
                None,
                session.pending_compactions.clone(),
            )))
            .unwrap();
        manager.add_session(session);
        let request = rx.recv().await.unwrap();
        assert!(manager.session_is_busy("compact"));
        let manager = tokio::sync::Mutex::new(manager);
        assert!(crate::app::sessions::set_agent_session_archived_with(
            &pool, &manager, "compact", true,
        )
        .await
        .is_err());
        drop(request);
        assert!(!manager.lock().await.session_is_busy("compact"));
        crate::app::sessions::set_agent_session_archived_with(&pool, &manager, "compact", true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn archive_busy_check_covers_runtime_queue_requests_and_background() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("archive"));
        assert!(!manager.session_is_busy("archive"));
        let session = manager.get_session("archive").unwrap();
        session.working.store(true, Ordering::SeqCst);
        assert!(manager.session_is_busy("archive"));
        manager
            .get_session("archive")
            .unwrap()
            .working
            .store(false, Ordering::SeqCst);
        let snapshot = manager
            .get_session("archive")
            .unwrap()
            .input_queue
            .enqueue("queued", vec![])
            .unwrap();
        assert!(manager.session_is_busy("archive"));
        manager
            .get_session("archive")
            .unwrap()
            .input_queue
            .remove(&snapshot.items[0].id)
            .unwrap();
        assert!(!manager.session_is_busy("archive"));
        let (permission, _reply) = pending("permission", "Write");
        manager
            .get_session_mut("archive")
            .unwrap()
            .pending_permission
            .push_back(permission);
        assert!(manager.session_is_busy("archive"));
        manager
            .get_session_mut("archive")
            .unwrap()
            .pending_permission
            .clear();
        let registry = Arc::new(crate::native::agent::background::BackgroundTaskRegistry::new());
        let (task, _receiver) = registry.register("background", "agent");
        manager.get_session_mut("archive").unwrap().background = Some(registry.clone());
        assert!(manager.session_is_busy("archive"));
        registry.finish(&task.id, Ok("done".to_string()));
        assert!(!manager.session_is_busy("archive"));
        manager.get_session_mut("archive").unwrap().closing = true;
        assert!(manager.session_is_busy("archive"));
    }

    #[tokio::test]
    async fn busy_session_archive_is_rejected_without_changing_or_stopping_it() {
        let pool = crate::db::test_support::setup_migrated_pool().await;
        sqlx::query("INSERT INTO agent_sessions (id, title) VALUES ('busy', 'working')")
            .execute(&pool)
            .await
            .unwrap();
        let mut manager = NativeAgentManager::new();
        let session = live_session("busy");
        session.working.store(true, Ordering::SeqCst);
        manager.add_session(session);
        let manager = tokio::sync::Mutex::new(manager);
        assert!(crate::app::sessions::set_agent_session_archived_with(
            &pool, &manager, "busy", true
        )
        .await
        .is_err());
        let archived: i32 =
            sqlx::query_scalar("SELECT archived FROM agent_sessions WHERE id = 'busy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, 0);
        assert!(manager.lock().await.session_is_busy("busy"));
        assert!(!manager
            .lock()
            .await
            .get_session("busy")
            .unwrap()
            .cancel
            .is_cancelled());
    }

    fn pending(
        request_id: &str,
        tool: &str,
    ) -> (
        PendingPermission,
        oneshot::Receiver<NativePermissionDecision>,
    ) {
        let (reply, rx) = oneshot::channel();
        (
            PendingPermission {
                request: PermissionRequest {
                    request_id: request_id.to_string(),
                    profile_id: "prof-1".to_string(),
                    workspace_id: Some("ws-1".to_string()),
                    session_kind: "execution".to_string(),
                    tool_name: tool.to_string(),
                    kind: NativeToolRiskKind::Overwrite,
                    summary: format!("覆盖 {tool}"),
                    remote: false,
                    mcp_server_id: None,
                    suggested_rule: None,
                    file_access: None,
                    allow_once_only: false,
                },
                reply,
            },
            rx,
        )
    }

    #[tokio::test]
    async fn permission_queue_does_not_deny_previous() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("sess-1"));
        let (first, mut first_rx) = pending("r1", "Write");
        let (second, mut second_rx) = pending("r2", "Bash");
        assert!(manager.enqueue_permission("sess-1", first).expect("first"));
        assert!(!manager
            .enqueue_permission("sess-1", second)
            .expect("second"));
        assert!(first_rx.try_recv().is_err());
        assert!(second_rx.try_recv().is_err());
        let next = manager
            .resolve_permission("sess-1", "r1", NativePermissionDecision::AllowOnce)
            .expect("resolve first");
        assert_eq!(
            next.as_ref().map(|item| item.request_id.as_str()),
            Some("r2")
        );
        assert_eq!(
            first_rx.try_recv().expect("first decision"),
            NativePermissionDecision::AllowOnce
        );
        assert!(second_rx.try_recv().is_err());
        let next = manager
            .resolve_permission("sess-1", "r2", NativePermissionDecision::Deny)
            .expect("resolve second");
        assert!(next.is_none());
        assert_eq!(
            second_rx.try_recv().expect("second decision"),
            NativePermissionDecision::Deny
        );
    }

    #[tokio::test]
    async fn always_allow_saves_before_resolution_and_refreshes_other_sessions() {
        use crate::native::tools::contract::PermissionCapability;
        use crate::native::tools::file_access::{
            FileAccessPath, FileAccessPrompt, PathAccessScope, PermissionTarget,
        };
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(outside.path())
            .unwrap()
            .join("file.txt")
            .to_string_lossy()
            .into_owned();
        let mut manager = NativeAgentManager::new();
        for id in ["one", "two"] {
            let mut session = live_session(id);
            session.workspace_root = Some(workspace.path().to_path_buf());
            manager.add_session(session);
        }
        let (mut pending, mut reply) = pending("request", "Read");
        pending.request.file_access = Some(FileAccessPrompt {
            target: PermissionTarget::Local,
            paths: vec![FileAccessPath {
                path: path.clone(),
                requested_path: path.clone(),
                capability: PermissionCapability::Read,
                scope: PathAccessScope::Exact,
                operation: "read".into(),
                outside_workspace: true,
            }],
        });
        manager.enqueue_permission("one", pending).unwrap();
        let selections = vec![FileAccessSelection {
            path,
            directory: false,
        }];
        assert!(manager
            .save_permission_rules(config.path(), "one", "stale", Some(&selections), None)
            .is_err());
        assert!(manager
            .save_permission_rules(config.path(), "one", "request", Some(&[]), None)
            .is_err());
        let blocked = workspace.path().join(".noxcode");
        std::fs::write(&blocked, "not a directory").unwrap();
        assert!(manager
            .save_permission_rules(config.path(), "one", "request", Some(&selections), None)
            .is_err());
        assert!(matches!(
            reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(
            manager.get_session("one").unwrap().pending_permission.len(),
            1
        );
        assert!(manager
            .get_session("two")
            .unwrap()
            .permission_rules
            .read()
            .unwrap()
            .is_empty());
        std::fs::remove_file(&blocked).unwrap();
        manager
            .save_permission_rules(config.path(), "one", "request", Some(&selections), None)
            .unwrap();
        assert_eq!(
            manager
                .get_session("two")
                .unwrap()
                .permission_rules
                .read()
                .unwrap()
                .allow
                .len(),
            1
        );
        manager
            .resolve_permission("one", "request", NativePermissionDecision::AllowAlways)
            .unwrap();
        assert_eq!(reply.await.unwrap(), NativePermissionDecision::AllowAlways);
        assert!(!manager
            .get_session("one")
            .unwrap()
            .allow_all_high_risk
            .load(Ordering::SeqCst));
        let rules = crate::native::permission_rules::load_effective_rules(
            config.path(),
            Some(workspace.path()),
        );
        crate::native::permission_rules::delete_rule(
            config.path(),
            Some(workspace.path()),
            &rules.allow[0].id,
        )
        .unwrap();
        manager.refresh_permission_rules(config.path());
        assert!(manager
            .get_session("two")
            .unwrap()
            .permission_rules
            .read()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn closed_permission_request_cannot_save_an_allow_rule() {
        let config = tempfile::tempdir().unwrap();
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("one"));
        let (mut pending, reply) = pending("closed", "Bash");
        pending.request.suggested_rule = Some(PermissionRuleSuggestion {
            capability: crate::native::tools::contract::PermissionCapability::Bash,
            pattern: "echo*".into(),
            source: crate::native::tools::contract::PatternSource::Command,
        });
        drop(reply);
        manager.enqueue_permission("one", pending).unwrap();
        assert!(manager
            .save_permission_rules(config.path(), "one", "closed", None, None)
            .is_err());
        assert!(!crate::native::permission_rules::global_rules_path(config.path()).exists());
    }

    #[tokio::test]
    async fn expire_permission_keeps_fifo_order() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("sess-1"));
        let (first, mut first_rx) = pending("r1", "Write");
        let (second, mut second_rx) = pending("r2", "Bash");
        assert!(manager.enqueue_permission("sess-1", first).expect("first"));
        assert!(!manager
            .enqueue_permission("sess-1", second)
            .expect("second"));
        let next = manager
            .expire_permission("sess-1", "r1")
            .expect("expire first");
        assert_eq!(
            next.as_ref().map(|item| item.request_id.as_str()),
            Some("r2")
        );
        assert_eq!(
            first_rx.try_recv().expect("first decision"),
            NativePermissionDecision::Deny
        );
        assert!(second_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn workspace_hook_trust_never_grants_other_tool_permissions() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("sess-1"));
        let (request, rx) = pending("hooks", "WorkspaceHooks");
        manager.enqueue_permission("sess-1", request).unwrap();
        manager
            .resolve_permission("sess-1", "hooks", NativePermissionDecision::AllowSession)
            .unwrap();
        assert_eq!(rx.await.unwrap(), NativePermissionDecision::AllowSession);
        assert!(!manager
            .get_session("sess-1")
            .unwrap()
            .allow_all_high_risk
            .load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn graceful_finish_rejects_working_sessions_without_cancelling() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("sess-1"));
        let session = manager.get_session("sess-1").unwrap();
        session.working.store(true, Ordering::SeqCst);
        assert!(manager.begin_finish("sess-1").is_err());
        let session = manager.get_session("sess-1").unwrap();
        assert!(!session.closing);
        assert!(!session.cancel.is_cancelled());
        session.working.store(false, Ordering::SeqCst);
        assert!(manager.begin_finish("sess-1").unwrap().is_some());
        let session = manager.get_session("sess-1").unwrap();
        assert!(session.closing);
        assert!(!session.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn graceful_finish_rejects_a_pending_or_edited_input() {
        let mut manager = NativeAgentManager::new();
        let mut session = live_session("sess-1");
        let (tx, _rx) = mpsc::channel(8);
        session.followup_tx = tx;
        let queue = session.input_queue.clone();
        manager.add_session(session);
        let snapshot = queue.enqueue("pending", vec![]).unwrap();
        assert!(manager.begin_finish("sess-1").is_err());
        queue.update(&snapshot.items[0].id, None, true).unwrap();
        assert!(manager.begin_finish("sess-1").is_err());
        queue.remove(&snapshot.items[0].id).unwrap();
        assert!(manager.begin_finish("sess-1").unwrap().is_some());
    }

    #[tokio::test]
    async fn shutdown_idle_sessions_preserves_normal_completion() {
        let mut manager = NativeAgentManager::new();
        let mut session = live_session("sess-1");
        let cancel = session.cancel.clone();
        let cancel_run = cancel.clone();
        let (tx, mut rx) = mpsc::channel(1);
        session.followup_tx = tx;
        session.join = tokio::spawn(async move {
            assert!(matches!(rx.recv().await, Some(NativeFollowup::Finish)));
            assert!(!cancel_run.is_cancelled());
        });
        manager.add_session(session);
        shutdown_all_sessions(&tokio::sync::Mutex::new(manager)).await;
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_all_sessions_sends_finish_and_awaits_join() {
        let mut manager = NativeAgentManager::new();
        let (tx, mut rx) = mpsc::channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let join = tokio::spawn(async move {
            match rx.recv().await {
                Some(NativeFollowup::Finish) => {}
                Some(NativeFollowup::Input { .. }) | Some(NativeFollowup::Compact(_)) => {
                    panic!("unexpected input")
                }
                None => panic!("channel closed"),
            }
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        manager.add_session(NativeLiveSession {
            info: NativeSessionInfo {
                profile_id: String::new(),
                channel_id: "ch-1".to_string(),
                workspace_id: Some("ws-1".to_string()),
                session_kind: "execution".to_string(),
                session_record_id: "sess-shutdown".to_string(),
            },
            runtime: None,
            plan_mode: std::sync::Arc::default(),
            background: None,
            closing: false,
            cancel: CancelFlag::new(),
            followup_tx: tx,
            input_queue: Arc::new(crate::native::input_queue::NativeInputQueue::new("s1")),
            join,
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(true)),
            pending_compactions: Arc::default(),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
            permission_rules: crate::native::permission_rules::shared_rules(Default::default()),
            workspace_root: None,
            pending_plan_approval: VecDeque::new(),
        });
        let (pending, pending_rx) = pending("r1", "Write");
        let _ = manager.enqueue_permission("sess-shutdown", pending);
        let manager = tokio::sync::Mutex::new(manager);
        shutdown_all_sessions(&manager).await;
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(manager.lock().await.len(), 0);
        assert_eq!(
            pending_rx.await.expect("denied"),
            NativePermissionDecision::Deny
        );
    }

    #[tokio::test]
    async fn deny_pending_permission_drains_queue() {
        let mut manager = NativeAgentManager::new();
        manager.add_session(live_session("sess-1"));
        let (first, first_rx) = pending("r1", "Write");
        let (second, second_rx) = pending("r2", "Bash");
        let _ = manager.enqueue_permission("sess-1", first);
        let _ = manager.enqueue_permission("sess-1", second);
        manager.deny_pending_permission("sess-1");
        assert_eq!(
            first_rx.await.expect("first"),
            NativePermissionDecision::Deny
        );
        assert_eq!(
            second_rx.await.expect("second"),
            NativePermissionDecision::Deny
        );
    }
}
