import type {
  CreateSshConfigInput,
  CreateWorkspaceInput,
  SshConfig,
  SshConnectionTestResult,
} from "@/lib/types";

interface RemoteConnectionInput {
  name: string;
  remotePath: string;
  sshConfigId?: string;
  newConfig?: CreateSshConfigInput;
}

export interface RemoteConnectionApi {
  createSshConfig: (input: CreateSshConfigInput) => Promise<SshConfig>;
  testSshConnection: (id: string) => Promise<SshConnectionTestResult>;
  createWorkspace: (input: CreateWorkspaceInput) => Promise<unknown>;
  onConfigCreated: (config: SshConfig) => void;
  isCurrent?: () => boolean;
}

export class RemoteConnectionValidationError extends Error {
  constructor(
    readonly field: "remotePath" | "config" | "requiredFields" | "port" | "privateKey" | "password",
  ) {
    super(field);
  }
}

export async function submitRemoteConnection(
  input: RemoteConnectionInput,
  api: RemoteConnectionApi,
): Promise<boolean> {
  const remotePath = input.remotePath.trim();
  const newConfig = input.newConfig;
  const name = input.name.trim() || newConfig?.name.trim() || remotePath;
  let configId = input.sshConfigId?.trim();
  const isCurrent = () => api.isCurrent?.() !== false;

  if (!remotePath) throw new RemoteConnectionValidationError("remotePath");
  if (!configId && !newConfig) throw new RemoteConnectionValidationError("config");
  if (!configId && newConfig) {
    if (!newConfig.name.trim() || !newConfig.host.trim() || !newConfig.username.trim()) {
      throw new RemoteConnectionValidationError("requiredFields");
    }
    const port = newConfig.port ?? 22;
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      throw new RemoteConnectionValidationError("port");
    }
    if (newConfig.auth_type === "key" && !newConfig.private_key_path?.trim()) {
      throw new RemoteConnectionValidationError("privateKey");
    }
    if (newConfig.auth_type === "password" && !newConfig.password) {
      throw new RemoteConnectionValidationError("password");
    }
  }
  if (!isCurrent()) return false;

  if (!configId && newConfig) {
    const created = await api.createSshConfig({
      ...newConfig,
      name: newConfig.name.trim(),
      host: newConfig.host.trim(),
      username: newConfig.username.trim(),
      port: newConfig.port ?? 22,
      private_key_path: newConfig.auth_type === "key" ? newConfig.private_key_path?.trim() : null,
      password: newConfig.auth_type === "password" ? newConfig.password : null,
    });
    configId = created.id;
    // 测试可能失败，先保留已保存的配置，后续重试不能重复创建。
    api.onConfigCreated(created);
  }
  if (!configId) throw new RemoteConnectionValidationError("config");
  if (!isCurrent()) return false;
  const result = await api.testSshConnection(configId);
  if (!result.ok) throw new Error(result.message);
  if (!isCurrent()) return false;
  await api.createWorkspace({
    name,
    workspace_type: "ssh",
    ssh_config_id: configId,
    remote_repo_path: remotePath,
  });
  return isCurrent();
}
