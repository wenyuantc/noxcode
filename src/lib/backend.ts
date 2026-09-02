import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CreateSshConfigInput,
  SshConfig,
  SshConfigFileHost,
  SshConfigFileImport,
  SshConnectionTestResult,
  SshHostKeyChanged,
  SshHostTrustPrompt,
  SshPasswordProbeResult,
  UpdateSshConfigInput,
} from "./types";

export function listSshConfigs(): Promise<SshConfig[]> {
  return invoke("list_ssh_configs");
}

export function getSshConfig(id: string): Promise<SshConfig> {
  return invoke("get_ssh_config", { id });
}

export function createSshConfig(payload: CreateSshConfigInput): Promise<SshConfig> {
  return invoke("create_ssh_config", { payload });
}

export function updateSshConfig(id: string, updates: UpdateSshConfigInput): Promise<SshConfig> {
  return invoke("update_ssh_config", { id, updates });
}

export function deleteSshConfig(id: string): Promise<void> {
  return invoke("delete_ssh_config", { id });
}

export function probeSshPasswordAuth(sshConfigId: string): Promise<SshPasswordProbeResult> {
  return invoke("probe_ssh_password_auth", { sshConfigId });
}

export function testSshConnection(sshConfigId: string): Promise<SshConnectionTestResult> {
  return invoke("test_ssh_connection", { sshConfigId });
}

export function listSshConfigFileHosts(): Promise<SshConfigFileHost[]> {
  return invoke("list_ssh_config_file_hosts");
}

export function importSshConfigFileHost(alias: string): Promise<SshConfigFileImport> {
  return invoke("import_ssh_config_file_host", { alias });
}

export function resolveSshHostTrust(promptId: string, accept: boolean): Promise<void> {
  return invoke("resolve_ssh_host_trust", { promptId, accept });
}

export function onSshHostTrustRequest(
  callback: (prompt: SshHostTrustPrompt) => void,
): Promise<UnlistenFn> {
  return listen<SshHostTrustPrompt>("ssh-host-trust-request", (event) => {
    callback(event.payload);
  });
}

export function onSshHostKeyChanged(
  callback: (info: SshHostKeyChanged) => void,
): Promise<UnlistenFn> {
  return listen<SshHostKeyChanged>("ssh-host-key-changed", (event) => {
    callback(event.payload);
  });
}
