import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NativePermissionDialog } from "./NativePermissionDialog";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { resolveNativeToolPermission } from "@/lib/backend";

const buttonActions = vi.hoisted(() => new Map<string, () => unknown>());

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
  Button: ({ children, onClick }: { children: ReactNode; onClick?: () => unknown }) => {
    if (onClick) buttonActions.set(String(children), onClick);
    return <button>{children}</button>;
  },
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
    buttonActions.clear();
    vi.mocked(resolveNativeToolPermission).mockReset().mockResolvedValue(undefined);
    useSessionStore.setState({
      permissions: {},
      selectedSessionId: "plan",
      configurationBySession: {},
      planModeBySession: {},
    });
    useWorkspaceStore.setState({ sessions: [] });
  });

  it("grants all session commands without exiting plan mode or changing permission mode", async () => {
    const runtime = {
      ai_channel_id: "channel",
      model: "model",
      reasoning_effort: null,
      permission_mode: "default",
      plan_mode: true,
    };
    useSessionStore.getState().setConfiguration("plan", runtime);
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
    expect(html).toContain("permissionAllowSessionCommands");
    expect(html).not.toContain(">permissionAllowSession<");
    expect(html).not.toContain("permissionAllowServer");
    expect(html).toContain("/workspace");
    await buttonActions.get("permissionAllowSessionCommands")!();
    expect(resolveNativeToolPermission).toHaveBeenCalledWith(
      "plan",
      "request",
      "allow_session_commands",
      undefined,
      undefined,
    );
    const state = useSessionStore.getState();
    expect(state.permissions.plan.request).toBeUndefined();
    expect(state.planModeBySession.plan).toBe(true);
    expect(state.configurationBySession.plan).toEqual(runtime);
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
      expect(html.includes("permissionAllowSessionCommands")).toBe(!allowOnceOnly);
      expect(html).not.toContain(">permissionAllowSession<");
    },
  );

  it("keeps the request available when session command approval fails", async () => {
    useSessionStore.getState().setPermission({
      session_record_id: "plan",
      request_id: "retry",
      profile_id: "",
      workspace_id: "workspace",
      session_kind: "plan",
      tool_name: "Bash",
      kind: "opaque",
      summary: "touch file",
      remote: false,
      mcp_server_id: null,
    });
    vi.mocked(resolveNativeToolPermission).mockRejectedValueOnce(new Error("expired"));
    renderToStaticMarkup(<NativePermissionDialog />);
    await buttonActions.get("permissionAllowSessionCommands")!();
    expect(useSessionStore.getState().permissions.plan.retry).toBeDefined();
    expect(useSessionStore.getState().configurationBySession.plan).toBeUndefined();
  });

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
