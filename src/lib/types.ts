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
