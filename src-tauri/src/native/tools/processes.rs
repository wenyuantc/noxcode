//! 会话内后台 Bash / Monitor 进程表。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::process::Child;
use tokio::sync::Notify;

use super::cancel::CancelFlag;
use super::local::{read_bounded, terminate_bash, BoundedOutput, BASH_OUTPUT_HARD_LIMIT};

const OUTPUT_PREVIEW: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    Exited,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSnapshot {
    pub process_id: String,
    pub command: String,
    pub description: String,
    pub status: ProcessStatus,
    pub pid: Option<u32>,
    pub started_at_ms: u64,
    pub exit_code: Option<i32>,
    pub output_preview: String,
}

struct ProcessEntry {
    snapshot: ProcessSnapshot,
    output: Arc<Mutex<String>>,
    done: Arc<Notify>,
    stop: CancelFlag,
}

pub struct ProcessRegistry {
    next_id: AtomicU64,
    items: Mutex<HashMap<String, ProcessEntry>>,
    on_change: Mutex<Option<ProcessChangeCallback>>,
}

type ProcessChangeCallback = Arc<dyn Fn(Vec<ProcessSnapshot>) + Send + Sync>;

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            items: Mutex::new(HashMap::new()),
            on_change: Mutex::new(None),
        }
    }

    pub fn set_on_change(&self, callback: Option<ProcessChangeCallback>) {
        if let Ok(mut slot) = self.on_change.lock() {
            *slot = callback;
        }
        self.emit();
    }

    pub fn snapshots(&self) -> Vec<ProcessSnapshot> {
        self.items
            .lock()
            .map(|items| {
                let mut list: Vec<_> = items.values().map(refresh_preview).collect();
                list.sort_by_key(|a| a.started_at_ms);
                list
            })
            .unwrap_or_default()
    }

    pub fn start(
        self: &Arc<Self>,
        command: String,
        description: String,
        mut child: Child,
        session_cancel: CancelFlag,
    ) -> Result<ProcessSnapshot, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst).to_string();
        let pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "后台 Bash stdout 不可用".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "后台 Bash stderr 不可用".to_string())?;
        let output = Arc::new(Mutex::new(String::new()));
        let done = Arc::new(Notify::new());
        let stop = CancelFlag::new();
        let snapshot = ProcessSnapshot {
            process_id: id.clone(),
            command,
            description,
            status: ProcessStatus::Running,
            pid,
            started_at_ms: now_ms(),
            exit_code: None,
            output_preview: String::new(),
        };
        {
            let mut items = self.items.lock().map_err(|_| "进程表不可用".to_string())?;
            items.insert(
                id.clone(),
                ProcessEntry {
                    snapshot: snapshot.clone(),
                    output: output.clone(),
                    done: done.clone(),
                    stop: stop.clone(),
                },
            );
        }
        self.emit();

        let out_stdout = output.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(buf) = read_bounded(stdout, BASH_OUTPUT_HARD_LIMIT / 2).await {
                append_output(&out_stdout, &buf);
            }
        });
        let out_stderr = output.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(buf) = read_bounded(stderr, BASH_OUTPUT_HARD_LIMIT / 2).await {
                append_output(&out_stderr, &buf);
            }
        });

        let registry = Arc::clone(self);
        let watch_id = id.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = tokio::select! {
                result = child.wait() => match result {
                    Ok(status) => WaitOutcome::Exited(status.code().unwrap_or(-1)),
                    Err(_) => WaitOutcome::Failed,
                },
                _ = wait_cancel(&stop) => {
                    terminate_bash(&mut child, pid).await;
                    WaitOutcome::Stopped
                }
                _ = wait_cancel(&session_cancel) => {
                    terminate_bash(&mut child, pid).await;
                    WaitOutcome::Stopped
                }
            };
            registry.finish(&watch_id, outcome);
        });
        Ok(snapshot)
    }

    pub async fn output(
        &self,
        process_id: &str,
        wait: bool,
        timeout_ms: Option<u64>,
    ) -> Result<String, String> {
        let done = {
            let items = self.items.lock().map_err(|_| "进程表不可用".to_string())?;
            let entry = items
                .get(process_id)
                .ok_or_else(|| format!("进程不存在: {process_id}"))?;
            if !wait || !matches!(entry.snapshot.status, ProcessStatus::Running) {
                return Ok(format_output(&entry.snapshot, &entry.output));
            }
            entry.done.clone()
        };
        if let Some(ms) = timeout_ms {
            let _ = tokio::time::timeout(Duration::from_millis(ms.max(1)), done.notified()).await;
        } else {
            done.notified().await;
        }
        let items = self.items.lock().map_err(|_| "进程表不可用".to_string())?;
        let entry = items
            .get(process_id)
            .ok_or_else(|| format!("进程不存在: {process_id}"))?;
        Ok(format_output(&entry.snapshot, &entry.output))
    }

    pub fn stop(&self, process_id: &str) -> Result<ProcessSnapshot, String> {
        let items = self.items.lock().map_err(|_| "进程表不可用".to_string())?;
        let entry = items
            .get(process_id)
            .ok_or_else(|| format!("进程不存在: {process_id}"))?;
        if matches!(entry.snapshot.status, ProcessStatus::Running) {
            entry.stop.cancel();
        }
        Ok(refresh_preview(entry))
    }

    pub fn stop_all(&self) {
        if let Ok(items) = self.items.lock() {
            for entry in items.values() {
                if matches!(entry.snapshot.status, ProcessStatus::Running) {
                    entry.stop.cancel();
                }
            }
        }
    }

    fn finish(&self, process_id: &str, outcome: WaitOutcome) {
        if let Ok(mut items) = self.items.lock() {
            if let Some(entry) = items.get_mut(process_id) {
                match outcome {
                    WaitOutcome::Exited(code) => {
                        entry.snapshot.status = if code == 0 {
                            ProcessStatus::Exited
                        } else {
                            ProcessStatus::Failed
                        };
                        entry.snapshot.exit_code = Some(code);
                    }
                    WaitOutcome::Stopped => {
                        entry.snapshot.status = ProcessStatus::Stopped;
                    }
                    WaitOutcome::Failed => {
                        entry.snapshot.status = ProcessStatus::Failed;
                    }
                }
                entry.done.notify_waiters();
            }
        }
        self.emit();
    }

    fn emit(&self) {
        let snapshots = self.snapshots();
        if let Ok(callback) = self.on_change.lock() {
            if let Some(callback) = callback.as_ref() {
                callback(snapshots);
            }
        }
    }
}

