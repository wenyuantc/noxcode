import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { WorktreeToggle } from "./WorktreeToggle";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
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

vi.mock("@/stores/uiStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/uiStore")>();
  return {
    useUiStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useUiStore.getState>) => unknown) =>
        selector(actual.useUiStore.getState()),
      actual.useUiStore,
    ),
  };
});

describe("WorktreeToggle", () => {
  beforeEach(() => {
    useWorkspaceStore.setState({ activeWorkspaceId: null });
    useUiStore.setState({ composerIsolateWorktree: false });
  });

  it("renders nothing without a workspace", () => {
    expect(renderToStaticMarkup(<WorktreeToggle />)).toBe("");
  });

  it("selects the current workspace segment by default", () => {
    useWorkspaceStore.setState({ activeWorkspaceId: "ws-1" });
    const html = renderToStaticMarkup(<WorktreeToggle />);
    expect(html).toContain("worktreeModeCurrent");
    expect(html).toContain("worktreeModeIsolate");
    expect(html).toContain("isolateWorktreeHint");
    expect(html).toMatch(/aria-pressed="true"[\s\S]*worktreeModeCurrent/);
    expect(html).toMatch(/aria-pressed="false"[\s\S]*worktreeModeIsolate/);
    expect(html).not.toContain("isolateWorktreeClear");
  });

  it("marks the isolate segment as pressed when isolation is on", () => {
    useWorkspaceStore.setState({ activeWorkspaceId: "ws-1" });
    useUiStore.setState({ composerIsolateWorktree: true });
    const html = renderToStaticMarkup(<WorktreeToggle />);
    expect(html).toMatch(/aria-pressed="false"[\s\S]*worktreeModeCurrent/);
    expect(html).toMatch(/aria-pressed="true"[\s\S]*worktreeModeIsolate/);
  });
});
