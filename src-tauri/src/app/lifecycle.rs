use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_sql::{DbInstances, DbPool};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{timeout_at, Instant};

use crate::native::manager::{shutdown_all_sessions, NativeAgentManager};
use crate::native::tools::CancelFlag;

const RUNNING: u8 = 0;
const DRAINING: u8 = 1;
const EXITING: u8 = 2;
const RESOURCE_BUDGET: Duration = Duration::from_secs(1);
const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitIntent {
    Restart,
    Quit(i32),
}

impl ExitIntent {
    fn budget(self) -> Duration {
        Duration::from_secs(if self == Self::Restart { 5 } else { 30 })
    }
}

#[derive(Default)]
struct ShutdownState {
    phase: AtomicU8,
    restarting: AtomicBool,
    stopping: CancelFlag,
}

impl ShutdownState {
    fn begin(&self, intent: ExitIntent) -> bool {
        if self
            .phase
            .compare_exchange(RUNNING, DRAINING, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        self.restarting
            .store(intent == ExitIntent::Restart, Ordering::SeqCst);
        self.stopping.cancel();
        true
    }

    fn finish(&self) -> bool {
        self.phase
            .compare_exchange(DRAINING, EXITING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

#[derive(Clone)]
pub(crate) struct Lifecycle {
    state: Arc<ShutdownState>,
    log: mpsc::SyncSender<Value>,
    version: String,
    started: std::time::Instant,
}

impl Lifecycle {
    pub(crate) fn new<R: Runtime>(app: &AppHandle<R>) -> Self {
        let (log, records) = mpsc::sync_channel(64);
        let directory = app.path().app_log_dir().ok();
        // Filesystem stalls must not hold the UI or the shutdown supervisor.
        let _ = std::thread::Builder::new()
            .name("lifecycle-log".into())
            .spawn(move || {
                for record in records {
                    if let Some(directory) = directory.as_ref() {
                        let _ = append_log(directory, &record);
                    }
                }
            });
        let lifecycle = Self {
            state: Arc::default(),
            log,
            version: app.package_info().version.to_string(),
            started: std::time::Instant::now(),
        };
        lifecycle.record("startup", json!({}));
        lifecycle
    }

    pub(crate) fn stopping(&self) -> CancelFlag {
        self.state.stopping.clone()
    }

    pub(crate) fn record(&self, event: &str, detail: Value) {
        let _ = self.log.try_send(json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "pid": std::process::id(),
            "version": self.version,
            "elapsed_ms": self.started.elapsed().as_millis(),
            "event": event,
            "detail": detail,
        }));
    }
}

fn append_log(directory: &PathBuf, record: &Value) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    // Separate processes (including development builds) must not rotate each other's log.
    let path = directory.join(format!("lifecycle-{}.jsonl", std::process::id()));
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES) {
        let previous = path.with_extension("jsonl.1");
        let _ = fs::remove_file(&previous);
        fs::rename(&path, previous)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record)?;
    file.write_all(b"\n")
}

pub(crate) fn is_stopping<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<Lifecycle>()
        .is_some_and(|lifecycle| lifecycle.state.stopping.is_cancelled())
}

pub(crate) fn require_running<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if is_stopping(app) {
        Err("应用正在退出，请在重新打开后重试".into())
    } else {
        Ok(())
    }
}

pub(crate) async fn fast_restart_requested<R: Runtime>(app: &AppHandle<R>) {
    if let Some(lifecycle) = app.try_state::<Lifecycle>() {
        lifecycle.state.stopping.cancelled().await;
        if lifecycle.state.restarting.load(Ordering::SeqCst) {
            return;
        }
    }
    std::future::pending().await
}

