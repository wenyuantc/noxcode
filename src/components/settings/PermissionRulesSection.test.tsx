import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getNativePermissionRules } from "@/lib/backend";
import type { Workspace } from "@/lib/types";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { PermissionRulesSection } from "./PermissionRulesSection";

vi.mock("@/lib/backend", () => ({
  getNativePermissionRules: vi.fn(),
  addNativePermissionRule: vi.fn(),
  deleteNativePermissionRule: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string; path?: string }) => {
      if (opts?.defaultValue) return opts.defaultValue;
      if (opts?.path) return `path:${opts.path}`;
      return key;
    },
  }),
}));

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

function sampleWorkspace(id: string, name: string): Workspace {
  return {
    id,
    name,
    workspace_type: "local",
    repo_path: `/repos/${id}`,
    ssh_config_id: null,
    remote_repo_path: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

describe("PermissionRulesSection workspace selector", () => {
  beforeEach(() => {
    vi.mocked(getNativePermissionRules)
      .mockReset()
      .mockResolvedValue({
        global: { allow: [], deny: [], ask: [] },
        workspace: { allow: [], deny: [], ask: [] },
        workspace_root: "/repos/ws-1",
        workspace_rules_path: "/repos/ws-1/.noxcode/permissions.json",
        workspace_target: { kind: "local" },
      });
  });

  it("renders workspace selector dropdown with active workspace selected", () => {
    useWorkspaceStore.setState({
      workspaces: [
        sampleWorkspace("ws-1", "Alpha Project"),
        sampleWorkspace("ws-2", "Beta Project"),
      ],
      activeWorkspaceId: "ws-1",
    });

    const html = renderToStaticMarkup(<PermissionRulesSection />);

    expect(html).toContain("工作区规则 (Workspace)");
    expect(html).toContain("Alpha Project");
    expect(html).toContain("全局规则 (Global)");
  });

  it("renders disabled selector with empty message when no workspaces exist", () => {
    useWorkspaceStore.setState({
      workspaces: [],
      activeWorkspaceId: null,
    });

    const html = renderToStaticMarkup(<PermissionRulesSection />);

    expect(html).toContain("工作区规则 (Workspace)");
    expect(html).toContain("暂无工作区");
  });
});
