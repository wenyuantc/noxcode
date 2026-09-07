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
});
