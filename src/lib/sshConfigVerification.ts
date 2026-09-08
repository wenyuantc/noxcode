import type {
  CreateSshConfigInput,
  SshAlgorithms,
  SshAuthType,
  SshConfig,
  SshConnectionTestResult,
  SshKnownHostsMode,
  SshPasswordProbeResult,
  UpdateSshConfigInput,
} from "@/lib/types";

export interface SshConfigFormValues {
  name: string;
  host: string;
  port: string;
  username: string;
  authType: SshAuthType;
  privateKeyPath: string;
  password: string;
  passphrase: string;
  knownHostsMode: SshKnownHostsMode;
  algorithms: SshAlgorithms;
}

export type SshConfigFormError =
  "requiredFields" | "privateKeyRequired" | "passwordRequired" | "passwordAuthRequired";

export class SshConfigFormValidationError extends Error {
  constructor(readonly field: SshConfigFormError) {
    super(field);
  }
}

export interface PersistSshConfigInput {
  selectedId: string | null;
  form: SshConfigFormValues;
  passwordConfigured?: boolean;
}

export interface SshConfigVerificationApi {
  createSshConfig: (input: CreateSshConfigInput) => Promise<SshConfig>;
  updateSshConfig: (id: string, updates: UpdateSshConfigInput) => Promise<SshConfig>;
  testSshConnection: (id: string) => Promise<SshConnectionTestResult>;
  probeSshPasswordAuth: (id: string) => Promise<SshPasswordProbeResult>;
}

export function validateSshConfigForm(
  form: SshConfigFormValues,
  options?: { requirePassword?: boolean; passwordConfigured?: boolean },
): SshConfigFormError | null {
  if (!form.name.trim() || !form.host.trim() || !form.username.trim()) {
    return "requiredFields";
  }
  if (form.authType === "key" && !form.privateKeyPath.trim()) {
    return "privateKeyRequired";
  }
  if (options?.requirePassword) {
    if (form.authType !== "password") {
      return "passwordAuthRequired";
    }
    if (!form.password && !options.passwordConfigured) {
      return "passwordRequired";
    }
  }
  return null;
}

function algorithmsOrNull(algorithms: SshAlgorithms): SshAlgorithms | null {
  return Object.values(algorithms).some((names) => names.length > 0) ? algorithms : null;
}

export function buildSshConfigWritePayload(form: SshConfigFormValues): {
  create: CreateSshConfigInput;
  update: UpdateSshConfigInput;
} {
  const privateKeyPath = form.authType === "key" ? form.privateKeyPath.trim() || null : null;
  const algorithms = algorithmsOrNull(form.algorithms);
  const base = {
    name: form.name.trim(),
    host: form.host.trim(),
    port: Number(form.port) || 22,
    username: form.username.trim(),
    auth_type: form.authType,
    private_key_path: privateKeyPath,
    known_hosts_mode: form.knownHostsMode,
    algorithms,
  };
  const update: UpdateSshConfigInput = { ...base };
  if (form.authType === "password" && form.password) {
    update.password = form.password;
  }
  if (form.passphrase) {
    update.passphrase = form.passphrase;
  }
  return {
    create: {
      ...base,
      password: form.authType === "password" && form.password ? form.password : null,
      passphrase: form.passphrase || null,
    },
    update,
  };
}

function assertValidForm(
  form: SshConfigFormValues,
  options?: { requirePassword?: boolean; passwordConfigured?: boolean },
): void {
  const error = validateSshConfigForm(form, options);
  if (error) {
    throw new SshConfigFormValidationError(error);
  }
}

export async function persistSshConfigForm(
  input: PersistSshConfigInput,
  api: Pick<SshConfigVerificationApi, "createSshConfig" | "updateSshConfig">,
): Promise<SshConfig> {
  assertValidForm(input.form);
  const payload = buildSshConfigWritePayload(input.form);
  if (input.selectedId) {
    return api.updateSshConfig(input.selectedId, payload.update);
  }
  return api.createSshConfig(payload.create);
}

export async function persistAndTestSshConfig(
  input: PersistSshConfigInput,
  api: SshConfigVerificationApi,
  onPersisted?: (config: SshConfig) => void,
): Promise<{ config: SshConfig; result: SshConnectionTestResult }> {
  const config = await persistSshConfigForm(input, api);
  onPersisted?.(config);
  const result = await api.testSshConnection(config.id);
  return { config, result };
}

export async function persistAndProbeSshPasswordAuth(
  input: PersistSshConfigInput,
  api: SshConfigVerificationApi,
  onPersisted?: (config: SshConfig) => void,
): Promise<{ config: SshConfig; result: SshPasswordProbeResult }> {
  assertValidForm(input.form, {
    requirePassword: true,
    passwordConfigured: input.passwordConfigured,
  });
  const config = await persistSshConfigForm(input, api);
  onPersisted?.(config);
  const result = await api.probeSshPasswordAuth(config.id);
  return { config, result };
}
