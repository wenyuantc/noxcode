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
