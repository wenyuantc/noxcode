export type SshAuthType = "key" | "password";

export type SshKnownHostsMode = "accept-new" | "strict" | "ask" | "off";

export type SshPasswordProbeStatus = "passed" | "failed" | "available";

export interface SshAlgorithms {
  kex: string[];
  host_key: string[];
  cipher: string[];
  mac: string[];
}

export interface SshSupportedAlgorithms {
  supported: SshAlgorithms;
  defaults: SshAlgorithms;
  legacy_preset: SshAlgorithms;
}

export interface SshConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: SshAuthType;
  private_key_path: string | null;
  known_hosts_mode: SshKnownHostsMode;
  algorithms: SshAlgorithms | null;
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
  algorithms?: SshAlgorithms | null;
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
  algorithms?: SshAlgorithms | null;
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

export const CHANNEL_INPUT_TYPES = ["text", "image", "video"] as const;
export type ChannelInputType = (typeof CHANNEL_INPUT_TYPES)[number];

export interface AiChannelModel {
  id: string;
  context_tokens: number | null;
  max_output_tokens: number | null;
  thinking_enabled: boolean | null;
  thinking_level: string | null;
  thinking_levels: string[] | null;
  /** 支持的输入类型，始终包含 "text"；null 表示未设置，采用目录默认。 */
  input_types: ChannelInputType[] | null;
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
  input_types: ChannelInputType[];
}

export interface AiChannel {
  id: string;
  name: string;
  protocol: AiChannelProtocol;
  base_url: string;
  extra_headers_json: string | null;
  models: AiChannelModel[];
  lite_model?: string | null;
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
  lite_model?: string | null;
  enabled?: boolean;
}

export interface UpdateAiChannelInput {
  name?: string;
  protocol?: AiChannelProtocol;
  base_url?: string;
  api_key?: string | null;
  extra_headers_json?: string | null;
  models?: AiChannelModel[];
  lite_model?: string | null;
  enabled?: boolean;
}

export type NativeMemoryKind = "user" | "feedback" | "project" | "reference";

export interface NativeMemoryEntry {
  file_name: string;
  name: string;
  description: string;
  kind: NativeMemoryKind | string;
  created_at: string;
  updated_at: string;
  body: string;
}