pub(crate) fn request<R: Runtime>(app: &AppHandle<R>, intent: ExitIntent) -> Result<(), String> {
    let lifecycle = app
        .try_state::<Lifecycle>()
        .ok_or("退出协调器尚未初始化")?
        .inner()
        .clone();
    if !lifecycle.state.begin(intent) {
        return Ok(());
    }
    let started = Instant::now();
    lifecycle.record(
        "shutdown_started",
        json!({"intent": format!("{intent:?}"), "budget_ms": intent.budget().as_millis()}),
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        drain(&app, &lifecycle, intent, started).await;
        if lifecycle.state.finish() {
            lifecycle.record("exit_dispatched", json!({"intent": format!("{intent:?}"), "shutdown_ms": started.elapsed().as_millis()}));
            match intent {
                ExitIntent::Restart => app.request_restart(),
                ExitIntent::Quit(code) => app.exit(code),
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<(), String> {
    request(&app, ExitIntent::Restart)
}

pub(crate) fn handle_exit_requested<R: Runtime>(
    app: &AppHandle<R>,
    code: Option<i32>,
    api: &tauri::ExitRequestApi,
) {
    let Some(lifecycle) = app.try_state::<Lifecycle>() else {
        return;
    };
    if lifecycle.state.phase.load(Ordering::SeqCst) != EXITING {
        api.prevent_exit();
        let _ = request(app, ExitIntent::Quit(code.unwrap_or(0)));
    }
}

async fn run_stage<F>(lifecycle: &Lifecycle, name: &str, deadline: Instant, work: F)
where
    F: Future<Output = Value> + Send + 'static,
{
    let started = Instant::now();
    lifecycle.record("stage_started", json!({"stage": name}));
    if started >= deadline {
        lifecycle.record(
            "stage_finished",
            json!({"stage": name, "result": {"status": "timed_out"}}),
        );
        return;
    }
    // A separate task lets the supervisor time out even if a cleanup future blocks a worker.
    let mut jobs = JoinSet::new();
    jobs.spawn(work);
    let result = match timeout_at(deadline, jobs.join_next()).await {
        Ok(Some(Ok(detail))) => json!({"status": "finished", "resources": detail}),
        Ok(Some(Err(error))) => json!({"status": "failed", "error": error.to_string()}),
        _ => json!({"status": "timed_out"}),
    };
    lifecycle.record(
        "stage_finished",
        json!({"stage": name, "elapsed_ms": started.elapsed().as_millis(), "result": result}),
    );
    // JoinSet aborts pending work on drop, without an unbounded join after the deadline.
}

async fn drain<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &Lifecycle,
    intent: ExitIntent,
    started: Instant,
) {
    let deadline = started + intent.budget();
    let session_deadline = deadline - RESOURCE_BUDGET * 2;
    // SQL's Exit hook blocks until all borrowed connections return. The coordinator owns
    // that wait now; DbInstances remains available until session persistence has finished.
    lifecycle.record("sql_exit_hook_removing", json!({}));
    let removed = app.remove_plugin("sql");
    lifecycle.record("sql_exit_hook_removed", json!({"removed": removed}));

    let session_app = app.clone();
    let session_log = lifecycle.clone();
    run_stage(
        lifecycle,
        "sessions_and_window",
        session_deadline,
        async move {
            let save = crate::window_state::save_main_window_size_async(&session_app);
            let sessions = async {
                if let Some(manager) = session_app.try_state::<Arc<Mutex<NativeAgentManager>>>() {
                    let count = manager.lock().await.len();
                    session_log.record(
                        "resource_count",
                        json!({"stage": "sessions", "count": count}),
                    );
                    shutdown_all_sessions(&manager, intent != ExitIntent::Restart, session_deadline)
                        .await
                } else {
                    Ok((0, 0))
                }
            };
            let (window, sessions) = tokio::join!(save, sessions);
            let sessions = match sessions {
                Ok((count, timed_out)) => json!({"count": count, "timed_out": timed_out}),
                Err(error) => json!({"error": error}),
            };
            json!({"window": window, "sessions": sessions})
        },
    )
    .await;

    let ssh = app
        .try_state::<super::ssh::SshPool>()
        .map(|pool| pool.inner().clone());
    run_stage(
        lifecycle,
        "ssh",
        deadline.min(Instant::now() + RESOURCE_BUDGET),
        async move {
            match ssh {
                Some(pool) => {
                    let (count, timed_out) = pool.shutdown().await;
                    json!({"connections": count, "timed_out": timed_out})
                }
                None => json!({"connections": 0}),
            }
        },
    )
    .await;

    let database_app = app.clone();
    let database_log = lifecycle.clone();
    run_stage(
        lifecycle,
        "database",
        deadline.min(Instant::now() + RESOURCE_BUDGET),
        async move {
            let Some(instances) = database_app.try_state::<DbInstances>() else {
                return json!({"pools": 0});
            };
            let pools = std::mem::take(&mut *instances.0.write().await);
            let count = pools.len();
            database_log.record(
                "resource_count",
                json!({"stage": "database", "count": count}),
            );
            futures_util::future::join_all(pools.values().map(|pool| {
                let DbPool::Sqlite(pool) = pool;
                pool.close()
            }))
            .await;
            json!({"pools": count})
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_lifecycle() -> Lifecycle {
        let (log, _records) = mpsc::sync_channel(64);
        Lifecycle {
            state: Arc::default(),
            log,
            version: "test".into(),
            started: std::time::Instant::now(),
        }
    }

    #[test]
    fn first_intent_wins_and_exit_is_dispatched_once() {
        for intent in [ExitIntent::Restart, ExitIntent::Quit(0)] {
            let state = Arc::new(ShutdownState::default());
            assert!(state.begin(intent));
            std::thread::scope(|scope| {
                for _ in 0..16 {
                    let state = state.clone();
                    scope.spawn(move || assert!(!state.begin(ExitIntent::Restart)));
                }
            });
            assert_eq!(
                state.restarting.load(Ordering::SeqCst),
                intent == ExitIntent::Restart
            );
            assert!(state.stopping.is_cancelled());
            assert!(state.finish());
            assert!(!state.finish());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stuck_stages_share_the_five_second_budget() {
        let lifecycle = test_lifecycle();
        let started = Instant::now();
        let deadline = started + ExitIntent::Restart.budget();
        run_stage(
            &lifecycle,
            "sessions",
            deadline - RESOURCE_BUDGET * 2,
            std::future::pending(),
        )
        .await;
        run_stage(
            &lifecycle,
            "ssh",
            deadline.min(Instant::now() + RESOURCE_BUDGET),
            std::future::pending(),
        )
        .await;
        run_stage(
            &lifecycle,
            "database",
            deadline.min(Instant::now() + RESOURCE_BUDGET),
            std::future::pending(),
        )
        .await;
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn borrowed_database_connection_does_not_hold_up_exit() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let connection = pool.acquire().await.unwrap();
        tokio::time::pause();
        let closing = pool.clone();
        let started = Instant::now();
        run_stage(
            &test_lifecycle(),
            "database",
            started + RESOURCE_BUDGET,
            async move {
                closing.close().await;
                json!({})
            },
        )
        .await;
        // Pausing after real SQLite I/O can leave a fractional timer tick.
        assert!(started.elapsed() >= RESOURCE_BUDGET);
        assert!(started.elapsed() <= RESOURCE_BUDGET + Duration::from_millis(1));
        assert!(pool.is_closed());
        drop(connection);
    }

    #[test]
    fn log_rotates_at_the_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(format!("lifecycle-{}.jsonl", std::process::id()));
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(MAX_LOG_BYTES).unwrap();
        append_log(
            &directory.path().to_path_buf(),
            &json!({"event": "startup"}),
        )
        .unwrap();
        assert!(path.with_extension("jsonl.1").exists());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "{\"event\":\"startup\"}\n"
        );
    }
}
