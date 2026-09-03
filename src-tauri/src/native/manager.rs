#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::native::tools::permission::{NativePermissionDecision, NativeToolRiskKind};
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

pub enum NativeFollowup {
    Input(String),
    Finish,
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
}

pub struct PendingPermission {
    pub request: PermissionRequest,
    pub reply: oneshot::Sender<NativePermissionDecision>,
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
    pub cancel: CancelFlag,
    pub followup_tx: mpsc::Sender<NativeFollowup>,
    pub join: JoinHandle<()>,
    pub allow_all_high_risk: Arc<AtomicBool>,
    pub pending_permission: VecDeque<PendingPermission>,
    pub pending_question: VecDeque<PendingPlanQuestion>,
}

#[derive(Default)]
pub struct NativeAgentManager {
    sessions: HashMap<String, NativeLiveSession>,
}

impl NativeAgentManager {
    pub fn new() -> Self {
        Self::default()
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

    pub fn deny_pending_permission(&mut self, session_record_id: &str) {
        if let Some(session) = self.sessions.get_mut(session_record_id) {
            while let Some(pending) = session.pending_permission.pop_front() {
                let _ = pending.reply.send(NativePermissionDecision::Deny);
            }
            session.pending_question.clear();
        }
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
        if decision == NativePermissionDecision::AllowSession
            && pending.request.kind != NativeToolRiskKind::Mcp
        {
            session
                .allow_all_high_risk
                .store(true, std::sync::atomic::Ordering::SeqCst);
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

    pub fn cancel_all(&mut self) {
        for session in self.sessions.values() {
            session.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            cancel: CancelFlag::new(),
            followup_tx: tx,
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
        });
        assert!(manager.has_channel_processes("ch-1"));
        assert!(manager.has_workspace_processes("ws-1"));
        assert!(!manager.has_workspace_processes("ws-other"));
        assert_eq!(manager.len(), 1);
        manager.cancel_all();
        manager.remove_session("sess-1");
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
            cancel: CancelFlag::new(),
            followup_tx: tx,
            join: tokio::spawn(async {}),
            allow_all_high_risk: Arc::new(AtomicBool::new(false)),
            pending_permission: VecDeque::new(),
            pending_question: VecDeque::new(),
        }
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
