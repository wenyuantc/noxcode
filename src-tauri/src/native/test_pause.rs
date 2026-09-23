//! 每个服务实例自己的测试屏障。未武装时立即返回，没有生产 IPC。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

struct Point {
    entered: AtomicBool,
    consumed: AtomicBool,
    entered_notify: Notify,
    release: Notify,
}

#[derive(Clone, Default)]
pub(crate) struct PauseHub {
    points: Arc<Mutex<HashMap<String, Arc<Point>>>>,
}

impl PauseHub {
    pub(crate) fn arm(&self, name: &str) {
        #[cfg(any(test, feature = "media-faults"))]
        self.points.lock().expect("pause").insert(
            name.to_string(),
            Arc::new(Point {
                entered: AtomicBool::new(false),
                consumed: AtomicBool::new(false),
                entered_notify: Notify::new(),
                release: Notify::new(),
            }),
        );
        #[cfg(not(any(test, feature = "media-faults")))]
        let _ = name;
    }

    pub(crate) async fn wait(&self, name: &str) {
        #[cfg(any(test, feature = "media-faults"))]
        {
            let point = self.points.lock().expect("pause").get(name).cloned();
            if let Some(point) = point {
                if point.consumed.swap(true, Ordering::SeqCst) {
                    return;
                }
                let released = point.release.notified();
                point.entered.store(true, Ordering::SeqCst);
                point.entered_notify.notify_one();
                released.await;
            }
        }
        #[cfg(not(any(test, feature = "media-faults")))]
        let _ = name;
    }

    pub(crate) async fn until_entered(&self, name: &str) {
        let point = self
            .points
            .lock()
            .expect("pause")
            .get(name)
            .cloned()
            .expect("pause armed");
        loop {
            let notified = point.entered_notify.notified();
            if point.entered.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn release(&self, name: &str) {
        if let Some(point) = self.points.lock().expect("pause").get(name) {
            point.release.notify_one();
        }
    }
}
