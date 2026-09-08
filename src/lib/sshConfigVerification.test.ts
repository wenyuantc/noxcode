import { describe, expect, it, vi } from "vitest";

import {
  persistAndProbeSshPasswordAuth,
  persistAndTestSshConfig,
  persistSshConfigForm,
  SshConfigFormValidationError,
  validateSshConfigForm,
  type SshConfigFormValues,
  type SshConfigVerificationApi,
} from "./sshConfigVerification";
import type { SshConfig, SshConnectionTestResult, SshPasswordProbeResult } from "./types";

function emptyAlgorithms() {
  return { kex: [], host_key: [], cipher: [], mac: [] };
}

function keyForm(overrides: Partial<SshConfigFormValues> = {}): SshConfigFormValues {
  return {
    name: "生产主机",
    host: "10.0.0.12",
    port: "22",
    username: "deploy",
    authType: "key",
    privateKeyPath: "~/.ssh/id_ed25519",
    password: "",
    passphrase: "",
    knownHostsMode: "accept-new",
    algorithms: emptyAlgorithms(),
    ...overrides,
  };
}

function passwordForm(overrides: Partial<SshConfigFormValues> = {}): SshConfigFormValues {
  return keyForm({
    authType: "password",
    privateKeyPath: "",
    password: "secret",
    ...overrides,
  });
}

const saved: SshConfig = {
  id: "ssh-new",
  name: "生产主机",
  host: "10.0.0.12",
  port: 22,
  username: "deploy",
  auth_type: "password",
  private_key_path: null,
  known_hosts_mode: "accept-new",
  algorithms: null,
  last_checked_at: null,
  last_check_status: null,
  last_check_message: null,
  password_probe_checked_at: null,
  password_probe_status: null,
  password_probe_message: null,
  password_configured: true,
  passphrase_configured: false,
  password_execution_allowed: false,
  created_at: "2026-09-08",
  updated_at: "2026-09-08",
};

const passedTest: SshConnectionTestResult = {
  ssh_config_id: saved.id,
  target_host_label: "deploy@10.0.0.12:22",
  ok: true,
  status: "passed",
  message: "连接成功",
  uname: "Linux",
  remote_git_version: "git version 2.43.0",
  checked_at: "2026-09-08",
};

const passedProbe: SshPasswordProbeResult = {
  ssh_config_id: saved.id,
  target_host_label: "deploy@10.0.0.12:22",
  supported: true,
  status: "passed",
  message: "密码认证探测通过",
  checked_at: "2026-09-08",
};

function makeApi(config: SshConfig = saved) {
  return {
    createSshConfig: vi.fn<SshConfigVerificationApi["createSshConfig"]>().mockResolvedValue(config),
    updateSshConfig: vi.fn<SshConfigVerificationApi["updateSshConfig"]>().mockResolvedValue(config),
    testSshConnection: vi
      .fn<SshConfigVerificationApi["testSshConnection"]>()
      .mockResolvedValue(passedTest),
    probeSshPasswordAuth: vi
      .fn<SshConfigVerificationApi["probeSshPasswordAuth"]>()
      .mockResolvedValue(passedProbe),
  };
}

describe("validateSshConfigForm", () => {
  it.each([
    ["name", keyForm({ name: " " })],
    ["host", keyForm({ host: " " })],
    ["username", keyForm({ username: " " })],
  ] as const)("requires %s", (_field, form) => {
    expect(validateSshConfigForm(form)).toBe("requiredFields");
  });

  it("requires a private key path for key auth", () => {
    expect(validateSshConfigForm(keyForm({ privateKeyPath: " " }))).toBe("privateKeyRequired");
  });

  it("rejects password probe on key auth without writing", () => {
    expect(validateSshConfigForm(keyForm(), { requirePassword: true })).toBe(
      "passwordAuthRequired",
    );
  });

  it("requires a password or an already stored password for probe", () => {
    expect(validateSshConfigForm(passwordForm({ password: "" }), { requirePassword: true })).toBe(
      "passwordRequired",
    );
    expect(
      validateSshConfigForm(passwordForm({ password: "" }), {
        requirePassword: true,
        passwordConfigured: true,
      }),
    ).toBeNull();
  });
});

