//! 后台子 Agent 任务与父子消息。
//!
//! `Agent(run_in_background=true)` 立即返回 `task_id`，子 Agent 在独立任务里运行；父 Agent
//! 用 `TaskOutput` 取结果 / 等待、`TaskStop` 取消、`SendMessage` 追加指令；子 Agent 用
//! `RespondToCoordinator` 给父 Agent 留言。完成的任务与留言会在父 Agent 下一次模型调用前
//! 以提醒形式注入。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, Notify};

use crate::native::manager::NativeFollowup;
use crate::native::tools::CancelFlag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Done(String),
    Failed(String),
    Stopped,
}

impl TaskStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done(_) => "done",
            Self::Failed(_) => "failed",
            Self::Stopped => "stopped",
        }
    }

    pub fn is_finished(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

pub struct BackgroundTask {
    pub id: String,
    pub description: String,
    pub kind: String,
    pub cancel: CancelFlag,
    status: Mutex<TaskStatus>,
    announced: Mutex<bool>,
    notify: Notify,
    steer_tx: mpsc::Sender<NativeFollowup>,
}

impl BackgroundTask {
    pub fn status(&self) -> TaskStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or(TaskStatus::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorMessage {
    pub task_id: String,
    pub description: String,
    pub message: String,
}

#[derive(Default)]
pub struct BackgroundTaskRegistry {
    tasks: Mutex<HashMap<String, Arc<BackgroundTask>>>,
    inbox: Mutex<Vec<CoordinatorMessage>>,
    seq: AtomicU32,
}

impl BackgroundTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个后台任务，返回任务句柄与给子 Agent 的 steer 接收端。
    pub fn register(
        &self,
        description: &str,
        kind: &str,
    ) -> (Arc<BackgroundTask>, mpsc::Receiver<NativeFollowup>) {
        let seq = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        let id = format!("task-{seq}");
        let (steer_tx, steer_rx) = mpsc::channel(16);
        let task = Arc::new(BackgroundTask {
            id: id.clone(),
            description: description.to_string(),
            kind: kind.to_string(),
            cancel: CancelFlag::new(),
            status: Mutex::new(TaskStatus::Running),
            announced: Mutex::new(false),
            notify: Notify::new(),
            steer_tx,
        });
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(id, task.clone());
        }
        (task, steer_rx)
    }

    pub fn get(&self, task_id: &str) -> Option<Arc<BackgroundTask>> {
        self.tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(task_id.trim()).cloned())
    }

    pub fn list(&self) -> Vec<Arc<BackgroundTask>> {
        let mut items: Vec<Arc<BackgroundTask>> = self
            .tasks
            .lock()
            .map(|tasks| tasks.values().cloned().collect())
            .unwrap_or_default();
        items.sort_by(|a, b| a.id.cmp(&b.id));
        items
    }

    pub fn finish(&self, task_id: &str, outcome: Result<String, String>) {
        let Some(task) = self.get(task_id) else {
            return;
        };
        if let Ok(mut status) = task.status.lock() {
            if *status == TaskStatus::Running {
                *status = match outcome {
                    Ok(report) => TaskStatus::Done(report),
                    Err(error) if task.cancel.is_cancelled() => {
                        let _ = error;
                        TaskStatus::Stopped
                    }
                    Err(error) => TaskStatus::Failed(error),
                };
            }
        }
        task.notify.notify_waiters();
    }

    /// 取消任务；已结束返回 false。
    pub fn stop(&self, task_id: &str) -> Option<bool> {
        let task = self.get(task_id)?;
        if task.status().is_finished() {
            return Some(false);
        }
        task.cancel.cancel();
        if let Ok(mut status) = task.status.lock() {
            *status = TaskStatus::Stopped;
        }
        task.notify.notify_waiters();
        Some(true)
    }

    pub fn stop_all(&self) {
        for task in self.list() {
            if !task.status().is_finished() {
                task.cancel.cancel();
                if let Ok(mut status) = task.status.lock() {
                    *status = TaskStatus::Stopped;
                }
                task.notify.notify_waiters();
            }
        }
    }

    /// 等待任务结束，超时返回当前状态。
    pub async fn wait(&self, task_id: &str, timeout: Duration) -> Option<TaskStatus> {
        let task = self.get(task_id)?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let status = task.status();
            if status.is_finished() {
                return Some(status);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Some(status);
            }
            let _ = tokio::time::timeout(
                remaining.min(Duration::from_millis(500)),
                task.notify.notified(),
            )
            .await;
        }
    }

    /// 父 → 子：把消息塞进子 Agent 的 steer 通道。
    pub async fn send_message(&self, task_id: &str, message: &str) -> Result<(), String> {
        let task = self
            .get(task_id)
            .ok_or_else(|| format!("未知任务：{task_id}"))?;
        if task.status().is_finished() {
            return Err(format!(
                "任务 {task_id} 已结束（{}）",
                task.status().label()
            ));
        }
        task.steer_tx
            .send(NativeFollowup::input(message))
            .await
            .map_err(|_| format!("任务 {task_id} 的消息通道已关闭"))
    }

    /// 子 → 父：留言进父 Agent 的收件箱。
    pub fn push_inbox(&self, task_id: &str, message: &str) {
        let description = self
            .get(task_id)
            .map(|task| task.description.clone())
            .unwrap_or_default();
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(CoordinatorMessage {
                task_id: task_id.to_string(),
                description,
                message: message.to_string(),
            });
        }
    }

    pub fn drain_inbox(&self) -> Vec<CoordinatorMessage> {
        self.inbox
            .lock()
            .map(|mut inbox| std::mem::take(&mut *inbox))
            .unwrap_or_default()
    }

    /// 尚未向父 Agent 宣告过的已结束任务。
    pub fn drain_finished(&self) -> Vec<Arc<BackgroundTask>> {
        let mut out = Vec::new();
        for task in self.list() {
            if !task.status().is_finished() {
                continue;
            }
            if let Ok(mut announced) = task.announced.lock() {
                if !*announced {
                    *announced = true;
                    out.push(task.clone());
                }
            }
        }
        out
    }

    /// 组装注入给父 Agent 的提醒文本；没有新事件返回 `None`。
    pub fn pending_notice(&self) -> Option<String> {
        let mut lines = Vec::new();
        for task in self.drain_finished() {
            let status = task.status();
            let summary = match &status {
                TaskStatus::Done(report) => format!("完成：{}", preview(report)),
                TaskStatus::Failed(error) => format!("失败：{}", preview(error)),
                TaskStatus::Stopped => "已停止".to_string(),
                TaskStatus::Running => continue,
            };
            lines.push(format!(
                "- 后台任务 {}（{}）{summary}。用 TaskOutput 读取完整结果。",
                task.id, task.description
            ));
        }
        for message in self.drain_inbox() {
            lines.push(format!(
                "- 任务 {}（{}）留言：{}",
                message.task_id, message.description, message.message
            ));
        }
        if lines.is_empty() {
            None
        } else {
            Some(format!("[后台任务提醒]\n{}", lines.join("\n")))
        }
    }

    /// TaskOutput 的文本表示。
    pub fn describe(&self, task_id: &str) -> Option<String> {
        let task = self.get(task_id)?;
        let status = task.status();
        Some(match status {
            TaskStatus::Running => format!(
                "任务 {} 仍在运行（{} / {}）。可用 TaskOutput(wait=true) 等待，或 SendMessage 追加指令。",
                task.id, task.kind, task.description
            ),
            TaskStatus::Done(report) => format!(
                "任务 {}（{} / {}）已完成。\n\n{report}",
                task.id, task.kind, task.description
            ),
            TaskStatus::Failed(error) => format!(
                "任务 {}（{} / {}）失败：{error}",
                task.id, task.kind, task.description
            ),
            TaskStatus::Stopped => format!(
                "任务 {}（{} / {}）已被停止。",
                task.id, task.kind, task.description
            ),
        })
    }
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 160 {
        flat
    } else {
        let prefix: String = flat.chars().take(159).collect();
        format!("{prefix}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_finish_wait_and_notice() {
        let registry = BackgroundTaskRegistry::new();
        let (task, mut steer_rx) = registry.register("检查测试", "explore");
        assert_eq!(task.id, "task-1");
        assert_eq!(task.status(), TaskStatus::Running);
        assert!(registry.describe("task-1").unwrap().contains("仍在运行"));
        let waited = registry.wait("task-1", Duration::from_millis(20)).await;
        assert_eq!(waited, Some(TaskStatus::Running));
        registry
            .send_message("task-1", "顺便看下 lint")
            .await
            .expect("send");
        match steer_rx.recv().await {
            Some(NativeFollowup::Input { text, images }) => {
                assert_eq!(text, "顺便看下 lint");
                assert!(images.is_empty());
            }
            _ => panic!("expected input"),
        }
        registry.push_inbox("task-1", "需要确认范围");
        registry.finish("task-1", Ok("全部通过".to_string()));
        let waited = registry.wait("task-1", Duration::from_secs(1)).await;
        assert_eq!(waited, Some(TaskStatus::Done("全部通过".to_string())));
        let notice = registry.pending_notice().expect("notice");
        assert!(notice.contains("task-1"));
        assert!(notice.contains("完成"));
        assert!(notice.contains("需要确认范围"));
        // 宣告过一次后不再重复。
        assert!(registry.pending_notice().is_none());
        assert!(registry.send_message("task-1", "x").await.is_err());
        assert_eq!(registry.stop("task-1"), Some(false));
        assert_eq!(registry.stop("missing"), None);
    }

    #[test]
    fn stop_marks_running_task_stopped_and_cancels_flag() {
        let registry = BackgroundTaskRegistry::new();
        let (task, _rx) = registry.register("长任务", "general");
        assert_eq!(registry.stop(&task.id), Some(true));
        assert!(task.cancel.is_cancelled());
        assert_eq!(task.status(), TaskStatus::Stopped);
        // 结束后 finish 不再覆盖状态。
        registry.finish(&task.id, Ok("late".to_string()));
        assert_eq!(task.status(), TaskStatus::Stopped);
        let (second, _rx2) = registry.register("另一个", "general");
        registry.stop_all();
        assert_eq!(second.status(), TaskStatus::Stopped);
    }
}