enum WaitOutcome {
    Exited(i32),
    Stopped,
    Failed,
}

fn append_output(output: &Mutex<String>, buf: &BoundedOutput) {
    if let Ok(mut text) = output.lock() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&buf.snapshot_text());
    }
}

async fn wait_cancel(flag: &CancelFlag) {
    loop {
        if flag.is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn refresh_preview(entry: &ProcessEntry) -> ProcessSnapshot {
    let mut snapshot = entry.snapshot.clone();
    if let Ok(text) = entry.output.lock() {
        snapshot.output_preview = tail(&text, OUTPUT_PREVIEW);
    }
    snapshot
}

fn format_output(snapshot: &ProcessSnapshot, output: &Mutex<String>) -> String {
    let body = output.lock().map(|text| text.clone()).unwrap_or_default();
    format!(
        "process_id={}\nstatus={:?}\nexit_code={}\ncommand={}\n\n{}",
        snapshot.process_id,
        snapshot.status,
        snapshot
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".to_string()),
        snapshot.command,
        if body.is_empty() {
            "(no output)"
        } else {
            &body
        }
    )
}

fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    text[text.len() - max..].to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_spawn::tokio_command;
    use std::process::Stdio;

    #[tokio::test]
    async fn start_list_and_stop_background_process() {
        let registry = Arc::new(ProcessRegistry::new());
        let mut cmd = tokio_command("bash");
        cmd.arg("-lc")
            .arg("while true; do echo tick; sleep 0.2; done")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().expect("spawn");
        let snap = registry
            .start(
                "sleep-loop".into(),
                "测试后台进程".into(),
                child,
                CancelFlag::new(),
            )
            .expect("start");
        assert_eq!(snap.status, ProcessStatus::Running);
        assert_eq!(registry.snapshots().len(), 1);
        tokio::time::sleep(Duration::from_millis(80)).await;
        registry.stop(&snap.process_id).expect("stop");
        let text = registry
            .output(&snap.process_id, true, Some(2_000))
            .await
            .expect("output");
        assert!(text.contains("process_id="));
        let final_snap = registry
            .snapshots()
            .into_iter()
            .find(|item| item.process_id == snap.process_id)
            .expect("listed");
        assert_ne!(final_snap.status, ProcessStatus::Running);
    }
}