describe("persistSshConfigForm", () => {
  it("does not call IPC when the form is invalid", async () => {
    const api = makeApi();
    await expect(
      persistSshConfigForm({ selectedId: null, form: keyForm({ name: "" }) }, api),
    ).rejects.toBeInstanceOf(SshConfigFormValidationError);
    expect(api.createSshConfig).not.toHaveBeenCalled();
    expect(api.updateSshConfig).not.toHaveBeenCalled();
  });

  it("creates a new configuration from the form", async () => {
    const api = makeApi();
    await persistSshConfigForm({ selectedId: null, form: passwordForm() }, api);
    expect(api.createSshConfig).toHaveBeenCalledWith({
      name: "生产主机",
      host: "10.0.0.12",
      port: 22,
      username: "deploy",
      auth_type: "password",
      private_key_path: null,
      password: "secret",
      passphrase: null,
      known_hosts_mode: "accept-new",
      algorithms: null,
    });
    expect(api.updateSshConfig).not.toHaveBeenCalled();
  });

  it("updates an existing configuration and keeps an empty password", async () => {
    const api = makeApi();
    await persistSshConfigForm(
      { selectedId: "ssh-existing", form: passwordForm({ password: "" }) },
      api,
    );
    expect(api.updateSshConfig).toHaveBeenCalledWith("ssh-existing", {
      name: "生产主机",
      host: "10.0.0.12",
      port: 22,
      username: "deploy",
      auth_type: "password",
      private_key_path: null,
      known_hosts_mode: "accept-new",
      algorithms: null,
    });
    expect(api.createSshConfig).not.toHaveBeenCalled();
  });
});

describe("persistAndTestSshConfig", () => {
  it("does not call IPC when validation fails", async () => {
    const api = makeApi();
    await expect(
      persistAndTestSshConfig({ selectedId: null, form: keyForm({ host: "" }) }, api),
    ).rejects.toThrow(SshConfigFormValidationError);
    expect(api.createSshConfig).not.toHaveBeenCalled();
    expect(api.testSshConnection).not.toHaveBeenCalled();
  });

  it("creates then tests a new configuration", async () => {
    const api = makeApi();
    const order: string[] = [];
    api.createSshConfig.mockImplementation(async () => {
      order.push("create");
      return saved;
    });
    api.testSshConnection.mockImplementation(async () => {
      order.push("test");
      return passedTest;
    });
    const onPersisted = vi.fn((config: SshConfig) => {
      order.push(`remember:${config.id}`);
    });
    await expect(
      persistAndTestSshConfig({ selectedId: null, form: keyForm() }, api, onPersisted),
    ).resolves.toEqual({ config: saved, result: passedTest });
    expect(order).toEqual(["create", "remember:ssh-new", "test"]);
    expect(api.testSshConnection).toHaveBeenCalledWith("ssh-new");
    expect(api.updateSshConfig).not.toHaveBeenCalled();
  });

  it("updates then tests an existing configuration", async () => {
    const api = makeApi();
    const order: string[] = [];
    api.updateSshConfig.mockImplementation(async () => {
      order.push("update");
      return saved;
    });
    api.testSshConnection.mockImplementation(async () => {
      order.push("test");
      return passedTest;
    });
    await persistAndTestSshConfig({ selectedId: "ssh-existing", form: passwordForm() }, api);
    expect(order).toEqual(["update", "test"]);
    expect(api.createSshConfig).not.toHaveBeenCalled();
    expect(api.testSshConnection).toHaveBeenCalledWith("ssh-new");
  });

  it("remembers the persisted id when the test throws", async () => {
    const api = makeApi();
    api.testSshConnection.mockRejectedValueOnce(new Error("IPC denied"));
    const onPersisted = vi.fn();
    await expect(
      persistAndTestSshConfig({ selectedId: null, form: passwordForm() }, api, onPersisted),
    ).rejects.toThrow("IPC denied");
    expect(onPersisted).toHaveBeenCalledWith(saved);
  });
});

describe("persistAndProbeSshPasswordAuth", () => {
  it("rejects key auth without writing", async () => {
    const api = makeApi();
    await expect(
      persistAndProbeSshPasswordAuth({ selectedId: null, form: keyForm() }, api),
    ).rejects.toMatchObject({ field: "passwordAuthRequired" });
    expect(api.createSshConfig).not.toHaveBeenCalled();
    expect(api.updateSshConfig).not.toHaveBeenCalled();
    expect(api.probeSshPasswordAuth).not.toHaveBeenCalled();
  });

  it("creates then probes a new password configuration", async () => {
    const api = makeApi();
    const order: string[] = [];
    api.createSshConfig.mockImplementation(async () => {
      order.push("create");
      return saved;
    });
    api.probeSshPasswordAuth.mockImplementation(async () => {
      order.push("probe");
      return passedProbe;
    });
    await expect(
      persistAndProbeSshPasswordAuth({ selectedId: null, form: passwordForm() }, api),
    ).resolves.toEqual({ config: saved, result: passedProbe });
    expect(order).toEqual(["create", "probe"]);
    expect(api.probeSshPasswordAuth).toHaveBeenCalledWith("ssh-new");
  });

  it("updates then probes when a stored password already exists", async () => {
    const api = makeApi();
    await persistAndProbeSshPasswordAuth(
      {
        selectedId: "ssh-existing",
        form: passwordForm({ password: "" }),
        passwordConfigured: true,
      },
      api,
    );
    expect(api.updateSshConfig).toHaveBeenCalled();
    expect(api.probeSshPasswordAuth).toHaveBeenCalledWith("ssh-new");
    expect(api.createSshConfig).not.toHaveBeenCalled();
  });
});
