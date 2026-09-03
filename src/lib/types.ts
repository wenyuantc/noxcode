export type SshAuthType = "key" | "password";

export type SshKnownHostsMode = "accept-new" | "strict" | "ask" | "off";

export type SshPasswordProbeStatus = "passed" | "failed" | "available";

export interface SshConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: SshAuthType;
  private_key_path: string | null;
  known_hosts_mode: SshKnownHostsMode;
  last_checked_at: string | null;
  last_check_status: string | null;
  last_check_message: string | null;
  password_probe_checked_at: string | null;
  password_probe_status: SshPasswordProbeStatus | null;
  password_probe_message: string | null;
  password_configured: boolean;
  passphrase_configured: boolean;
  password_execution_allowed: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateSshConfigInput {
  name: string;
  host: string;
  port?: number;
  username: string;
  auth_type: SshAuthType;
  private_key_path?: string | null;
  password?: string | null;
  passphrase?: string | null;
  known_hosts_mode?: SshKnownHostsMode;
}

export interface UpdateSshConfigInput {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  auth_type?: SshAuthType;
  private_key_path?: string | null;
  password?: string | null;
  passphrase?: string | null;
  known_hosts_mode?: SshKnownHostsMode;
}

export interface SshPasswordProbeResult {
  ssh_config_id: string;
  target_host_label: string;
  supported: boolean;
  status: string;
  message: string;
  checked_at: string;
}

export interface SshConnectionTestResult {
  ssh_config_id: string;
  target_host_label: string;
  ok: boolean;
  status: string;
  message: string;
  uname: string | null;
  remote_git_version: string | null;
  checked_at: string;
}

export interface SshConfigFileHost {
  alias: string;
  host: string;
  port: number;
  username: string | null;
  has_proxy_jump: boolean;
}

export interface SshConfigFileImport {
  alias: string;
  host: string;
  port: number;
  username: string;
  private_key_path: string | null;
  proxy_jump: string | null;
  proxy_jump_unsupported: boolean;
  warnings: string[];
}

export interface SshHostTrustPrompt {
  prompt_id: string;
  ssh_config_id: string;
  name: string;
  host: string;
  port: number;
  key_type: string;
  fingerprint_sha256: string;
  known_hosts_path: string;
}

export interface SshHostKeyChanged {
  ssh_config_id: string;
  name: string;
  host: string;
  port: number;
  key_type: string;
  fingerprint_sha256: string;
  known_hosts_path: string;
  line: number;
}

export interface GitRepoInfo {
  workspace_id: string;
  toplevel: string;
  prefix: string;
  git_dir: string;
  common_dir: string;
  head: string | null;
  branch: string | null;
  upstream: string | null;
  git_version: string;
}

export interface GitBranchInfo {
  oid: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
}

export interface GitStatusEntry {
  kind: "ordinary" | "rename" | "unmerged" | "untracked" | "ignored" | string;
  xy: string;
  path: string;
  orig_path: string | null;
  score: string | null;
  mode_head: string | null;
  mode_index: string | null;
  mode_worktree: string | null;
}

export interface GitStatus {
  branch: GitBranchInfo;
  entries: GitStatusEntry[];
}

export type GitFileDiffScope =
  "worktree" | "staged" | { range: { from_oid: string; to_oid: string } };

export type GitNumstatScope = "worktree" | "staged" | "upstream";

export interface GitFileDiff {
  path: string;
  old_path: string | null;
  patch: string;
  is_binary: boolean;
  truncated: boolean;
}

export interface GitNumstatEntry {
  path: string;
  orig_path: string | null;
  added: number | null;
  deleted: number | null;
  is_binary: boolean;
}

export interface GitBranch {
  name: string;
  oid: string;
  upstream: string | null;
  is_current: boolean;
}

export type GitCheckpointKind = "session_start" | "after_tool_call" | "manual" | "auto_pre_restore";

export interface GitCheckpoint {
  id: string;
  session_id: string;
  workspace_id: string;
  seq: number;
  ref_name: string;
  commit_oid: string;
  parent_oid: string | null;
  label: string | null;
  kind: GitCheckpointKind | string;
  created_at: string;
  ref_valid: boolean;
}

