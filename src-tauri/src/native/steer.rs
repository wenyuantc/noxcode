//! Current-turn input. Acceptance and the final turn seal share the same mutex.
//! Only durable receipts enter the mailbox; a reopened runtime never replays them.
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::native::model::types::NativeImage;

pub const MAX_TEXT_BYTES: usize = 200 * 1024;
const MAX_PENDING: usize = 8;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TERMINAL_RECEIPTS: usize = 128;
pub const SUPERSEDED: &str = "已由当前回合的新指令替代，未开始的操作已跳过";

#[derive(Debug, Serialize)]
pub struct SteerSubmissionError {
    pub kind: &'static str,
    pub message: String,
}

impl SteerSubmissionError {
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            kind: "rejected",
            message: message.into(),
        }
    }
}
impl From<String> for SteerSubmissionError {
    fn from(message: String) -> Self {
        Self {
            kind: "unavailable",
            message,
        }
    }
}
impl From<&str> for SteerSubmissionError {
    fn from(message: &str) -> Self {
        Self::rejected(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainOrigin {
    pub instance_id: String,
    pub generation: u64,
    pub child: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SteerStatus {
    Accepted,
    Applied,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SteerReceipt {
    pub session_record_id: String,
    pub instance_id: String,
    pub turn_id: String,
    pub input_id: String,
    pub text: String,
    pub image_count: usize,
    #[serde(default)]
    pub payload_hash: String,
    pub generation: u64,
    pub status: SteerStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerSnapshot {
    pub session_record_id: String,
    pub instance_id: String,
    pub turn_id: Option<String>,
    pub revision: u64,
    pub lifecycle: Option<crate::db::models::NativeTurnState>,
    pub receipts: Vec<SteerReceipt>,
}

pub struct SteerInput {
    pub receipt: SteerReceipt,
    pub images: Vec<NativeImage>,
}

struct Entry {
    receipt: SteerReceipt,
    paths: Vec<String>,
    image_bytes: usize,
}

#[derive(Default)]
struct State {
    turn_id: Option<String>,
    turn_generation: u64,
    lifecycle: Option<crate::db::models::NativeTurnState>,
    entries: VecDeque<Entry>,
    pending: VecDeque<SteerInput>,
    revision: u64,
}

type Persist = Arc<
    dyn Fn(SteerReceipt) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync,
>;
type Publish = Arc<dyn Fn(SteerSnapshot) + Send + Sync>;

pub struct SteerMailbox {
    pub instance_id: String,
    session_id: String,
    generation: AtomicU64,
    state: Mutex<State>,
    persist: RwLock<Option<Persist>>,
    publish: RwLock<Option<Publish>>,
}

impl SteerMailbox {
    pub fn new(session_id: &str, instance_id: &str) -> Self {
        Self {
            instance_id: instance_id.into(),
            session_id: session_id.into(),
            generation: AtomicU64::new(0),
            state: Mutex::new(State::default()),
            persist: RwLock::new(None),
            publish: RwLock::new(None),
        }
    }

    pub fn configure(&self, persist: Persist, publish: Publish) {
        *self.persist.write().expect("steer persistence") = Some(persist);
        *self.publish.write().expect("steer publisher") = Some(publish);
    }

    async fn save(&self, receipt: &SteerReceipt) -> Result<(), String> {
        let persist = self.persist.read().expect("steer persistence").clone();
        match persist {
            Some(persist) => persist(receipt.clone()).await,
            None => Err("转向输入持久化尚未就绪".into()),
        }
    }

    fn snapshot_with(&self, state: &State) -> SteerSnapshot {
        SteerSnapshot {
            session_record_id: self.session_id.clone(),
            instance_id: self.instance_id.clone(),
            turn_id: state.turn_id.clone(),
            revision: state.revision,
            lifecycle: state.lifecycle.clone(),
            receipts: state
                .entries
                .iter()
                .map(|entry| entry.receipt.clone())
                .collect(),
        }
    }

    fn changed(&self, state: &mut State) {
        state.revision += 1;
        if let Some(publish) = self.publish.read().expect("steer publisher").as_ref() {
            publish(self.snapshot_with(state));
        }
    }

    pub async fn snapshot(&self) -> SteerSnapshot {
        self.snapshot_with(&*self.state.lock().await)
    }

    pub fn origin(&self) -> MainOrigin {
        MainOrigin {
            instance_id: self.instance_id.clone(),
            generation: self.generation.load(Ordering::SeqCst),
            child: false,
        }
    }

    pub fn is_current(&self, origin: &MainOrigin) -> bool {
        origin.instance_id == self.instance_id
            && (origin.child || origin.generation == self.generation.load(Ordering::SeqCst))
    }

    pub async fn begin_turn(&self) -> MainOrigin {
        let mut state = self.state.lock().await;
        if state.turn_id.is_some() {
            return MainOrigin {
                instance_id: self.instance_id.clone(),
                generation: state.turn_generation,
                child: false,
            };
        }
        debug_assert!(state.pending.is_empty());
        let turn_id = uuid::Uuid::new_v4().to_string();
        state.turn_id = Some(turn_id.clone());
        state.lifecycle = Some(crate::db::models::NativeTurnState {
            steer_turn_id: Some(turn_id.clone()),
            session_record_id: self.session_id.clone(),
            instance_id: self.instance_id.clone(),
            turn_id,
            revision: state.revision + 1,
            state: "working".into(),
        });
        state.turn_generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.changed(&mut state);
        self.origin()
    }

    /// Retain the completed turn identity through waiting/idle compaction. Every
    /// lifecycle transition advances the same runtime revision used by snapshots.
    pub async fn lifecycle_state(&self, value: &str) -> Option<crate::db::models::NativeTurnState> {
        let mut state = self.state.lock().await;
        let mut lifecycle = state.lifecycle.clone()?;
        state.revision += 1;
        lifecycle.revision = state.revision;
        lifecycle.steer_turn_id = state.turn_id.clone();
        lifecycle.state = value.into();
        state.lifecycle = Some(lifecycle.clone());
        Some(lifecycle)
    }

    pub fn payload_hash(text: &str, paths: &[String]) -> String {
        use sha2::{Digest, Sha256};
        let payload = serde_json::to_vec(&(text, paths)).expect("serializable steer payload");
        format!("{:x}", Sha256::digest(payload))
    }

    pub fn validate_payload(input_id: &str, text: &str, paths: &[String]) -> Result<(), String> {
        uuid::Uuid::parse_str(input_id).map_err(|_| "输入标识必须是 UUID".to_string())?;
        if text.len() > MAX_TEXT_BYTES {
            return Err("转向文字不能超过 200 KiB".into());
        }
        if paths.len() > 8 {
            return Err("每条转向最多附带 8 张图片".into());
        }
        if text.trim().is_empty() && paths.is_empty() {
            return Err("输入内容不能为空".into());
        }
        Ok(())
    }

    fn duplicate(
        state: &State,
        turn_id: &str,
        input_id: &str,
        text: &str,
        paths: &[String],
    ) -> Result<Option<SteerReceipt>, String> {
        if let Some(entry) = state
            .entries
            .iter()
            .find(|entry| entry.receipt.input_id == input_id)
        {
            if entry.receipt.turn_id != turn_id
                || entry.receipt.text != text
                || entry.paths != paths
            {
                return Err("相同输入标识不能用于不同内容".into());
            }
            return Ok(Some(entry.receipt.clone()));
        }
        Ok(None)
    }

    pub async fn prior(
        &self,
        turn_id: &str,
        input_id: &str,
        text: &str,
        paths: &[String],
    ) -> Result<Option<SteerReceipt>, String> {
        Self::validate_payload(input_id, text, paths)?;
        Self::duplicate(&*self.state.lock().await, turn_id, input_id, text, paths)
    }

    fn image_bytes(images: &[NativeImage]) -> usize {
        images
            .iter()
            .map(|image| {
                image.data_base64.len() / 4 * 3
                    - image
                        .data_base64
                        .bytes()
                        .rev()
                        .take_while(|b| *b == b'=')
                        .count()
            })
            .sum()
    }

    fn validate_admission(state: &State, turn_id: &str, image_bytes: usize) -> Result<(), String> {
        if state.turn_id.as_deref() != Some(turn_id) {
            return Err("当前回合已结束或已切换，请保留草稿并重试".into());
        }
        let pending: Vec<_> = state
            .entries
            .iter()
            .filter(|e| e.receipt.status == SteerStatus::Accepted)
            .collect();
        if pending.len() >= MAX_PENDING {
            return Err("当前回合最多待处理 8 条转向指令".into());
        }
        if image_bytes + pending.iter().map(|entry| entry.image_bytes).sum::<usize>()
            > MAX_IMAGE_BYTES
        {
            return Err("待处理转向图片总大小不能超过 64 MiB".into());
        }
        Ok(())
    }

    pub async fn check_admission(
        &self,
        turn_id: &str,
        input_id: &str,
        text: &str,
        paths: &[String],
        images: &[NativeImage],
    ) -> Result<Option<SteerReceipt>, String> {
        Self::validate_payload(input_id, text, paths)?;
        let state = self.state.lock().await;
        if let Some(receipt) = Self::duplicate(&state, turn_id, input_id, text, paths)? {
            return Ok(Some(receipt));
        }
        Self::validate_admission(&state, turn_id, Self::image_bytes(images))?;
        Ok(None)
    }

    // Terminal entries are only a bounded display/cache. The IPC's durable UUID
    // lookup is authoritative, including entries evicted during the same turn.
    fn prune_terminal(state: &mut State) {
        let mut excess = state
            .entries
            .iter()
            .filter(|entry| entry.receipt.status != SteerStatus::Accepted)
            .count()
            .saturating_sub(MAX_TERMINAL_RECEIPTS);
        state.entries.retain(|entry| {
            if excess > 0 && entry.receipt.status != SteerStatus::Accepted {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }

    pub async fn accept(
        &self,
        turn_id: &str,
        input_id: &str,
        text: &str,
        paths: &[String],
        images: Vec<NativeImage>,
    ) -> Result<SteerReceipt, String> {
        Self::validate_payload(input_id, text, paths)?;
        let mut state = self.state.lock().await;
        if let Some(prior) = Self::duplicate(&state, turn_id, input_id, text, paths)? {
            return Ok(prior);
        }
        let image_bytes = Self::image_bytes(&images);
        Self::validate_admission(&state, turn_id, image_bytes)?;
        let receipt = SteerReceipt {
            session_record_id: self.session_id.clone(),
            instance_id: self.instance_id.clone(),
            turn_id: turn_id.into(),
            input_id: input_id.into(),
            text: text.into(),
            image_count: images.len(),
            payload_hash: Self::payload_hash(text, paths),
            generation: self.generation.load(Ordering::SeqCst) + 1,
            status: SteerStatus::Accepted,
            error: None,
        };
        // Holding the gate across durable acceptance makes success vs final seal linearizable.
        self.save(&receipt).await?;
        self.generation.store(receipt.generation, Ordering::SeqCst);
        state.entries.push_back(Entry {
            receipt: receipt.clone(),
            paths: paths.to_vec(),
            image_bytes,
        });
        state.pending.push_back(SteerInput {
            receipt: receipt.clone(),
            images,
        });
        self.changed(&mut state);
        Ok(receipt)
    }

    pub async fn take(&self) -> Option<SteerInput> {
        self.state.lock().await.pending.pop_front()
    }

    pub async fn finish_input(
        &self,
        input_id: &str,
        status: SteerStatus,
        error: Option<String>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.receipt.input_id == input_id)
            .ok_or("转向输入已失效")?;
        if entry.receipt.status != SteerStatus::Accepted {
            return Ok(());
        }
        let mut receipt = entry.receipt.clone();
        receipt.status = status;
        receipt.error = error;
        self.save(&receipt).await?;
        entry.receipt = receipt;
        entry.image_bytes = 0;
        Self::prune_terminal(&mut state);
        self.changed(&mut state);
        Ok(())
    }

    pub async fn seal(&self) -> bool {
        let mut state = self.state.lock().await;
        if state
            .entries
            .iter()
            .any(|entry| entry.receipt.status == SteerStatus::Accepted)
        {
            return false;
        }
        state.turn_id = None;
        self.changed(&mut state);
        true
    }

    pub async fn cancel(&self, reason: &str) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.turn_id = None;
        state.pending.clear();
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut failure = None;
        for entry in &mut state.entries {
            if entry.receipt.status == SteerStatus::Accepted {
                let mut receipt = entry.receipt.clone();
                receipt.status = SteerStatus::Cancelled;
                receipt.error = Some(reason.into());
                if let Err(error) = self.save(&receipt).await {
                    failure = Some(error);
                }
                entry.receipt = receipt;
                entry.image_bytes = 0;
            }
        }
        Self::prune_terminal(&mut state);
        self.changed(&mut state);
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mailbox() -> SteerMailbox {
        let mailbox = SteerMailbox::new("session", "runtime");
        mailbox.configure(Arc::new(|_| Box::pin(async { Ok(()) })), Arc::new(|_| {}));
        mailbox.begin_turn().await;
        mailbox
    }

    #[tokio::test]
    async fn acceptance_seal_dedup_and_generation() {
        let mailbox = mailbox().await;
        let origin = mailbox.origin();
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let receipt = mailbox
            .accept(&turn, &id, "new", &[], vec![])
            .await
            .unwrap();
        assert!(!mailbox.is_current(&origin));
        assert!(!mailbox.seal().await);
        assert_eq!(
            receipt,
            mailbox
                .accept(&turn, &id, "new", &[], vec![])
                .await
                .unwrap()
        );
        assert!(mailbox
            .accept(&turn, &id, "different", &[], vec![])
            .await
            .is_err());
        assert!(mailbox.take().await.is_some());
        assert!(mailbox.take().await.is_none());
        assert!(
            !mailbox.seal().await,
            "an async hook owns the claimed input"
        );
        mailbox
            .finish_input(&id, SteerStatus::Applied, None)
            .await
            .unwrap();
        assert!(mailbox.seal().await);
        assert!(mailbox
            .accept(
                &turn,
                &uuid::Uuid::new_v4().to_string(),
                "late",
                &[],
                vec![]
            )
            .await
            .is_err());
        assert_eq!(
            mailbox
                .prior(&turn, &id, "new", &[])
                .await
                .unwrap()
                .unwrap()
                .status,
            SteerStatus::Applied
        );
    }

    #[tokio::test]
    async fn durable_failure_never_acknowledges_or_delivers() {
        let mailbox = mailbox().await;
        mailbox.configure(
            Arc::new(|_| Box::pin(async { Err("disk full".into()) })),
            Arc::new(|_| {}),
        );
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let origin = mailbox.origin();
        assert!(mailbox
            .accept(&turn, &uuid::Uuid::new_v4().to_string(), "new", &[], vec![])
            .await
            .is_err());
        assert!(mailbox.take().await.is_none());
        assert!(mailbox.is_current(&origin));
        assert!(mailbox.seal().await);
    }

    #[tokio::test]
    async fn pending_limit_utf8_stale_and_cancel() {
        let mailbox = mailbox().await;
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let id = || uuid::Uuid::new_v4().to_string();
        assert!(mailbox
            .accept("old", &id(), "x", &[], vec![])
            .await
            .is_err());
        assert!(mailbox
            .accept(
                &turn,
                &id(),
                &"中".repeat(MAX_TEXT_BYTES / 3 + 1),
                &[],
                vec![]
            )
            .await
            .is_err());
        for _ in 0..8 {
            mailbox
                .accept(&turn, &id(), "x", &[], vec![])
                .await
                .unwrap();
        }
        assert!(mailbox
            .accept(&turn, &id(), "full", &[], vec![])
            .await
            .is_err());
        mailbox.cancel("stopped").await.unwrap();
        assert!(mailbox.take().await.is_none());
        assert!(mailbox
            .snapshot()
            .await
            .receipts
            .iter()
            .all(|r| r.status == SteerStatus::Cancelled));
    }
    #[tokio::test]
    async fn accepted_durability_and_final_seal_share_a_single_gate() {
        let mailbox = Arc::new(mailbox().await);
        let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let release_save = release.clone();
        mailbox.configure(
            Arc::new(move |_| {
                let entered = entered.clone();
                let release = release_save.clone();
                Box::pin(async move {
                    entered.send(()).unwrap();
                    let _permit = release.acquire().await.unwrap();
                    Ok(())
                })
            }),
            Arc::new(|_| {}),
        );
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let accepting = mailbox.clone();
        let accept = tokio::spawn(async move {
            accepting
                .accept(
                    &turn,
                    &uuid::Uuid::new_v4().to_string(),
                    "racing",
                    &[],
                    vec![],
                )
                .await
        });
        entered_rx.recv().await.unwrap();
        let sealing = mailbox.clone();
        let seal = tokio::spawn(async move { sealing.seal().await });
        tokio::task::yield_now().await;
        assert!(!seal.is_finished());
        release.add_permits(1);
        accept.await.unwrap().unwrap();
        assert!(!seal.await.unwrap());
        assert!(mailbox.take().await.is_some());
    }

    #[tokio::test]
    async fn aggregate_decoded_image_limit_includes_claimed_input() {
        let mailbox = mailbox().await;
        let turn = mailbox.snapshot().await.turn_id.unwrap();
        let image = NativeImage {
            name: "large.png".into(),
            mime_type: "image/png".into(),
            data_base64: "AAAA".repeat(MAX_IMAGE_BYTES / 8 / 3),
        };
        let images = vec![image; 8];
        let id = uuid::Uuid::new_v4().to_string();
        mailbox
            .accept(&turn, &id, "first", &["stage.png".into()], images)
            .await
            .unwrap();
        let _claimed = mailbox.take().await.unwrap();
        let over = NativeImage {
            name: "other.png".into(),
            mime_type: "image/png".into(),
            data_base64: "AAAA".repeat(8),
        };
        assert!(mailbox
            .accept(
                &turn,
                &uuid::Uuid::new_v4().to_string(),
                "over",
                &["other.png".into()],
                vec![over.clone()]
            )
            .await
            .is_err());
        mailbox
            .finish_input(&id, SteerStatus::Rejected, Some("blocked".into()))
            .await
            .unwrap();
        mailbox
            .accept(
                &turn,
                &uuid::Uuid::new_v4().to_string(),
                "fits",
                &["other.png".into()],
                vec![over],
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn lifecycle_identity_precedes_working_and_survives_seal_and_idle_compaction() {
        let mailbox = mailbox().await;
        let initial = mailbox.snapshot().await;
        let turn = initial.turn_id.unwrap();
        assert_eq!(initial.lifecycle.as_ref().unwrap().turn_id, turn);
        let working = mailbox.lifecycle_state("working").await.unwrap();
        assert_eq!(working.instance_id, "runtime");
        assert_eq!(working.turn_id, turn);
        assert_eq!(working.steer_turn_id.as_deref(), Some(turn.as_str()));
        assert!(working.revision > initial.revision);
        assert!(mailbox.seal().await);
        let sealed = mailbox.snapshot().await;
        assert!(sealed.turn_id.is_none());
        assert_eq!(sealed.lifecycle.unwrap().turn_id, turn);
        let waiting = mailbox.lifecycle_state("waiting_input").await.unwrap();
        assert_eq!(waiting.turn_id, turn);
        assert!(waiting.steer_turn_id.is_none());
        assert!(waiting.revision > sealed.revision);
        let compacting = mailbox.lifecycle_state("working").await.unwrap();
        let compacted = mailbox.lifecycle_state("waiting_input").await.unwrap();
        assert_eq!(compacting.turn_id, turn);
        assert!(compacting.steer_turn_id.is_none());
        assert_eq!(compacted.turn_id, turn);
        assert!(compacting.revision > waiting.revision && compacted.revision > compacting.revision);
        assert!(
            mailbox.snapshot().await.turn_id.is_none(),
            "idle compaction does not open a user turn"
        );
        mailbox.begin_turn().await;
        let next = mailbox.snapshot().await;
        assert_ne!(next.turn_id.as_deref(), Some(turn.as_str()));
        let lifecycle = next.lifecycle.unwrap();
        assert_eq!(lifecycle.turn_id, next.turn_id.unwrap());
        assert_eq!(lifecycle.state, "working");
        assert_eq!(
            lifecycle.steer_turn_id.as_deref(),
            Some(lifecycle.turn_id.as_str())
        );
        assert!(lifecycle.revision > compacted.revision);
        let wire = serde_json::to_value(&compacted).unwrap();
        assert_eq!(wire["instance_id"], "runtime");
        assert_eq!(wire["turn_id"], turn);
        assert_eq!(wire["revision"].as_u64(), Some(compacted.revision));
    }
}
