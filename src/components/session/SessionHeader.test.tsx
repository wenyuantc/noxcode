import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SessionHeader } from "./SessionHeader";

vi.mock("@/lib/backend", () => ({
  getWorktreeMergeState: vi.fn(async () => ({ in_progress: false, conflicts: [] })),
  listGitBranches: vi.fn(async () => []),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
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

vi.mock("@/stores/settingsStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/settingsStore")>();
  return {
    useSettingsStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useSettingsStore.getState>) => unknown) =>
        selector(actual.useSettingsStore.getState()),
      actual.useSettingsStore,
    ),
  };
});

describe("SessionHeader", () => {
  beforeEach(() => {
    useSessionStore.setState({ selectedSessionId: null });
    useWorkspaceStore.setState({ activeWorkspaceId: null, sessions: [] });
  });

  it("stacks above the event stream so branch menus are not covered", () => {
    const html = renderToStaticMarkup(<SessionHeader />);
    expect(html).toContain("z-40");
  });
});
