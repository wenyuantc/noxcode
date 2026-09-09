import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { MergeWorktreeDialog } from "./MergeWorktreeDialog";

vi.mock("@/lib/backend", () => ({
  mergeSessionWorktree: vi.fn(),
  resolveSessionWorktreeMerge: vi.fn(),
  generateGitCommitMessage: vi.fn(),
  listGitBranches: vi.fn(async () => [{ name: "dev", is_current: true }]),
  startNativeSession: vi.fn(),
  resumeNativeSession: vi.fn(),
}));

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

describe("MergeWorktreeDialog", () => {
  beforeEach(() => {
    useSessionStore.setState({ worktreeMergePrompt: null });
    useSettingsStore.setState({
      ai: {
        commit_message: {
          enabled: true,
          channel_id: null,
          model: null,
          reasoning_effort: null,
          style: "detailed",
        },
        session_title: {
          enabled: false,
          channel_id: null,
          model: null,
          reasoning_effort: null,
        },
      },
    });
  });

  it("renders nothing when there is no prompt", () => {
    expect(renderToStaticMarkup(<MergeWorktreeDialog />)).toBe("");
  });

  it("renders choose actions for an isolated session", () => {
    useSessionStore.setState({
      worktreeMergePrompt: {
        sessionId: "s1",
        workspaceId: "ws-1",
        phase: "choose",
        conflicts: [],
      },
    });
    const html = renderToStaticMarkup(<MergeWorktreeDialog />);
    expect(html).toContain("git:mergeWorktreeTitle");
    expect(html).toContain("git:mergeWorktreeCurrent");
    expect(html).toContain("git:mergeWorktreeBranch");
    expect(html).toContain("git:mergeWorktreeKeep");
    expect(html).toContain("git:mergeWorktreeBranchName");
    expect(html).toContain("git:mergeWorktreeBranchNameHint");
    expect(html).toContain("git:commitMessage");
    expect(html).toContain("git:mergeWorktreeCommitHint");
    expect(html).toContain("git:generateCommit");
    expect(html).toContain('role="combobox"');
  });

  it("renders conflict files and resolve actions", () => {
    useSessionStore.setState({
      worktreeMergePrompt: {
        sessionId: "s1",
        workspaceId: "ws-1",
        phase: "conflict",
        conflicts: ["README.md", "src/main.rs"],
      },
    });
    const html = renderToStaticMarkup(<MergeWorktreeDialog />);
    expect(html).toContain("git:mergeWorktreeConflictsTitle");
    expect(html).toContain("README.md");
    expect(html).toContain("src/main.rs");
    expect(html).toContain("git:mergeWorktreeAi");
    expect(html).toContain("git:mergeWorktreeManual");
    expect(html).toContain("git:mergeWorktreeAbort");
  });
});
