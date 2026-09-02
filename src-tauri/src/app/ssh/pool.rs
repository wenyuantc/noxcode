use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::Handle;
use russh::Disconnect;

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
    shutting_down: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct SshPool(Arc<Inner>);

impl SshPool {
    pub(crate) fn new(trust: Arc<HostTrustBroker>, idle_timeout: Duration) -> Self {
        Self(Arc::new(Inner {
            entries: std::sync::Mutex::new(HashMap::new()),
            trust,
            connect_count: AtomicUsize::new(0),
            idle_timeout,
            shutting_down: AtomicBool::new(false),
        }))
    }

    pub(crate) fn trust(&self) -> &Arc<HostTrustBroker> {
        &self.0.trust
    }

    #[allow(dead_code)]
    pub(crate) fn connect_count(&self) -> usize {
        self.0.connect_count.load(Ordering::SeqCst)
    }

    fn entry_for(&self, ssh_config_id: &str) -> Arc<Entry> {
        let mut entries = self.0.entries.lock().expect("ssh pool entries lock");
        entries
            .entry(ssh_config_id.to_string())
            .or_insert_with(|| {
                Arc::new(Entry {
                    slot: tokio::sync::Mutex::new(None),
                })
            })
            .clone()
    }

    async fn disconnect_live(live: Live) {
        let _ = live
            .handle
            .disconnect(Disconnect::ByApplication, "idle", "en")
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
        let entry = self.entry_for(&params.ssh_config_id);
        let mut slot = entry.slot.lock().await;
        self.ensure_live(&mut slot, params).await?;
        match slot.as_ref() {
            Some(live) => match live.handle.channel_open_session().await {
                Ok(channel) => {
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
    }

    pub(crate) async fn invalidate(&self, ssh_config_id: &str) {
        let entry = {
            let entries = self.0.entries.lock().expect("ssh pool entries lock");
            entries.get(ssh_config_id).cloned()
        };
        if let Some(entry) = entry {
            let mut slot = entry.slot.lock().await;
            if let Some(live) = slot.take() {
                Self::disconnect_live(live).await;
            }
        }
    }

    pub(crate) fn start_idle_reaper(&self, interval: Duration) {
        let inner = self.0.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if inner.shutting_down.load(Ordering::Relaxed) {
                    break;
                }
                let entries: Vec<Arc<Entry>> = {
                    let map = inner.entries.lock().expect("ssh pool entries lock");
                    map.values().cloned().collect()
                };
                for entry in entries {
                    if let Ok(mut slot) = entry.slot.try_lock() {
                        let expired = slot
                            .as_ref()
                            .is_some_and(|live| live.last_used.elapsed() > inner.idle_timeout);
                        if expired {
                            if let Some(live) = slot.take() {
                                SshPool::disconnect_live(live).await;
                            }
                        }
                    }
                }
            }
        });
    }

    pub(crate) async fn shutdown(&self) {
        self.0.shutting_down.store(true, Ordering::Relaxed);
        let entries: Vec<Arc<Entry>> = {
            let mut map = self.0.entries.lock().expect("ssh pool entries lock");
            map.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            let mut slot = entry.slot.lock().await;
            if let Some(live) = slot.take() {
                Self::disconnect_live(live).await;
            }
        }
    }
}
