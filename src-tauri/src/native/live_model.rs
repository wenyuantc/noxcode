use std::sync::{Arc, Mutex};

use crate::native::model::client::ModelClient;
use crate::native::tools::hooks::HookAgentHandler;

#[derive(Clone)]
pub struct LiveModelSnapshot {
    pub revision: u64,
    pub client: ModelClient,
    pub model: String,
    pub channel_id: String,
    pub channel_name: String,
    pub protocol: String,
    pub lite_model: Option<String>,
    pub effort: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub thinking_enabled: bool,
    pub context_tokens: Option<u32>,
    pub context_token_limit: usize,
    pub execution_target: Option<String>,
    pub hook_agent: Option<HookAgentHandler>,
}

pub type SharedLiveModel = Arc<Mutex<LiveModelSnapshot>>;

pub fn write_live_model(slot: &SharedLiveModel, next: LiveModelSnapshot) -> u64 {
    let mut guard = slot.lock().unwrap_or_else(|error| error.into_inner());
    let revision = guard.revision.saturating_add(1);
    *guard = LiveModelSnapshot { revision, ..next };
    revision
}
