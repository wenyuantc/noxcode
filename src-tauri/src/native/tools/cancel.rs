use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct CancelFlag {
    inner: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_wakes_all_waiters_and_is_not_lost_before_waiting() {
        let flag = CancelFlag::new();
        let first = flag.clone();
        let second = flag.clone();
        let a = tokio::spawn(async move { first.cancelled().await });
        let b = tokio::spawn(async move { second.cancelled().await });
        tokio::task::yield_now().await;
        flag.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            a.await.unwrap();
            b.await.unwrap();
            flag.cancelled().await;
        })
        .await
        .unwrap();
    }
}
