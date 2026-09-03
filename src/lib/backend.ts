import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AgentSession,
  AppHealthCheck,
  DatabaseBackupResult,
  DatabaseRestoreResult,
  AgentSessionEvent,
  AgentSessionExit,
  AgentSessionOutput,
  AgentSessionResumeInfo,
  AgentSessionStarted,
  AiChannel,
  CreateAiChannelInput,
  CreateNativeSubagentInput,
  CreateSshConfigInput,
  CreateWorkspaceInput,
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
  ListNativeApiCallLogsInput,
  McpServersDocument,
  ModelCatalogEntry,
  NativeApiCallLogDetail,
  NativeApiCallLogPage,
  NativeContextUsage,
  NativeGlobalSkills,
  NativePermissionDecision,
  NativePermissionRequest,
  NativePlanQuestionRequest,
  NativeSettings,
  NativeSubagent,
  NativeTextDelta,
  NativeTurnState,
  NetworkSettings,
  QuickPrompt,
  SshConfig,
  SshConfigFileHost,
  SshConfigFileImport,
  SshConnectionTestResult,
  SshHostKeyChanged,
  SshHostTrustPrompt,
  SshPasswordProbeResult,
  StartNativeSessionInput,
  TestAiChannelInput,
  TestAiChannelResult,
  UpdateAiChannelInput,
  UpdateNativeSettingsInput,
  UpdateNativeSubagentInput,
  UpdateSshConfigInput,
  UpdateWorkspaceInput,
  Workspace,
  WorkspaceHealth,
} from "./types";

export function healthCheck(): Promise<AppHealthCheck> {
  return invoke("health_check");
}

export function backupDatabase(destinationPath: string): Promise<DatabaseBackupResult> {
  return invoke("backup_database", { destinationPath });
}

export function restoreDatabase(sourcePath: string): Promise<DatabaseRestoreResult> {
  return invoke("restore_database", { sourcePath });
}

export function openDatabaseFolder(): Promise<void> {
  return invoke("open_database_folder");
}

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

export function checkoutGitBranch(workspaceId: string, name: string): Promise<GitBranch> {
  return invoke("checkout_git_branch", { workspaceId, name });
}

