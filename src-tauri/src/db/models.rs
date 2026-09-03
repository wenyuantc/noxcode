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
    /// 轻量模型（压缩摘要 / 记忆抽取 / 钩子判定用），为空则用主模型。
    #[sqlx(default)]
    pub lite_model: Option<String>,
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
    #[serde(default)]
    pub lite_model: Option<String>,
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
    #[serde(default)]
    pub lite_model: Option<String>,
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
    #[serde(default, deserialize_with = "deserialize_explicit_nullable")]
    pub lite_model: Option<Option<String>>,
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
    #[serde(default = "default_artifact_retention_days")]
    pub artifact_retention_days: i32,
    #[serde(default = "default_model_retry_max_retries")]
    pub model_retry_max_retries: i32,
    #[serde(default = "default_model_retry_base_delay_ms")]
    pub model_retry_base_delay_ms: i32,
    #[serde(default = "default_model_retry_max_delay_ms")]
    pub model_retry_max_delay_ms: i32,
    #[serde(default = "default_model_retry_backoff_factor")]
    pub model_retry_backoff_factor: f64,
    #[serde(default = "default_bash_default_timeout_secs")]
    pub bash_default_timeout_secs: i32,
    #[serde(default = "default_true")]
    pub shell_snapshot_enabled: bool,
    #[serde(default = "default_true")]
    pub rg_sidecar_enabled: bool,
    #[serde(default = "default_auto_compact_threshold_percent")]
    pub auto_compact_threshold_percent: i32,
    #[serde(default = "default_true")]
    pub microcompact_enabled: bool,
    #[serde(default = "default_true")]
    pub memory_enabled: bool,
    #[serde(default = "default_memory_dream_interval")]
    pub memory_dream_interval: i32,
    #[serde(default)]
    pub hooks: Vec<NativeHook>,
    #[serde(default)]
    pub global_prompt_template: String,
}

fn default_auto_compact_threshold_percent() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_AUTO_COMPACT_THRESHOLD_PERCENT
}

fn default_memory_dream_interval() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_MEMORY_DREAM_INTERVAL
}

fn default_true() -> bool {
    true
}

fn default_artifact_retention_days() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_ARTIFACT_RETENTION_DAYS
}

fn default_model_retry_max_retries() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_MODEL_RETRY_MAX_RETRIES
}

fn default_model_retry_base_delay_ms() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_MODEL_RETRY_BASE_DELAY_MS
}

fn default_model_retry_max_delay_ms() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_MODEL_RETRY_MAX_DELAY_MS
}

fn default_model_retry_backoff_factor() -> f64 {
    crate::native::settings::DEFAULT_NATIVE_MODEL_RETRY_BACKOFF_FACTOR
}

fn default_bash_default_timeout_secs() -> i32 {
    crate::native::settings::DEFAULT_NATIVE_BASH_DEFAULT_TIMEOUT_SECS
}

fn default_hook_handler_type() -> String {
    "command".to_string()
}

fn default_hook_source() -> String {
    "global".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeHook {
    pub id: String,
    /// `session_start` / `user_prompt_submit` / `pre_tool_use` / `post_tool_use` /
    /// `post_tool_use_failure` / `permission_request` / `stop`。
    pub event: String,
    /// 工具名列表或 `*`；非工具事件忽略。
    pub matcher: String,
    /// `command` 处理器的 shell 命令；`http` / `agent` 处理器可留空。
    #[serde(default)]
    pub command: String,
    pub timeout_secs: i32,
    pub enabled: bool,
    /// `command` | `http` | `agent`。
    #[serde(default = "default_hook_handler_type")]
    pub handler_type: String,
    /// `http` 处理器的 POST 地址。
    #[serde(default)]
    pub url: Option<String>,
    /// `agent` 处理器的判定提示词。
    #[serde(default)]
    pub agent_prompt: Option<String>,
    /// `global` | `workspace` | `plugin`：来源，用于展示与去重。
    #[serde(default = "default_hook_source")]
    pub source: String,
}

impl NativeHook {
    /// 构造一条 `command` 处理器的钩子（测试与旧调用方用）。
    pub fn shell(
        id: impl Into<String>,
        event: impl Into<String>,
        matcher: impl Into<String>,
        command: impl Into<String>,
        timeout_secs: i32,
        enabled: bool,
    ) -> Self {
        Self {
            id: id.into(),
            event: event.into(),
            matcher: matcher.into(),
            command: command.into(),
            timeout_secs,
            enabled,
            handler_type: default_hook_handler_type(),
            url: None,
            agent_prompt: None,
            source: default_hook_source(),
        }
    }
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
    #[serde(default)]
    pub artifact_retention_days: Option<i32>,
    #[serde(default)]
    pub model_retry_max_retries: Option<i32>,
    #[serde(default)]
    pub model_retry_base_delay_ms: Option<i32>,
    #[serde(default)]
    pub model_retry_max_delay_ms: Option<i32>,
    #[serde(default)]
    pub model_retry_backoff_factor: Option<f64>,
    #[serde(default)]
    pub bash_default_timeout_secs: Option<i32>,
    #[serde(default)]
    pub shell_snapshot_enabled: Option<bool>,
    #[serde(default)]
    pub rg_sidecar_enabled: Option<bool>,
    #[serde(default)]
    pub auto_compact_threshold_percent: Option<i32>,
    #[serde(default)]
    pub microcompact_enabled: Option<bool>,
    #[serde(default)]
    pub memory_enabled: Option<bool>,
    #[serde(default)]
    pub memory_dream_interval: Option<i32>,
    pub hooks: Option<Vec<NativeHook>>,
    pub global_prompt_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
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
    /// `stdio`（默认）| `http`（Streamable HTTP）| `sse`（旧版 HTTP+SSE）。
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// http / sse 传输的地址。
    #[serde(default)]
    pub url: Option<String>,
    /// http / sse 传输附加的请求头。
    #[serde(default)]
    pub headers: Vec<McpEnvVar>,
    /// OAuth 2.1 授权码 + PKCE 配置；有则请求带 `Authorization: Bearer`。
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
}

pub const MCP_TRANSPORT_STDIO: &str = "stdio";
pub const MCP_TRANSPORT_HTTP: &str = "http";
pub const MCP_TRANSPORT_SSE: &str = "sse";

fn default_mcp_transport() -> String {
    MCP_TRANSPORT_STDIO.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthConfig {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
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
