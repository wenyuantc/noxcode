import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AiChannel,
  CreateAiChannelInput,
  CreateSshConfigInput,
  GitBranch,
  GitCheckpoint,
  GitCheckpointKind,
  GitCommitResult,
  GitFileDiff,
  GitFileDiffScope,
  GitNumstatEntry,
  GitNumstatScope,
  GitPushResult,
  GitRepoInfo,
  GitRestorePreview,
  GitRestoreResult,
  GitStatus,
  ListAiChannelModelsResult,
  ModelCatalogEntry,
  NetworkSettings,
  SshConfig,
  SshConfigFileHost,
  SshConfigFileImport,
  SshConnectionTestResult,
  SshHostKeyChanged,
  SshHostTrustPrompt,
  SshPasswordProbeResult,
  TestAiChannelInput,
  TestAiChannelResult,
  UpdateAiChannelInput,
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

export function getGitRepoInfo(workspaceId: string): Promise<GitRepoInfo> {
  return invoke("get_git_repo_info", { workspaceId });
}

export function getGitStatus(workspaceId: string, untrackedMode?: string): Promise<GitStatus> {
  return invoke("get_git_status", { workspaceId, untrackedMode });
}

export function getGitFileDiff(
  workspaceId: string,
  path: string,
  scope: GitFileDiffScope,
  oldPath?: string,
): Promise<GitFileDiff> {
  return invoke("get_git_file_diff", { workspaceId, path, scope, oldPath });
}

export function getGitNumstat(
  workspaceId: string,
  scope: GitNumstatScope,
): Promise<GitNumstatEntry[]> {
  return invoke("get_git_numstat", { workspaceId, scope });
}

export function stageGitPaths(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("stage_git_paths", { workspaceId, paths });
}

export function unstageGitPaths(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("unstage_git_paths", { workspaceId, paths });
}

export function restoreGitPaths(workspaceId: string, paths: string[]): Promise<void> {
  return invoke("restore_git_paths", { workspaceId, paths });
}

export function commitGitChanges(
  workspaceId: string,
  message: string,
  paths?: string[],
): Promise<GitCommitResult> {
  return invoke("commit_git_changes", { workspaceId, message, paths });
}

export function pushGitBranch(
  workspaceId: string,
  remote?: string,
  branch?: string,
  setUpstream = false,
): Promise<GitPushResult> {
  return invoke("push_git_branch", { workspaceId, remote, branch, setUpstream });
}

export function listGitBranches(workspaceId: string): Promise<GitBranch[]> {
  return invoke("list_git_branches", { workspaceId });
}

export function createGitBranch(
  workspaceId: string,
  name: string,
  checkout: boolean,
): Promise<GitBranch> {
  return invoke("create_git_branch", { workspaceId, name, checkout });
}

export function createGitCheckpoint(
  workspaceId: string,
  sessionId: string,
  label?: string,
  kind?: GitCheckpointKind,
): Promise<GitCheckpoint> {
  return invoke("create_git_checkpoint", { workspaceId, sessionId, label, kind });
}

export function listGitCheckpoints(
  workspaceId: string,
  sessionId: string,
): Promise<GitCheckpoint[]> {
  return invoke("list_git_checkpoints", { workspaceId, sessionId });
}

export function previewGitCheckpointRestore(
  workspaceId: string,
  checkpointId: string,
): Promise<GitRestorePreview> {
  return invoke("preview_git_checkpoint_restore", { workspaceId, checkpointId });
}

export function restoreGitCheckpoint(
  workspaceId: string,
  checkpointId: string,
  deleteNewPaths?: string[],
): Promise<GitRestoreResult> {
  return invoke("restore_git_checkpoint", { workspaceId, checkpointId, deleteNewPaths });
}

export function clearGitCheckpoints(workspaceId: string): Promise<number> {
  return invoke("clear_git_checkpoints", { workspaceId });
}

export function listAiChannels(): Promise<AiChannel[]> {
  return invoke("list_ai_channels");
}

export function createAiChannel(payload: CreateAiChannelInput): Promise<AiChannel> {
  return invoke("create_ai_channel", { payload });
}

export function updateAiChannel(id: string, updates: UpdateAiChannelInput): Promise<AiChannel> {
  return invoke("update_ai_channel", { id, updates });
}

export function deleteAiChannel(id: string): Promise<void> {
  return invoke("delete_ai_channel", { id });
}

export function testAiChannel(payload: TestAiChannelInput): Promise<TestAiChannelResult> {
  return invoke("test_ai_channel", { payload });
}

export function listAiChannelModels(
  payload: TestAiChannelInput,
): Promise<ListAiChannelModelsResult> {
  return invoke("list_ai_channel_models", { payload });
}

export function listModelCatalog(): Promise<ModelCatalogEntry[]> {
  return invoke("list_model_catalog");
}

export function getNetworkSettings(): Promise<NetworkSettings> {
  return invoke("get_network_settings");
}

export function updateNetworkSettings(payload: NetworkSettings): Promise<NetworkSettings> {
  return invoke("update_network_settings", { payload });
}
