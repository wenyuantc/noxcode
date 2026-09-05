use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::Notify;

use crate::app::shared::new_id;
use crate::native::model::types::NativeImage;
use crate::native::tools::CancelFlag;

const MAX_PENDING_INPUTS: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeQueuedInputView {
    pub id: String,
    pub text: String,
    pub image_count: usize,
    pub editing: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeInputQueueSnapshot {
    pub session_record_id: String,
    pub queue_id: String,
    pub revision: u64,
    pub items: Vec<NativeQueuedInputView>,
}

pub struct NativeQueuedInput {
    pub id: String,
    pub text: String,
    pub images: Vec<NativeImage>,
    editing: bool,
}

#[derive(Default)]
struct QueueState {
    items: VecDeque<NativeQueuedInput>,
    revision: u64,
    closed: bool,
}

type OnChange = Arc<dyn Fn(NativeInputQueueSnapshot) + Send + Sync>;

pub struct NativeInputQueue {
    pub id: String,
    session_record_id: String,
    state: Mutex<QueueState>,
    changed: Notify,
    on_change: Mutex<Option<OnChange>>,
}

impl NativeInputQueue {
    pub fn new(session_record_id: &str) -> Self {
        Self {
            id: new_id(),
            session_record_id: session_record_id.to_string(),
            state: Mutex::new(QueueState::default()),
            changed: Notify::new(),
            on_change: Mutex::new(None),
        }
    }

    pub fn set_on_change(&self, callback: OnChange) {
        *self.on_change.lock().expect("input queue callback") = Some(callback);
    }

    fn snapshot_with(&self, state: &QueueState) -> NativeInputQueueSnapshot {
        NativeInputQueueSnapshot {
            session_record_id: self.session_record_id.clone(),
            queue_id: self.id.clone(),
            revision: state.revision,
            items: state
                .items
                .iter()
                .map(|item| NativeQueuedInputView {
                    id: item.id.clone(),
                    text: item.text.clone(),
                    image_count: item.images.len(),
                    editing: item.editing,
                })
                .collect(),
        }
    }

    pub fn snapshot(&self) -> NativeInputQueueSnapshot {
        self.snapshot_with(&self.state.lock().expect("input queue"))
    }

    fn publish(&self, state: &mut QueueState) -> NativeInputQueueSnapshot {
        state.revision += 1;
        let snapshot = self.snapshot_with(state);
        // Publish under the queue lock so edits, claims and their events stay ordered.
        if let Some(callback) = self
            .on_change
            .lock()
            .expect("input queue callback")
            .as_ref()
        {
            callback(snapshot.clone());
        }
        self.changed.notify_one();
        snapshot
    }

    pub fn enqueue(
        &self,
        text: &str,
        images: Vec<NativeImage>,
    ) -> Result<NativeInputQueueSnapshot, String> {
        if text.trim().is_empty() && images.is_empty() {
            return Err("输入内容不能为空".to_string());
        }
        let mut state = self.state.lock().expect("input queue");
        if state.closed {
            return Err("会话已结束，无法追加指令".to_string());
        }
        if state.items.len() >= MAX_PENDING_INPUTS {
            return Err(format!(
                "待执行指令最多 {MAX_PENDING_INPUTS} 条，请等待或移除后重试"
            ));
        }
        state.items.push_back(NativeQueuedInput {
            id: new_id(),
            text: text.trim().to_string(),
            images,
            editing: false,
        });
        Ok(self.publish(&mut state))
    }

    pub fn update(
        &self,
        id: &str,
        text: Option<&str>,
        editing: bool,
    ) -> Result<NativeInputQueueSnapshot, String> {
        let mut state = self.state.lock().expect("input queue");
        let item = state
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| "指令已开始执行或已移除，无法编辑".to_string())?;
        if let Some(text) = text {
            if text.trim().is_empty() && item.images.is_empty() {
                return Err("输入内容不能为空".to_string());
            }
            item.text = text.trim().to_string();
        }
        item.editing = editing;
        Ok(self.publish(&mut state))
    }

    pub fn remove(&self, id: &str) -> Result<NativeInputQueueSnapshot, String> {
        let mut state = self.state.lock().expect("input queue");
        let index = state
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| "指令已开始执行或已移除".to_string())?;
        state.items.remove(index);
        Ok(self.publish(&mut state))
    }

    pub fn is_empty(&self) -> bool {
        self.state.lock().expect("input queue").items.is_empty()
    }

    pub fn is_busy(&self, working: &AtomicBool) -> bool {
        let state = self.state.lock().expect("input queue");
        !state.items.is_empty() || working.load(Ordering::SeqCst)
    }

    pub fn close(&self) {
        let mut state = self.state.lock().expect("input queue");
        state.closed = true;
        state.items.clear();
        self.publish(&mut state);
    }

    fn take(&self, cancel: &CancelFlag, working: &AtomicBool) -> Option<NativeQueuedInput> {
        let mut state = self.state.lock().expect("input queue");
        if state.closed || cancel.is_cancelled() || state.items.front()?.editing {
            return None;
        }
        let item = state.items.pop_front();
        // Claim and mark working atomically with respect to graceful-finish checks.
        working.store(true, Ordering::SeqCst);
        self.publish(&mut state);
        item
    }

    // Only the session's between-turn loop calls recv; the model's steer loop never sees this queue.
    pub async fn recv(
        &self,
        cancel: &CancelFlag,
        working: &AtomicBool,
    ) -> Option<NativeQueuedInput> {
        loop {
            let notified = self.changed.notified();
            if let Some(item) = self.take(cancel, working) {
                return Some(item);
            }
            if cancel.is_cancelled() || self.state.lock().expect("input queue").closed {
                return None;
            }
            tokio::select! {
                _ = cancel.cancelled() => return None,
                _ = notified => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn preserves_fifo_and_pauses_an_item_while_it_is_being_edited() {
        let queue = NativeInputQueue::new("session");
        let cancel = CancelFlag::new();
        let working = AtomicBool::new(false);
        let first = queue.enqueue("first", vec![]).unwrap().items[0].id.clone();
        let second = queue.enqueue("second", vec![]).unwrap().items[1].id.clone();
        queue.update(&first, None, true).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), queue.recv(&cancel, &working))
                .await
                .is_err()
        );
        let edited = queue.update(&first, Some("edited first"), false).unwrap();
        assert_eq!(edited.items[1].id, second);
        assert_eq!(
            queue.recv(&cancel, &working).await.unwrap().text,
            "edited first"
        );
        assert!(queue.update(&first, Some("too late"), false).is_err());
        assert_eq!(queue.recv(&cancel, &working).await.unwrap().text, "second");
        assert!(queue.is_empty());
        assert!(queue.is_busy(&working));
    }

    #[tokio::test]
    async fn cancellation_and_close_never_start_pending_input() {
        let queue = NativeInputQueue::new("session");
        queue.enqueue("pending", vec![]).unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();
        assert!(queue.recv(&cancel, &AtomicBool::new(false)).await.is_none());
        assert_eq!(queue.snapshot().items.len(), 1);
        queue.close();
        assert!(queue
            .recv(&CancelFlag::new(), &AtomicBool::new(false))
            .await
            .is_none());
        assert!(queue.enqueue("late", vec![]).is_err());
        assert!(queue.is_empty());
    }

    #[test]
    fn validates_capacity_and_publishes_monotonic_snapshots() {
        let queue = NativeInputQueue::new("session");
        let snapshots = Arc::new(Mutex::new(vec![]));
        let observed = snapshots.clone();
        queue.set_on_change(Arc::new(move |snapshot| {
            observed.lock().unwrap().push(snapshot)
        }));
        assert!(queue.enqueue(" ", vec![]).is_err());
        for i in 0..MAX_PENDING_INPUTS {
            queue.enqueue(&i.to_string(), vec![]).unwrap();
        }
        assert!(queue.enqueue("full", vec![]).is_err());
        let id = queue.snapshot().items[0].id.clone();
        assert!(queue.update(&id, Some(" "), false).is_err());
        queue.remove(&id).unwrap();
        queue.enqueue("last", vec![]).unwrap();
        assert_eq!(queue.snapshot().items[0].text, "1");
        for (i, snapshot) in snapshots.lock().unwrap().iter().enumerate() {
            assert_eq!(snapshot.revision, i as u64 + 1);
            assert_eq!(snapshot.session_record_id, "session");
            assert_eq!(snapshot.queue_id, queue.id);
        }
    }
}
