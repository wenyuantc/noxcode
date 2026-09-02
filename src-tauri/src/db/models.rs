#![allow(dead_code)]

use serde::{Deserialize, Deserializer, Serialize};
use sqlx::FromRow;

fn deserialize_explicit_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
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
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub ai_channel_id: Option<String>,
    pub model: String,
    pub reasoning_effort: String,
    pub system_prompt: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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
    pub profile_id: Option<String>,
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
