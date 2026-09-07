import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NativePermissionDialog } from "./NativePermissionDialog";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

vi.mock("@/lib/backend", () => ({ resolveNativeToolPermission: vi.fn() }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/components/ui/dialog", () => {
  const Part = ({ children }: { children: ReactNode }) => <div>{children}</div>;
  return {
    Dialog: Part,
    DialogContent: Part,
    DialogDescription: Part,
    DialogFooter: Part,
    DialogHeader: Part,
    DialogTitle: Part,
  };
});
vi.mock("@/components/ui/button", () => ({
  Button: ({ children }: { children: ReactNode }) => <button>{children}</button>,
}));
vi.mock("@/stores/sessionStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/sessionStore")>();
  return {
    useSessionStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useSessionStore.getState>) => unknown) =>
        selector(actual.useSessionStore.getState()),
      actual.useSessionStore,
    ),
  };
});
vi.mock("@/stores/workspaceStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/workspaceStore")>();
  return {
    useWorkspaceStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useWorkspaceStore.getState>) => unknown) =>
        selector(actual.useWorkspaceStore.getState()),
      actual.useWorkspaceStore,
    ),
  };
});

describe("plan Bash permission controls", () => {
  beforeEach(() => {
    useSessionStore.setState({ permissions: {}, selectedSessionId: "plan" });
    useWorkspaceStore.setState({ sessions: [] });
  });

  it("offers always allow for a plan command without offering session access", () => {
    useSessionStore.getState().setPermission({
      session_record_id: "plan",
      request_id: "request",
      profile_id: "",
      workspace_id: "workspace",
      session_kind: "plan",
      tool_name: "Bash",
      kind: "opaque",
      summary:
        'ls -la "$HOME/Library/Application Support/com.wenyuan.noxcode/" 2>/dev/null | head -30',
      remote: false,
      mcp_server_id: null,
      allow_once_only: false,
      suggested_rule: {
        capability: "bash",
        source: "command",
        pattern: "ls -la",
        plan_bash: { target: { kind: "local" }, workspace_root: "/workspace" },
      },
    });
    const html = renderToStaticMarkup(<NativePermissionDialog />);
    expect(html).toContain("permissionAllowOnce");
    expect(html).toContain("permissionAlways");
    expect(html).toContain("permissionDeny");
    expect(html).not.toContain("permissionAllowSession");
    expect(html).not.toContain("permissionAllowServer");
    expect(html).toContain("/workspace");
  });

  it.each([true, false])(
    "limits one-call request actions when allow_once_only=%s",
    (allowOnceOnly) => {
      useSessionStore.getState().setPermission({
        session_record_id: "plan",
        request_id: "request",
        profile_id: "",
        workspace_id: "workspace",
        session_kind: "plan",
        tool_name: "Bash",
        kind: "overwrite",
        summary: "printf approved > file.txt",
        remote: false,
        mcp_server_id: null,
        allow_once_only: allowOnceOnly,
        suggested_rule: { capability: "bash", source: "command", pattern: "printf*" },
      });
      const html = renderToStaticMarkup(<NativePermissionDialog />);
      expect(html).toContain("permissionAllowOnce");
      expect(html).toContain("permissionDeny");
      expect(html.includes("permissionAllowAlways")).toBe(!allowOnceOnly);
      expect(html.includes("permissionAllowSession")).toBe(!allowOnceOnly);
    },
  );

  it("renders risk callout and command block for opaque long bash command", () => {
    const longCmd =
      'grep -rn "continuation" /Users/wenyuantc/IdeaProjects/my/noxcode/src-tauri/src/native/model/client.rs | grep -in "config\\|disable" -C 5 | head; echo "---"; grep -rn "supports_continuation\\|disable_continuation\\|no_continuation" /Users/wenyuantc/IdeaProjects/my/noxcode/src-tauri/src/';
    useSessionStore.getState().setPermission({
      session_record_id: "session-1",
      request_id: "req-opaque",
      profile_id: "",
      workspace_id: "workspace",
      session_kind: "chat",
      tool_name: "Bash",
      kind: "opaque",
      summary: `不透明命令：${longCmd}`,
      remote: false,
      mcp_server_id: null,
      allow_once_only: false,
    });
    const html = renderToStaticMarkup(<NativePermissionDialog />);
    expect(html).toContain("不透明命令");
    expect(html).toContain("continuation");
    expect(html).toContain("supports_continuation");
    expect(html).toContain("permissionCommand");
    expect(html).toContain("permissionAllowOnce");
    expect(html).toContain("permissionDeny");
  });

  describe("parsePermissionSummary", () => {
    it("handles undefined or empty summary", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary(undefined, "Bash")).toEqual({
        riskReason: null,
        command: null,
        detail: "",
      });
      expect(parsePermissionSummary("", "Bash")).toEqual({
        riskReason: null,
        command: null,
        detail: "",
      });
    });

    it("splits Chinese colon for bash command", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary("不透明命令：ls -la", "Bash")).toEqual({
        riskReason: "不透明命令",
        command: "ls -la",
        detail: "ls -la",
      });
    });

    it("splits ASCII colon for bash command", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary("Risk: echo hello", "bash")).toEqual({
        riskReason: "Risk",
        command: "echo hello",
        detail: "echo hello",
      });
    });

    it("treats entire summary as command when bash has no colon", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary("cat /etc/hosts", "Bash")).toEqual({
        riskReason: null,
        command: "cat /etc/hosts",
        detail: "cat /etc/hosts",
      });
    });

    it("handles non-bash tool with risk prefix", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary("应用补丁：新增 5 行", "ApplyPatch")).toEqual({
        riskReason: "应用补丁",
        command: null,
        detail: "新增 5 行",
      });
    });

    it("handles non-bash tool without risk prefix", async () => {
      const { parsePermissionSummary } = await import("./NativePermissionDialog");
      expect(parsePermissionSummary("覆盖已有文件 /path/file", "Edit")).toEqual({
        riskReason: null,
        command: null,
        detail: "覆盖已有文件 /path/file",
      });
    });
  });
});
