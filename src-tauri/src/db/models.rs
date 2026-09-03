#![allow(dead_code)]

use serde::{Deserialize, Deserializer, Serialize};
use sqlx::FromRow;

use crate::app::ssh::algorithms::SshAlgorithms;

fn deserialize_explicit_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

fn default_scope_all() -> String {
    "all".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SshConfigRecord {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: Option<String>,
    pub password_ref: Option<String>,
    pub passphrase_ref: Option<String>,
    pub known_hosts_mode: String,
    pub algorithms_json: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_check_status: Option<String>,
    pub last_check_message: Option<String>,
    pub password_probe_checked_at: Option<String>,
    pub password_probe_status: Option<String>,
    pub password_probe_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: Option<String>,
    pub known_hosts_mode: String,
    pub algorithms: Option<SshAlgorithms>,
    pub last_checked_at: Option<String>,
    pub last_check_status: Option<String>,
    pub last_check_message: Option<String>,
    pub password_probe_checked_at: Option<String>,
    pub password_probe_status: Option<String>,
    pub password_probe_message: Option<String>,
    pub password_configured: bool,
    pub passphrase_configured: bool,
    pub password_execution_allowed: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<SshConfigRecord> for SshConfig {
    fn from(value: SshConfigRecord) -> Self {
        let password_probe_status = value.password_probe_status.clone();
        Self {
            id: value.id,
            name: value.name,
            host: value.host,
            port: value.port,
            username: value.username,
            auth_type: value.auth_type,
            private_key_path: value.private_key_path,
            known_hosts_mode: value.known_hosts_mode,
            algorithms: value
                .algorithms_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            last_checked_at: value.last_checked_at,
            last_check_status: value.last_check_status,
            last_check_message: value.last_check_message,
            password_probe_checked_at: value.password_probe_checked_at,
            password_probe_status,
            password_probe_message: value.password_probe_message,
            password_configured: value.password_ref.is_some(),
            passphrase_configured: value.passphrase_ref.is_some(),
            password_execution_allowed: matches!(
                value.password_probe_status.as_deref(),
                Some("passed" | "available")
            ),
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSshConfig {
    pub name: String,
    pub host: String,
    pub port: Option<i64>,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: Option<String>,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    pub known_hosts_mode: Option<String>,
    pub algorithms: Option<SshAlgorithms>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSshConfig {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub private_key_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub password: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub passphrase: Option<Option<String>>,
    pub known_hosts_mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub algorithms: Option<Option<SshAlgorithms>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordAuthProbeResult {
    pub ssh_config_id: String,
    pub target_host_label: String,
    pub supported: bool,
    pub status: String,
    pub message: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiChannelRecord {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub extra_headers_json: Option<String>,
    pub models_json: String,
    pub enabled: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelModelConfig {
    pub id: String,
    #[serde(default)]
    pub context_tokens: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub thinking_levels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChannel {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub extra_headers_json: Option<String>,
    pub models: Vec<ChannelModelConfig>,
    pub enabled: bool,
    pub api_key: Option<String>,
    pub api_key_configured: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAiChannel {
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub extra_headers_json: Option<String>,
    pub models: Option<Vec<ChannelModelConfig>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAiChannel {
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub extra_headers_json: Option<Option<String>>,
    pub models: Option<Vec<ChannelModelConfig>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAiChannelPayload {
    pub id: Option<String>,
    pub protocol: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub extra_headers_json: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAiChannelResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAiChannelModelsResult {
    pub models: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub workspace_type: String,
    pub repo_path: Option<String>,
    pub ssh_config_id: Option<String>,
    pub remote_repo_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgentSessionRecord {
    pub id: String,
    pub ai_channel_id: Option<String>,
    pub workspace_id: Option<String>,
    pub working_dir: Option<String>,
    pub execution_target: String,
    pub ssh_config_id: Option<String>,
    pub target_host_label: Option<String>,
    pub session_kind: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub exit_code: Option<i32>,
    pub resume_session_id: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub created_at: String,
    pub title: Option<String>,
    pub pinned: i32,
    pub context_usage_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgentSessionEvent {
    pub id: String,
    pub session_id: String,
    pub event_type: String,
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GitCheckpoint {
    pub id: String,
    pub session_id: String,
    pub workspace_id: String,
    pub seq: i64,
    pub ref_name: String,
    pub commit_oid: String,
    pub parent_oid: Option<String>,
    pub label: Option<String>,
    pub kind: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppHealthCheck {
    pub database_loaded: bool,
    pub database_path: Option<String>,
    pub database_current_version: Option<i64>,
    pub database_current_description: Option<String>,
    pub database_latest_version: i64,
    pub git_available: bool,
    pub git_version: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseBackupResult {
    pub source_path: String,
    pub destination_path: String,
    pub database_version: Option<i64>,
    pub created_at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseRestoreResult {
    pub source_path: String,
    pub backup_path: String,
    pub database_version: Option<i64>,
    pub restored_at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeSettings {
    pub max_turns: i32,
    pub max_subagent_turns: i32,
    pub permission_mode: String,
    pub max_concurrent_subagents: i32,
    pub subagent_policy: String,
    pub context_window_tokens: i32,
    #[serde(default)]
    pub use_custom_context_window: bool,
    pub rollout_token_budget: i64,
    pub max_tool_output_tokens: i32,
    pub permission_timeout_secs: i32,
    pub subagent_budget_share_percent: i32,
    pub auto_checkpoint_after_tool_call: bool,
    pub checkpoint_retention_days: i32,
    pub desktop_notifications: bool,
    #[serde(default)]
    pub hooks: Vec<NativeHook>,
    #[serde(default)]
    pub global_prompt_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeHook {
    pub id: String,
    pub event: String,
    pub matcher: String,
    pub command: String,
    pub timeout_secs: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateNativeSettings {
    pub max_turns: Option<i32>,
    pub max_subagent_turns: Option<i32>,
    pub permission_mode: Option<String>,
    pub max_concurrent_subagents: Option<i32>,
    pub subagent_policy: Option<String>,
    pub context_window_tokens: Option<i32>,
    pub use_custom_context_window: Option<bool>,
    pub rollout_token_budget: Option<i64>,
    pub max_tool_output_tokens: Option<i32>,
    pub permission_timeout_secs: Option<i32>,
    pub subagent_budget_share_percent: Option<i32>,
    pub auto_checkpoint_after_tool_call: Option<bool>,
    pub checkpoint_retention_days: Option<i32>,
    pub desktop_notifications: Option<bool>,
    pub hooks: Option<Vec<NativeHook>>,
    pub global_prompt_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    pub enabled: bool,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default = "default_scope_all")]
    pub scope: String,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServersDocument {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateMcpServersPayload {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspace {
    pub name: String,
    pub workspace_type: String,
    pub repo_path: Option<String>,
    pub ssh_config_id: Option<String>,
    pub remote_repo_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspace {
    pub name: Option<String>,
    pub workspace_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub repo_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub ssh_config_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub remote_repo_path: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHealth {
    pub workspace_id: String,
    pub ok: bool,
    pub message: String,
    pub git_version: Option<String>,
    pub toplevel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionResumeInfo {
    pub session_id: String,
    pub resumable: bool,
    pub model: Option<String>,
    pub turns: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartNativeSessionInput {
    pub ai_channel_id: String,
    pub workspace_id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub system_prompt: Option<String>,
    pub resume_session_id: Option<String>,
    pub image_paths: Option<Vec<String>>,
    pub plan_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionStarted {
    pub profile_id: String,
    pub workspace_id: String,
    pub session_kind: String,
    pub session_record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionOutput {
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub session_record_id: String,
    pub session_event_id: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionExit {
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub session_kind: String,
    pub session_record_id: String,
    pub code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeTextDelta {
    pub session_record_id: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeContextUsage {
    pub session_record_id: String,
    pub used_tokens: usize,
    pub limit_tokens: usize,
    pub generation: u32,
    pub compactions: u32,
    #[serde(default)]
    pub mcp_tokens: usize,
    #[serde(default)]
    pub system_tool_tokens: usize,
    #[serde(default)]
    pub skill_tokens: usize,
    #[serde(default)]
    pub system_prompt_tokens: usize,
    #[serde(default)]
    pub other_tokens: usize,
    #[serde(default)]
    pub message_tokens: usize,
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub cached_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeTurnState {
    pub session_record_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NativeApiCallLogListItem {
    pub id: String,
    pub call_id: String,
    pub attempt: i64,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub protocol: String,
    pub response_encoding: Option<String>,
    pub model: Option<String>,
    pub thinking_enabled: i64,
    pub thinking_level: Option<String>,
    pub request_format: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub http_status: Option<i64>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub workspace_id: Option<String>,
    pub profile_name: Option<String>,
    pub workspace_name: Option<String>,
    pub execution_target: Option<String>,
    pub call_kind: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NativeApiCallLogDetail {
    pub id: String,
    pub call_id: String,
    pub attempt: i64,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub protocol: String,
    pub response_encoding: Option<String>,
    pub model: Option<String>,
    pub thinking_enabled: i64,
    pub thinking_level: Option<String>,
    pub request_format: String,
    pub request_body: Option<String>,
    pub request_truncated: i64,
    pub response_body: Option<String>,
    pub response_truncated: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub http_status: Option<i64>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub profile_id: Option<String>,
    pub workspace_id: Option<String>,
    pub profile_name: Option<String>,
    pub workspace_name: Option<String>,
    pub subagent_id: Option<String>,
    pub call_kind: Option<String>,
    pub execution_target: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListNativeApiCallLogsPayload {
    pub workspace_id: Option<String>,
    pub profile_id: Option<String>,
    pub execution_target: Option<String>,
    pub session_id: Option<String>,
    pub channel_name: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub include_total: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, FromRow)]
pub struct NativeApiCallLogStats {
    pub total: i64,
    pub success: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens_sum: Option<i64>,
    pub total_tokens_sum: Option<i64>,
    pub avg_first_token_ms: Option<f64>,
    pub avg_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeApiCallLogPage {
    pub items: Vec<NativeApiCallLogListItem>,
    pub total: i64,
    pub stats: NativeApiCallLogStats,
}