export function listGitFiles(
  workspaceId: string,
  query?: string,
  limit?: number,
): Promise<string[]> {
  return invoke("list_git_files", { workspaceId, query, limit });
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

export function getQuickPrompts(): Promise<QuickPrompt[]> {
  return invoke("get_quick_prompts");
}

export function updateQuickPrompts(payload: QuickPrompt[]): Promise<QuickPrompt[]> {
  return invoke("update_quick_prompts", { payload });
}

export function listWorkspaces(): Promise<Workspace[]> {
  return invoke("list_workspaces");
}

export function createWorkspace(payload: CreateWorkspaceInput): Promise<Workspace> {
  return invoke("create_workspace", { payload });
}

export function updateWorkspace(id: string, updates: UpdateWorkspaceInput): Promise<Workspace> {
  return invoke("update_workspace", { id, updates });
}

export function deleteWorkspace(id: string): Promise<void> {
  return invoke("delete_workspace", { id });
}

export function checkWorkspaceHealth(workspaceId: string): Promise<WorkspaceHealth> {
  return invoke("check_workspace_health", { workspaceId });
}

export function ensureScratchWorkspace(): Promise<Workspace> {
  return invoke("ensure_scratch_workspace");
}

export function listAgentSessions(workspaceId?: string, limit?: number): Promise<AgentSession[]> {
  return invoke("list_agent_sessions", { workspaceId, limit });
}

export function getAgentSessionLogLines(
  sessionId: string,
  afterEventId?: string,
  limit?: number,
): Promise<AgentSessionEvent[]> {
  return invoke("get_agent_session_log_lines", { sessionId, afterEventId, limit });
}

export function prepareAgentSessionResume(sessionId: string): Promise<AgentSessionResumeInfo> {
  return invoke("prepare_agent_session_resume", { sessionId });
}

export function deleteAgentSession(sessionId: string): Promise<void> {
  return invoke("delete_agent_session", { sessionId });
}

export function startNativeSession(payload: StartNativeSessionInput): Promise<AgentSessionStarted> {
  return invoke("start_native_session", { payload });
}

export function stopNativeSession(sessionRecordId: string): Promise<void> {
  return invoke("stop_native_session", { sessionRecordId });
}

export function stopNative(profileId: string): Promise<void> {
  return invoke("stop_native", { profileId });
}

export function restartNativeSession(
  payload: StartNativeSessionInput,
): Promise<AgentSessionStarted> {
  return invoke("restart_native_session", { payload });
}

export function resumeNativeSession(
  payload: StartNativeSessionInput,
  resumeSessionId?: string,
): Promise<AgentSessionStarted> {
  return invoke("resume_native_session", { payload, resumeSessionId });
}

export function sendNativeInput(sessionRecordId: string, input: string): Promise<void> {
  return invoke("send_native_input", { sessionRecordId, input });
}

export function finishNativeInput(sessionRecordId: string): Promise<void> {
  return invoke("finish_native_input", { sessionRecordId });
}

export function resolveNativeToolPermission(
  sessionRecordId: string,
  requestId: string,
  decision: NativePermissionDecision,
): Promise<void> {
  return invoke("resolve_native_tool_permission", { sessionRecordId, requestId, decision });
}

export function answerNativePlanQuestion(
  sessionRecordId: string,
  requestId: string,
  skipped: boolean,
  answers: string[],
): Promise<void> {
  return invoke("answer_native_plan_question", { sessionRecordId, requestId, skipped, answers });
}

export function getNativeSettings(): Promise<NativeSettings> {
  return invoke("get_native_settings");
}

export function updateNativeSettings(updates: UpdateNativeSettingsInput): Promise<NativeSettings> {
  return invoke("update_native_settings", { updates });
}

export function listNativeGlobalSkills(): Promise<NativeGlobalSkills> {
  return invoke("list_native_global_skills");
}

export function openNativeSkillsDir(): Promise<void> {
  return invoke("open_native_skills_dir");
}

export const NATIVE_SUBAGENT_CUSTOM_TOOLS = [
  "Read",
  "Grep",
  "Glob",
  "Bash",
  "Edit",
  "Write",
  "WebFetch",
  "WebSearch",
  "TodoWrite",
  "ApplyPatch",
  "Skill",
] as const;

export type NativeSubagentCustomTool = (typeof NATIVE_SUBAGENT_CUSTOM_TOOLS)[number];

export function listNativeSubagents(): Promise<NativeSubagent[]> {
  return invoke("list_native_subagents");
}

export function createNativeSubagent(payload: CreateNativeSubagentInput): Promise<NativeSubagent> {
  return invoke("create_native_subagent", { payload });
}

export function updateNativeSubagent(
  id: string,
  payload: UpdateNativeSubagentInput,
): Promise<NativeSubagent> {
  return invoke("update_native_subagent", { id, payload });
}

export function deleteNativeSubagent(id: string): Promise<void> {
  return invoke("delete_native_subagent", { id });
}

export function listNativeApiCallLogs(
  payload?: ListNativeApiCallLogsInput,
): Promise<NativeApiCallLogPage> {
  return invoke("list_native_api_call_logs", { payload });
}

export function getNativeApiCallLog(id: string): Promise<NativeApiCallLogDetail> {
  return invoke("get_native_api_call_log", { id });
}

export function getMcpServers(): Promise<McpServersDocument> {
  return invoke("get_mcp_servers");
}

export function updateMcpServers(payload: McpServersDocument): Promise<McpServersDocument> {
  return invoke("update_mcp_servers", { payload });
}

export function resetMcpServers(): Promise<McpServersDocument> {
  return invoke("reset_mcp_servers");
}

export function exportMcpServersSnippet(): Promise<string> {
  return invoke("export_mcp_servers_snippet");
}

export function onNativeStdout(
  callback: (output: AgentSessionOutput) => void,
): Promise<UnlistenFn> {
  return listen<AgentSessionOutput>("native-stdout", (event) => {
    callback(event.payload);
  });
}

export function onNativeExit(callback: (exit: AgentSessionExit) => void): Promise<UnlistenFn> {
  return listen<AgentSessionExit>("native-exit", (event) => {
    callback(event.payload);
  });
}

export function onNativeSession(
  callback: (session: AgentSessionStarted) => void,
): Promise<UnlistenFn> {
  return listen<AgentSessionStarted>("native-session", (event) => {
    callback(event.payload);
  });
}

export function onNativeTextDelta(callback: (delta: NativeTextDelta) => void): Promise<UnlistenFn> {
  return listen<NativeTextDelta>("native-text-delta", (event) => {
    callback(event.payload);
  });
}

export function onNativePermissionRequest(
  callback: (request: NativePermissionRequest) => void,
): Promise<UnlistenFn> {
  return listen<NativePermissionRequest>("native-permission-request", (event) => {
    callback(event.payload);
  });
}

export function onNativePlanQuestion(
  callback: (request: NativePlanQuestionRequest) => void,
): Promise<UnlistenFn> {
  return listen<NativePlanQuestionRequest>("native-plan-question", (event) => {
    callback(event.payload);
  });
}

export function onNativeContextUsage(
  callback: (usage: NativeContextUsage) => void,
): Promise<UnlistenFn> {
  return listen<NativeContextUsage>("native-context-usage", (event) => {
    callback(event.payload);
  });
}

export function onNativeTurnState(callback: (state: NativeTurnState) => void): Promise<UnlistenFn> {
  return listen<NativeTurnState>("native-turn-state", (event) => {
    callback(event.payload);
  });
}
