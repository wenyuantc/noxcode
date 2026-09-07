import { describe, expect, it, vi } from "vitest";

import { submitRemoteConnection, type RemoteConnectionApi } from "./remoteConnection";
import type { CreateSshConfigInput, SshConfig, SshConnectionTestResult } from "./types";

const form: CreateSshConfigInput = {
  name: "Server",
  host: "example.com",
  username: "deploy",
  port: 22,
  auth_type: "password",
  password: "secret",
};
const saved: SshConfig = {
  id: "ssh-new",
  name: "Server",
  host: "example.com",
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
  created_at: "2026-09-07",
  updated_at: "2026-09-07",
};
const passed: SshConnectionTestResult = {
  ssh_config_id: saved.id,
  target_host_label: "deploy@example.com:22",
  ok: true,
  status: "passed",
  message: "Connected",
  uname: "Linux",
  remote_git_version: "git version 2.43.0",
  checked_at: "2026-09-07",
};

function makeApi() {
  return {
    createSshConfig: vi.fn<RemoteConnectionApi["createSshConfig"]>().mockResolvedValue(saved),
    testSshConnection: vi.fn<RemoteConnectionApi["testSshConnection"]>().mockResolvedValue(passed),
    createWorkspace: vi.fn<RemoteConnectionApi["createWorkspace"]>().mockResolvedValue(undefined),
    onConfigCreated: vi.fn<RemoteConnectionApi["onConfigCreated"]>(),
  };
}

const base = { name: "Workspace", remotePath: " /srv/repo " };

describe("submitRemoteConnection", () => {
  const invalidInputs: Parameters<typeof submitRemoteConnection>[0][] = [
    { ...base, sshConfigId: "ssh-existing", remotePath: " " },
    { ...base, newConfig: form, remotePath: " " },
    { ...base, sshConfigId: " " },
    ...["name", "host", "username"].map((field) => ({
      ...base,
      newConfig: { ...form, [field]: " " },
    })),
    { ...base, newConfig: { ...form, password: "" } },
    { ...base, newConfig: { ...form, auth_type: "key", private_key_path: " " } },
    ...[0, 65536, 1.5, Number.NaN].map((port) => ({ ...base, newConfig: { ...form, port } })),
  ];

  it.each(invalidInputs)("rejects invalid input without side effects: %j", async (input) => {
    const api = makeApi();
    await expect(submitRemoteConnection(input, api)).rejects.toThrow();
    expect(api.createSshConfig).not.toHaveBeenCalled();
    expect(api.testSshConnection).not.toHaveBeenCalled();
    expect(api.createWorkspace).not.toHaveBeenCalled();
    expect(api.onConfigCreated).not.toHaveBeenCalled();
  });

  it.each(["existing", "new"] as const)(
    "tests the %s configuration before creating a workspace",
    async (kind) => {
      const api = makeApi();
      const order: string[] = [];
      api.createSshConfig.mockImplementation(async () => {
        order.push("save");
        return saved;
      });
      api.onConfigCreated.mockImplementation(() => {
        order.push("remember");
      });
      api.testSshConnection.mockImplementation(async () => {
        order.push("test");
        return passed;
      });
      api.createWorkspace.mockImplementation(async () => {
        order.push("workspace");
      });
      const input =
        kind === "new" ? { ...base, newConfig: form } : { ...base, sshConfigId: "ssh-existing" };
      await expect(submitRemoteConnection(input, api)).resolves.toBe(true);
      expect(order).toEqual(
        kind === "new" ? ["save", "remember", "test", "workspace"] : ["test", "workspace"],
      );
      expect(api.createWorkspace).toHaveBeenCalledWith({
        name: "Workspace",
        workspace_type: "ssh",
        ssh_config_id: kind === "new" ? saved.id : "ssh-existing",
        remote_repo_path: "/srv/repo",
      });
    },
  );

  it.each(["failed", "rejected"] as const)(
    "stops workspace creation on a %s existing-configuration test",
    async (failure) => {
      const api = makeApi();
      if (failure === "failed") {
        api.testSshConnection.mockResolvedValueOnce({
          ...passed,
          ok: false,
          message: "Authentication failed",
        });
      } else {
        api.testSshConnection.mockRejectedValueOnce(new Error("IPC denied"));
      }
      await expect(
        submitRemoteConnection({ ...base, sshConfigId: "existing" }, api),
      ).rejects.toThrow(failure === "failed" ? "Authentication failed" : "IPC denied");
      expect(api.createSshConfig).not.toHaveBeenCalled();
      expect(api.createWorkspace).not.toHaveBeenCalled();
    },
  );

  it.each(["failed", "rejected"] as const)(
    "retains the configuration ID after a %s test and reuses it on retry",
    async (failure) => {
      const api = makeApi();
      let retainedId: string | undefined;
      api.onConfigCreated.mockImplementation((config) => {
        retainedId = config.id;
      });
      api.testSshConnection.mockImplementationOnce(async () => {
        expect(retainedId).toBe(saved.id);
        if (failure === "rejected") throw new Error("IPC denied");
        return { ...passed, ok: false, message: "Authentication failed" };
      });
      await expect(submitRemoteConnection({ ...base, newConfig: form }, api)).rejects.toThrow(
        failure === "failed" ? "Authentication failed" : "IPC denied",
      );
      expect(api.createWorkspace).not.toHaveBeenCalled();
      expect(retainedId).toBe(saved.id);
      await expect(submitRemoteConnection({ ...base, sshConfigId: retainedId }, api)).resolves.toBe(
        true,
      );
      expect(api.createSshConfig).toHaveBeenCalledTimes(1);
      expect(api.testSshConnection).toHaveBeenNthCalledWith(1, saved.id);
      expect(api.testSshConnection).toHaveBeenNthCalledWith(2, saved.id);
      expect(api.createWorkspace).toHaveBeenCalledTimes(1);
    },
  );

  it("does not create a workspace when the dialog becomes stale during testing", async () => {
    const api = makeApi();
    let current = true;
    api.testSshConnection.mockImplementation(async () => {
      current = false;
      return passed;
    });
    await expect(
      submitRemoteConnection(
        { ...base, sshConfigId: "existing" },
        { ...api, isCurrent: () => current },
      ),
    ).resolves.toBe(false);
    expect(api.createWorkspace).not.toHaveBeenCalled();
  });
});