export interface NativeAutomation {
  id: string;
  workspace_id: string;
  name: string;
  prompt: string;
  cron: string;
  timezone: string | null;
  enabled: number;
  channel_id: string | null;
  model: string | null;
  last_run_at: string | null;
  next_run_at: string | null;
  last_session_id: string | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateNativeAutomationInput {
  workspace_id: string;
  name: string;
  prompt: string;
  cron: string;
  channel_id?: string | null;
  model?: string | null;
  enabled?: boolean | null;
}

export interface UpdateNativeAutomationInput {
  name?: string;
  prompt?: string;
  cron?: string;
  channel_id?: string | null;
  model?: string | null;
  enabled?: boolean;
}

export interface NativeGoalChecklistItem {
  item: string;
  done: boolean;
}

export interface NativeGoal {
  id: string;
  session_record_id: string;
  title: string;
  status: string;
  checklist: NativeGoalChecklistItem[];
  note: string | null;
  updated_at: string;
}

export interface NativeMemoryView {
  dir: string;
  index: string;
  entries: NativeMemoryEntry[];
  extractions: number;
  dreams: number;
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

export type NativeToolPhase = "start" | "result";

export interface NativeToolEvent {
  phase: NativeToolPhase;
  call_id: string;
  name: string;
  title: string;
  args_summary?: string;
  ok?: boolean | null;
  duration_ms?: number | null;
  result_preview?: string | null;
  subagent_tag?: string | null;
  mcp_server?: string | null;
  mcp_tool?: string | null;
  image_names?: string[];
}

export interface NativeToolImage {
  name: string;
  mime_type: string;
  data_url: string;
}

export interface AgentSessionOutput {
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  session_record_id: string;
  session_event_id: string;
  line: string;
  tool?: NativeToolEvent | null;
  images?: NativeToolImage[] | null;
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

export type NativeToolRiskKind =
  "overwrite" | "delete" | "push" | "force_git" | "mcp" | "opaque" | "rule";

export type NativePermissionDecision =
  "allow_session" | "allow_once" | "allow_server" | "allow_always" | "deny";

export type PermissionCapability =
  | "read"
  | "edit"
  | "bash"
  | "mcp"
  | "web_search"
  | "web_fetch"
  | "subagent"
  | "skill"
  | "todo_read"
  | "todo_write"
  | "ask_user"
  | "automation_read"
  | "automation_write"
  | "agent_message_send"
  | "agent_message_respond"
  | "session_context_read"
  | "goal_read"
  | "workflow";

export type PermissionPatternSource = "command" | "path" | "input" | "tool_name";

export type PermissionRuleScope = "workspace" | "global";

export type PermissionRuleEffect = "allow" | "deny" | "ask";

export interface PermissionRuleSuggestion {
  capability: PermissionCapability;
  pattern: string;
  source: PermissionPatternSource;
}

export interface PermissionRule {
  id: string;
  capability: PermissionCapability;
  pattern: string;
  source: PermissionPatternSource;
  scope: PermissionRuleScope;
  note: string;
}

export interface PermissionRules {
  allow: PermissionRule[];
  deny: PermissionRule[];
  ask: PermissionRule[];
}

export interface NativePermissionRulesView {
  global: PermissionRules;
  workspace: PermissionRules | null;
  workspace_root: string | null;
}

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
  suggested_rule?: PermissionRuleSuggestion | null;
}

export interface NativePlanApprovalRequest {
  session_record_id: string;
  request_id: string;
  profile_id: string;
  workspace_id: string | null;
  session_kind: string;
  plan: string;
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

export interface NativePlanModeChanged {
  session_record_id: string;
  plan_mode: boolean;
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

export type NativeHookEvent =
  | "session_start"
  | "user_prompt_submit"
  | "pre_tool_use"
  | "post_tool_use"
  | "post_tool_use_failure"
  | "permission_request"
  | "stop";

export const NATIVE_HOOK_EVENTS: NativeHookEvent[] = [
  "session_start",
  "user_prompt_submit",
  "pre_tool_use",
  "post_tool_use",
  "post_tool_use_failure",
  "permission_request",
  "stop",
];

export type NativeHookHandlerType = "command" | "http" | "agent";

export const NATIVE_HOOK_HANDLER_TYPES: NativeHookHandlerType[] = ["command", "http", "agent"];

export interface NativeHook {
  id: string;
  event: NativeHookEvent | string;
  matcher: string;
  command: string;
  timeout_secs: number;
  enabled: boolean;
  handler_type: NativeHookHandlerType | string;
  url?: string | null;
  agent_prompt?: string | null;
  source?: string;
}

export type NativePermissionMode = "default" | "edit" | "build" | "yolo";

export const NATIVE_PERMISSION_MODES: NativePermissionMode[] = ["default", "edit", "build", "yolo"];

export function isNativePermissionMode(value: unknown): value is NativePermissionMode {
  return value === "default" || value === "edit" || value === "build" || value === "yolo";
}

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
  artifact_retention_days: number;
  model_retry_max_retries: number;
  model_retry_base_delay_ms: number;
  model_retry_max_delay_ms: number;
  model_retry_backoff_factor: number;
  bash_default_timeout_secs: number;
  shell_snapshot_enabled: boolean;
  rg_sidecar_enabled: boolean;
  auto_compact_threshold_percent: number;
  microcompact_enabled: boolean;
  memory_enabled: boolean;
  memory_dream_interval: number;
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
  artifact_retention_days?: number;
  model_retry_max_retries?: number;
  model_retry_base_delay_ms?: number;
  model_retry_max_delay_ms?: number;
  model_retry_backoff_factor?: number;
  bash_default_timeout_secs?: number;
  shell_snapshot_enabled?: boolean;
  rg_sidecar_enabled?: boolean;
  auto_compact_threshold_percent?: number;
  microcompact_enabled?: boolean;
  memory_enabled?: boolean;
  memory_dream_interval?: number;
  hooks?: NativeHook[];
  global_prompt_template?: string;
}

export type NativeSkillSource =
  | "workspace_noxcode"
  | "workspace_zcode"
  | "workspace_agents"
  | "workspace_claude"
  | "plugin"
  | "global";

export interface NativeSkill {
  name: string;
  description: string;
  source: NativeSkillSource;
  dir: string;
  skill_md_path: string;
  body: string;
  extra_files: string[];
  allowed_tools: string[];
  argument_hint: string | null;
  when_to_use: string | null;
  plugin: string | null;
}

export interface SkillDiagnostic {
  code: string;
  path: string;
  message: string;
  level: string;
}

export interface NativeSkillsView {
  global_dir: string;
  workspace_root: string | null;
  skills: NativeSkill[];
  disabled_paths: string[];
  diagnostics: SkillDiagnostic[];
}

export interface CreateNativeSkillInput {
  scope: "global" | "project";
  name: string;
  description: string;
  workspace_id?: string | null;
}

export interface ExternalSkillItem {
  name: string;
  description: string;
  source_path: string;
  importable: boolean;
  skip_reason: string | null;
}

export interface ExternalSkillGroup {
  id: string;
  label: string;
  scope: string;
  skills: ExternalSkillItem[];
}

export interface ExternalSkillScan {
  groups: ExternalSkillGroup[];
}

export interface ImportExternalSkillsInput {
  workspace_id?: string | null;
  target: "global" | "project";
  mode: "copy" | "symlink";
  items: Array<{ source_path: string; name: string }>;
}

export interface ImportExternalSkillsResult {
  imported: string[];
  skipped: string[];
  failed: string[];
}

export type NativeSlashCommandSource =
  "workspace_noxcode" | "workspace_claude" | "plugin" | "global";

/** 自定义斜杠命令（Markdown 文件）。 */
export interface NativeSlashCommand {
  name: string;
  description: string;
  argument_hint: string | null;
  allowed_tools: string[];
  model: string | null;
  skills: string[];
  source: NativeSlashCommandSource;
  plugin: string | null;
  path: string;
  body: string;
}

export interface ExpandedSlashCommand {
  name: string;
  prompt: string;
  allowed_tools: string[];
  model: string | null;
  skills: string[];
}

export type NativePluginSource = "global" | "workspace";

export interface PluginUserConfigField {
  key: string;
  description: string;
  default: string | null;
  required: boolean;
}

export interface NativePlugin {
  name: string;
  version: string | null;
  description: string;
  source: NativePluginSource;
  root: string;
  manifest_path: string;
  enabled: boolean;
  skill_dirs: string[];
  command_dirs: string[];
  agent_dirs: string[];
  hooks: NativeHook[];
  mcp_servers: McpServerConfig[];
  user_config_fields: PluginUserConfigField[];
  user_config: Record<string, string>;
  errors: string[];
}

export interface NativePluginsView {
  dir: string;
  plugins: NativePlugin[];
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
  permission_mode?: string | null;
  disallowed_tools?: string[];
  /** `json`（设置页维护）或 `file`（`.md` 档案，只读）。 */
  source?: "json" | "file" | string;
  path?: string | null;
  max_turns?: number | null;
  skills?: string[];
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
  permission_mode?: string | null;
  disallowed_tools?: string[] | null;
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
  permission_mode?: string | null;
  disallowed_tools?: string[];
}

export interface McpEnvVar {
  key: string;
  value: string;
}

export type McpTransport = "stdio" | "http" | "sse";

export interface McpOAuthConfig {
  client_id: string;
  client_secret: string | null;
  authorize_url: string;
  token_url: string;
  scopes: string[];
}

export interface McpServerConfig {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: McpEnvVar[];
  enabled: boolean;
  notes: string | null;
  scope: "all" | "workspaces";
  workspace_ids: string[];
  transport: McpTransport;
  url: string | null;
  headers: McpEnvVar[];
  oauth: McpOAuthConfig | null;
}

export interface McpOAuthStart {
  serverId: string;
  authorizeUrl: string;
  redirectUri: string;
}

export interface McpOAuthStatus {
  serverId: string;
  authorized: boolean;
  expiresAt: number | null;
  hasRefreshToken: boolean;
}

export interface McpOAuthEvent {
  serverId: string;
  ok: boolean;
  message: string;
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
