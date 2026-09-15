import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useGitStore } from "@/stores/gitStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { GitPanel, gitPullToast } from "./GitPanel";

vi.mock("@/lib/backend", () => ({
  clearGitCheckpoints: vi.fn(),
  commitGitChanges: vi.fn(),
  generateGitCommitMessage: vi.fn(),
  getGitStatus: vi.fn(),
  listActivityLogs: vi.fn(),
  listGitCheckpoints: vi.fn(),
  previewGitCheckpointRestore: vi.fn(),
  pushGitBranch: vi.fn(),
  restoreGitCheckpoint: vi.fn(),
  restoreGitPaths: vi.fn(),
  stageGitPaths: vi.fn(),
  unstageGitPaths: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

vi.mock("@/lib/toast", () => ({
  showToast: vi.fn(),
  runToastAction: vi.fn(),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "zh-CN" },
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

vi.mock("@/stores/gitStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/gitStore")>();
  return {
    useGitStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useGitStore.getState>) => unknown) =>
        selector(actual.useGitStore.getState()),
      actual.useGitStore,
    ),
  };
});

const translate = (key: string) => key;

describe("gitPullToast", () => {
  it("maps a successful pull to exactly one success toast", () => {
    expect(
      gitPullToast({ status: "success", result: { updated: true, message: "pulled" } }, translate),
    ).toEqual({ variant: "success", description: "pullSuccess" });
    expect(
      gitPullToast({ status: "success", result: { updated: false, message: "latest" } }, translate),
    ).toEqual({ variant: "success", description: "pullUpToDate" });
  });

  it("stays silent before a pull starts and while it is still running", () => {
    expect(gitPullToast(undefined, translate)).toBeNull();
    expect(gitPullToast({ status: "pulling" }, translate)).toBeNull();
  });

  it("maps a failed pull to a manual-close error toast carrying the backend message", () => {
    expect(gitPullToast({ status: "error", error: "fatal: no upstream" }, translate)).toEqual({
      variant: "error",
      title: "pullFailed",
      description: "fatal: no upstream",
    });
  });
});

describe("GitPanel pull feedback", () => {
  beforeEach(() => {
    useWorkspaceStore.setState({ activeWorkspaceId: "ws-1", sessions: [] });
    useGitStore.setState({ pulls: {} });
  });

  it("does not render an inline result bar after a successful pull", () => {
    useGitStore.setState({
      pulls: { "ws-1": { status: "success", result: { updated: true, message: "pulled" } } },
    });

    const html = renderToStaticMarkup(<GitPanel />);

    expect(html).not.toContain("pullSuccess");
    expect(html).not.toContain("pullUpToDate");
    expect(html).not.toContain('role="status"');
    expect(html).not.toContain('role="alert"');
  });

  it("does not render an inline error bar after a failed pull", () => {
    useGitStore.setState({
      pulls: { "ws-1": { status: "error", error: "fatal: no upstream" } },
    });

    const html = renderToStaticMarkup(<GitPanel />);

    expect(html).not.toContain("pullFailed");
    expect(html).not.toContain("fatal: no upstream");
    expect(html).not.toContain('role="alert"');
  });

  it("keeps the pulling state visible through the busy indicator", () => {
    useGitStore.setState({ pulls: { "ws-1": { status: "pulling" } } });

    const html = renderToStaticMarkup(<GitPanel />);

    expect(html).toContain('title="pulling"');
    expect(html).toMatch(/aria-label="pull"[^>]*disabled/);
    expect(html).toContain("animate-spin");
  });
});
