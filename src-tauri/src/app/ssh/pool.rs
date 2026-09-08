use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::Handle;
use russh::Disconnect;

use crate::native::tools::CancelFlag;

use super::client::{connect_and_authenticate, ClientHandler, ConnectParams};
use super::error::SshError;
use super::known_hosts::HostTrustBroker;

struct Live {
    handle: Handle<ClientHandler>,
    fingerprint: String,
    last_used: Instant,
}

struct Entry {
    slot: tokio::sync::Mutex<Option<Live>>,
}

struct Inner {
    entries: std::sync::Mutex<HashMap<String, Arc<Entry>>>,
    trust: Arc<HostTrustBroker>,
    connect_count: AtomicUsize,
    idle_timeout: Duration,
    stopping: CancelFlag,
    app_stopping: CancelFlag,
}

#[derive(Clone)]
pub(crate) struct SshPool(Arc<Inner>);

impl SshPool {
    pub(crate) fn new(trust: Arc<HostTrustBroker>, idle_timeout: Duration) -> Self {
        Self::with_shutdown(trust, idle_timeout, CancelFlag::new())
    }

    pub(crate) fn with_shutdown(
        trust: Arc<HostTrustBroker>,
        idle_timeout: Duration,
        app_stopping: CancelFlag,
    ) -> Self {
        Self(Arc::new(Inner {
            entries: std::sync::Mutex::new(HashMap::new()),
            trust,
            connect_count: AtomicUsize::new(0),
            idle_timeout,
            stopping: CancelFlag::new(),
            app_stopping,
        }))
    }

    pub(crate) fn trust(&self) -> &Arc<HostTrustBroker> {
        &self.0.trust
    }

    #[allow(dead_code)]
    pub(crate) fn connect_count(&self) -> usize {
        self.0.connect_count.load(Ordering::SeqCst)
    }