export interface GitRestorePreview {
  checkpoint_id: string;
  blocked_reason: string | null;
  warnings: string[];
  will_overwrite: string[];
  will_recreate: string[];
  wont_be_touched: string[];
}

export interface GitRestoreResult {
  pre_restore_checkpoint: GitCheckpoint;
  restored: string[];
  deleted: string[];
  skipped_ignored: string[];
  failed: string[];
}

export interface ActivityLog {
  id: string;
  kind: string;
  workspace_id: string | null;
  session_id: string | null;
  summary: string;
  payload_json: string | null;
  created_at: string;
}

export interface GitCommitResult {
  oid: string;
  message: string;
}

export interface GitPushResult {
  remote: string | null;
  branch: string | null;
  set_upstream: boolean;
  message: string;
}

export type AiChannelProtocol = "openai" | "anthropic" | "codex";

export interface AiChannelModel {
  id: string;
  context_tokens: number | null;
  max_output_tokens: number | null;
  thinking_enabled: boolean | null;
  thinking_level: string | null;
  thinking_levels: string[] | null;
}

export interface ModelCatalogEntry {
  id: string;
  aliases: string[];
  vendor: string;
  label: string;
  context_tokens: number;
  max_output_tokens: number;
  thinking: boolean;
  thinking_levels: string[];
}

export interface AiChannel {
  id: string;
  name: string;
  protocol: AiChannelProtocol;
  base_url: string;
  extra_headers_json: string | null;
  models: AiChannelModel[];
  enabled: boolean;
  api_key: string | null;
  api_key_configured: boolean;
  created_at: string;
  updated_at: string;
}

export interface CreateAiChannelInput {
  name: string;
  protocol: AiChannelProtocol;
  base_url: string;
  api_key?: string | null;
  extra_headers_json?: string | null;
  models?: AiChannelModel[];
  enabled?: boolean;
}

export interface UpdateAiChannelInput {
  name?: string;
  protocol?: AiChannelProtocol;
  base_url?: string;
  api_key?: string | null;
  extra_headers_json?: string | null;
  models?: AiChannelModel[];
  enabled?: boolean;
}

export interface TestAiChannelInput {
  id?: string | null;
  protocol?: AiChannelProtocol;
  base_url?: string;
  api_key?: string | null;
  extra_headers_json?: string | null;
  model?: string | null;
}

export interface TestAiChannelResult {
  ok: boolean;
  status: number | null;
  message: string;
}

export interface ListAiChannelModelsResult {
  models: string[];
  message: string;
  truncated?: boolean;
}

export interface NetworkSettings {
  http_proxy: string | null;
  no_proxy: string | null;
  ca_cert_path: string | null;
}

