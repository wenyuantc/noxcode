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

  it("renders the off pill when a workspace is selected", () => {
    useWorkspaceStore.setState({ activeWorkspaceId: "ws-1" });
    const html = renderToStaticMarkup(<WorktreeToggle />);
    expect(html).toContain("isolateWorktree");
    expect(html).toContain("isolateWorktreeHint");
    expect(html).toContain('aria-pressed="false"');
    expect(html).not.toContain("isolateWorktreeClear");
  });

  it("shows the clear button when isolation is on", () => {
    useWorkspaceStore.setState({ activeWorkspaceId: "ws-1" });
    useUiStore.setState({ composerIsolateWorktree: true });
    const html = renderToStaticMarkup(<WorktreeToggle />);
    expect(html).toContain("isolateWorktreeClear");
    expect(html).toContain('aria-pressed="true"');
  });
});