    fn require_running(&self) -> Result<(), SshError> {
        if self.0.stopping.is_cancelled() || self.0.app_stopping.is_cancelled() {
            Err(SshError::ShuttingDown)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn cancelled(&self) {
        tokio::select! {
            biased;
            _ = self.0.stopping.cancelled() => {},
            _ = self.0.app_stopping.cancelled() => {},
        }
    }

    fn entry_for(&self, ssh_config_id: &str) -> Result<Arc<Entry>, SshError> {
        let mut entries = self.0.entries.lock().expect("ssh pool entries lock");
        self.require_running()?;
        Ok(entries
            .entry(ssh_config_id.to_string())
            .or_insert_with(|| {
                Arc::new(Entry {
                    slot: tokio::sync::Mutex::new(None),
                })
            })
            .clone())
    }

    async fn disconnect_live(live: Live) {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            live.handle
                .disconnect(Disconnect::ByApplication, "closing", "en"),
        )
        .await;
    }

    async fn ensure_live(
        &self,
        slot: &mut Option<Live>,
        params: &ConnectParams,
    ) -> Result<(), SshError> {
        if let Some(live) = slot.as_ref() {
            let same_fingerprint = live.fingerprint == params.fingerprint();
            if same_fingerprint
                && !live.handle.is_closed()
                && live.handle.send_keepalive(true).await.is_ok()
            {
                if let Some(live) = slot.as_mut() {
                    live.last_used = Instant::now();
                }
                return Ok(());
            }
        }

        if let Some(old) = slot.take() {
            Self::disconnect_live(old).await;
        }

        self.0.connect_count.fetch_add(1, Ordering::SeqCst);
        let handle = connect_and_authenticate(params, &self.0.trust).await?;
        *slot = Some(Live {
            handle,
            fingerprint: params.fingerprint(),
            last_used: Instant::now(),
        });
        Ok(())
    }

    pub(crate) async fn open_session(
        &self,
        params: &ConnectParams,
    ) -> Result<russh::Channel<russh::client::Msg>, SshError> {
        let entry = self.entry_for(&params.ssh_config_id)?;
        tokio::select! {
            biased;
            _ = self.cancelled() => Err(SshError::ShuttingDown),
            result = async {
                let mut slot = entry.slot.lock().await;
                self.ensure_live(&mut slot, params).await?;
                match slot.as_ref() {
                    Some(live) => match live.handle.channel_open_session().await {
                        Ok(channel) => {
                            self.require_running()?;
                            if let Some(live) = slot.as_mut() {
                                live.last_used = Instant::now();
                            }
                            Ok(channel)
                        }
                        Err(_) => {
                            slot.take();
                            Err(SshError::ConnectionLost)
                        }
                    },
                    None => Err(SshError::ConnectionLost),
                }
            } => result,
        }
    }

    pub(crate) async fn invalidate(&self, ssh_config_id: &str) {
        let entry = {
            let entries = self.0.entries.lock().expect("ssh pool entries lock");
            entries.get(ssh_config_id).cloned()
        };
        if let Some(entry) = entry {
            tokio::select! {
                biased;
                _ = self.cancelled() => {},
                _ = async {
                    let mut slot = entry.slot.lock().await;
                    if let Some(live) = slot.take() {
                        Self::disconnect_live(live).await;
                    }
                } => {},
            }
        }
    }

    pub(crate) fn start_idle_reaper(&self, interval: Duration) {
        let pool = self.clone();
        tauri::async_runtime::spawn(async move {
            let reap = async {
                loop {
                    tokio::time::sleep(interval).await;
                    let entries: Vec<Arc<Entry>> = {
                        let map = pool.0.entries.lock().expect("ssh pool entries lock");
                        map.values().cloned().collect()
                    };
                    for entry in entries {
                        if let Ok(mut slot) = entry.slot.try_lock() {
                            let expired = slot
                                .as_ref()
                                .is_some_and(|live| live.last_used.elapsed() > pool.0.idle_timeout);
                            if expired {
                                if let Some(live) = slot.take() {
                                    SshPool::disconnect_live(live).await;
                                }
                            }
                        }
                    }
                }
            };
            tokio::select! {
                biased;
                _ = pool.cancelled() => {},
                _ = reap => {},
            }
        });
    }

    pub(crate) async fn shutdown(&self) -> (usize, usize) {
        self.0.stopping.cancel();
        let entries: Vec<Arc<Entry>> = {
            let mut map = self.0.entries.lock().expect("ssh pool entries lock");
            map.drain().map(|(_, entry)| entry).collect()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let results = futures_util::future::join_all(entries.into_iter().map(|entry| async move {
            tokio::time::timeout_at(deadline, async {
                let mut slot = entry.slot.lock().await;
                if let Some(live) = slot.take() {
                    Self::disconnect_live(live).await;
                }
            })
            .await
            .is_ok()
        }))
        .await;
        (
            results.len(),
            results.iter().filter(|finished| !**finished).count(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::AuthMaterial;
    use super::super::known_hosts::KnownHostsPolicy;
    use super::*;

    fn params() -> ConnectParams {
        ConnectParams {
            ssh_config_id: "test".into(),
            name: "test".into(),
            host: "127.0.0.1".into(),
            port: 1,
            username: "test".into(),
            auth: AuthMaterial::Password("test".into()),
            policy: KnownHostsPolicy::Off,
            known_hosts_path: "/unused".into(),
            algorithms: None,
        }
    }

    fn pool(stopping: CancelFlag) -> SshPool {
        SshPool::with_shutdown(
            Arc::new(HostTrustBroker::new(Duration::from_secs(120))),
            Duration::from_secs(600),
            stopping,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn locked_entries_share_one_shutdown_deadline() {
        let pool = pool(CancelFlag::new());
        let mut locks = Vec::new();
        for id in ["a", "b", "c"] {
            let entry = pool.entry_for(id).unwrap();
            locks.push(tokio::spawn(async move {
                let _guard = entry.slot.lock().await;
                std::future::pending::<()>().await;
            }));
        }
        tokio::task::yield_now().await;
        let started = tokio::time::Instant::now();
        assert_eq!(pool.shutdown().await, (3, 3));
        assert_eq!(started.elapsed(), Duration::from_secs(1));
        assert!(matches!(
            pool.open_session(&params()).await,
            Err(SshError::ShuttingDown)
        ));
        for lock in locks {
            lock.abort();
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_a_pending_connection_lock_without_connecting() {
        let stopping = CancelFlag::new();
        let pool = pool(stopping.clone());
        let entry = pool.entry_for("test").unwrap();
        let _lock = entry.slot.lock().await;
        let pending_pool = pool.clone();
        let connection = tokio::spawn(async move { pending_pool.open_session(&params()).await });
        tokio::task::yield_now().await;
        stopping.cancel();
        assert!(matches!(
            connection.await.unwrap(),
            Err(SshError::ShuttingDown)
        ));
        assert_eq!(pool.connect_count(), 0);
        assert!(pool.entry_for("new").is_err());
    }

    #[tokio::test]
    async fn shutdown_cancels_a_stalled_ssh_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stopping = CancelFlag::new();
        let pool = pool(stopping.clone());
        let mut params = params();
        params.port = listener.local_addr().unwrap().port();
        let connection = tokio::spawn(async move { pool.open_session(&params).await });
        let (_socket, _) = listener.accept().await.unwrap();
        stopping.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), connection)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(SshError::ShuttingDown)));
    }
}