export const NATIVE_THINKING_LEVELS = [
  "none",
  "no_think",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

export type ReasoningEffort = "low" | "medium" | "high" | "xhigh" | "max";

export type WorkspaceType = "local" | "ssh";

export interface Workspace {
  id: string;
  name: string;
  workspace_type: WorkspaceType;
  repo_path: string | null;
  ssh_config_id: string | null;
  remote_repo_path: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateWorkspaceInput {
  name: string;
  workspace_type: WorkspaceType;
  repo_path?: string | null;
  ssh_config_id?: string | null;
  remote_repo_path?: string | null;
}

export interface UpdateWorkspaceInput {
  name?: string;
  workspace_type?: WorkspaceType;
  repo_path?: string | null;
  ssh_config_id?: string | null;
  remote_repo_path?: string | null;
}

export interface WorkspaceHealth {
  workspace_id: string;
  ok: boolean;
  message: string;
  git_version: string | null;
  toplevel: string | null;
}

export interface AgentSession {
  id: string;
  ai_channel_id: string | null;
  workspace_id: string | null;
  working_dir: string | null;
  execution_target: string;
  ssh_config_id: string | null;
  target_host_label: string | null;
  session_kind: string;
  status: string;
  started_at: string;
  ended_at: string | null;
  exit_code: number | null;
  resume_session_id: string | null;
  title?: string | null;
  pinned: number;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  reasoning_tokens: number | null;
  cached_tokens: number | null;
  created_at: string;
  context_usage_json?: string | null;
}

export interface AgentSessionEvent {
  id: string;
  session_id: string;
  event_type: string;
  message: string | null;
  created_at: string;
}

export interface AgentSessionResumeInfo {
  session_id: string;
  resumable: boolean;
  model: string | null;
  turns: number | null;
  message: string;
}

export interface StartNativeSessionInput {
  ai_channel_id: string;
  workspace_id: string;
  prompt: string;
  model?: string | null;
  reasoning_effort?: string | null;
  system_prompt?: string | null;
  resume_session_id?: string | null;
  image_paths?: string[] | null;
  plan_mode?: boolean | null;
}

export interface AgentSessionStarted {
  profile_id: string;
  workspace_id: string;
  session_kind: string;
  session_record_id: string;
}

export interface AgentSessionOutput {
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  session_record_id: string;
  session_event_id: string;
  line: string;
}

export interface AgentSessionExit {
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  session_record_id: string;
  code: number;
}

export interface NativeTextDelta {
  session_record_id: string;
  kind: string;
  text: string;
  clear: boolean;
}

export type NativeToolRiskKind = "overwrite" | "delete" | "push" | "force_git" | "mcp" | "opaque";

export type NativePermissionDecision = "allow_session" | "allow_once" | "allow_server" | "deny";

export interface NativePermissionRequest {
  session_record_id: string;
  request_id: string;
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  tool_name: string;
  kind: NativeToolRiskKind;
  summary: string;
  remote: boolean;
  mcp_server_id: string | null;
}

export interface NativePlanQuestion {
  prompt: string;
  options: string[];
}

export interface NativePlanQuestionRequest {
  session_record_id: string;
  request_id: string;
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  questions: NativePlanQuestion[];
}

export interface NativeContextUsage {
  session_record_id: string;
  used_tokens: number;
  limit_tokens: number;
  generation: number;
  compactions: number;
  mcp_tokens?: number;
  system_tool_tokens?: number;
  skill_tokens?: number;
  system_prompt_tokens?: number;
  other_tokens?: number;
  message_tokens?: number;
  prompt_tokens?: number;
  cached_tokens?: number;
}

export type NativeTurnStateKind = "waiting_input" | "working";

export interface NativeTurnState {
  session_record_id: string;
  state: NativeTurnStateKind | string;
}

export interface QuickPrompt {
  id: string;
  label: string;
  prompt: string;
}

export interface AppHealthCheck {
  database_loaded: boolean;
  database_path: string | null;
  database_current_version: number | null;
  database_current_description: string | null;
  database_latest_version: number;
  git_available: boolean;
  git_version: string | null;
  checked_at: string;
}

export interface DatabaseBackupResult {
  source_path: string;
  destination_path: string;
  database_version: number | null;
  created_at: string;
  message: string;
}

export interface DatabaseRestoreResult {
  source_path: string;
  backup_path: string;
  database_version: number | null;
  restored_at: string;
  message: string;
}

export interface NativeHook {
  id: string;
  event: string;
  matcher: string;
  command: string;
  timeout_secs: number;
  enabled: boolean;
}

export type NativePermissionMode = "confirm" | "auto_edit" | "full";

export interface NativeSettings {
  max_turns: number;
  max_subagent_turns: number;
  permission_mode: NativePermissionMode;
  max_concurrent_subagents: number;
  subagent_policy: string;
  context_window_tokens: number;
  use_custom_context_window: boolean;
  rollout_token_budget: number;
  max_tool_output_tokens: number;
  permission_timeout_secs: number;
  subagent_budget_share_percent: number;
  auto_checkpoint_after_tool_call: boolean;
  checkpoint_retention_days: number;
  desktop_notifications: boolean;
  hooks: NativeHook[];
  global_prompt_template: string;
}

export interface UpdateNativeSettingsInput {
  max_turns?: number;
  max_subagent_turns?: number;
  permission_mode?: NativePermissionMode;
  max_concurrent_subagents?: number;
  subagent_policy?: string;
  context_window_tokens?: number;
  use_custom_context_window?: boolean;
  rollout_token_budget?: number;
  max_tool_output_tokens?: number;
  permission_timeout_secs?: number;
  subagent_budget_share_percent?: number;
  auto_checkpoint_after_tool_call?: boolean;
  checkpoint_retention_days?: number;
  desktop_notifications?: boolean;
  hooks?: NativeHook[];
  global_prompt_template?: string;
}

export type NativeSkillSource = "workspace_agents" | "workspace_claude" | "global";

export interface NativeSkill {
  name: string;
  description: string;
  source: NativeSkillSource;
  dir: string;
  skill_md_path: string;
  body: string;
  extra_files: string[];
}

export interface NativeGlobalSkills {
  dir: string;
  skills: NativeSkill[];
}

export interface NativeSubagent {
  id: string;
  name: string;
  description: string;
  model_mode: string;
  channel_id: string | null;
  model: string | null;
  tool_mode: string;
  tools: string[];
  system_prompt: string;
  inject_agents_md: boolean;
  scope: string;
  workspace_ids: string[];
}

export interface CreateNativeSubagentInput {
  name: string;
  description: string;
  model_mode?: string | null;
  channel_id?: string | null;
  model?: string | null;
  tool_mode?: string | null;
  tools?: string[] | null;
  system_prompt?: string | null;
  inject_agents_md?: boolean | null;
  scope?: string | null;
  workspace_ids?: string[] | null;
}

export interface UpdateNativeSubagentInput {
  name?: string;
  description?: string;
  model_mode?: string;
  channel_id?: string | null;
  model?: string | null;
  tool_mode?: string;
  tools?: string[];
  system_prompt?: string;
  inject_agents_md?: boolean;
  scope?: string;
  workspace_ids?: string[];
}

export interface McpEnvVar {
  key: string;
  value: string;
}

export interface McpServerConfig {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: McpEnvVar[];
  enabled: boolean;
  notes: string | null;
}

export interface McpServersDocument {
  servers: McpServerConfig[];
}

export interface NativeApiCallLogListItem {
  id: string;
  call_id: string;
  attempt: number;
  channel_id: string | null;
  channel_name: string | null;
  protocol: string;
  response_encoding: string | null;
  model: string | null;
  thinking_enabled: number;
  thinking_level: string | null;
  request_format: string;
  input_tokens: number | null;
  output_tokens: number | null;
  cached_tokens: number | null;
  total_tokens: number | null;
  first_token_ms: number | null;
  duration_ms: number | null;
  status: string;
  http_status: number | null;
  session_id: string | null;
  profile_id: string | null;
  workspace_id: string | null;
  profile_name: string | null;
  workspace_name: string | null;
  execution_target: string | null;
  call_kind: string | null;
  created_at: string;
}

export interface NativeApiCallLogDetail extends NativeApiCallLogListItem {
  request_body: string | null;
  request_truncated: number;
  response_body: string | null;
  response_truncated: number;
  error_message: string | null;
  subagent_id: string | null;
}

export interface ListNativeApiCallLogsInput {
  workspace_id?: string | null;
  profile_id?: string | null;
  execution_target?: string | null;
  session_id?: string | null;
  channel_name?: string | null;
  model?: string | null;
  status?: string | null;
  start_date?: string | null;
  end_date?: string | null;
  limit?: number | null;
  offset?: number | null;
  include_total?: boolean | null;
}

export interface NativeApiCallLogStats {
  total: number;
  success: number;
  failed: number;
  cancelled: number;
  input_tokens: number;
  output_tokens: number;
  cached_tokens_sum: number | null;
  total_tokens_sum: number | null;
  avg_first_token_ms: number | null;
  avg_duration_ms: number | null;
}

export interface NativeApiCallLogPage {
  items: NativeApiCallLogListItem[];
  total: number;
  stats: NativeApiCallLogStats;
}
