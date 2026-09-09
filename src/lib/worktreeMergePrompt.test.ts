import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend", () => ({
  getWorktreeMergeState: vi.fn(async () => ({ in_progress: false, conflicts: [] })),
  resolveSessionWorktreeMerge: vi.fn(),
}));

import { getWorktreeMergeState, resolveSessionWorktreeMerge } from "@/lib/backend";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import {
  maybeFinishAiMergeResolve,
  maybeOpenWorktreeMerge,
  mergeConflictResolvePrompt,
  mergeWorktreeDialogKey,
} from "./worktreeMergePrompt";

const getState = vi.mocked(getWorktreeMergeState);
const completeMerge = vi.mocked(resolveSessionWorktreeMerge);

describe("maybeOpenWorktreeMerge", () => {
  beforeEach(() => {
    getState.mockReset();
    getState.mockResolvedValue({ in_progress: false, conflicts: [] });
    completeMerge.mockReset();
    useSessionStore.setState({
      worktreeMergePrompt: null,
      mergedWorktreeBySession: {},
      autoPromptedWorktreeBySession: {},
      pendingAiMergeResolveBySession: {},
      configurationBySession: {
        s1: {
          ai_channel_id: "c",
          model: "m",
          reasoning_effort: null,
          permission_mode: "ask",
          plan_mode: false,
          worktree_path: "/cfg/worktrees/s1",
        },
      },
    });
    useWorkspaceStore.setState({
      sessions: [
        {
          id: "s1",
          workspace_id: "ws-1",
          working_dir: "/cfg/worktrees/s1",
        } as never,
      ],
      refreshSessions: vi.fn(async () => undefined),
    });
  });

  it("opens the merge dialog when a turn finishes on an isolated worktree", async () => {
    await expect(
      maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "turn" }),
    ).resolves.toBe(true);
    expect(useSessionStore.getState().worktreeMergePrompt).toMatchObject({
      sessionId: "s1",
      workspaceId: "ws-1",
      phase: "choose",
    });
    expect(useSessionStore.getState().autoPromptedWorktreeBySession.s1).toBe(true);
  });

  it("does not prompt again on later waiting_input after keep", async () => {
    await maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "turn" });
    useSessionStore.getState().closeWorktreeMergePrompt();
    await expect(
      maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "turn" }),
    ).resolves.toBe(false);
    expect(useSessionStore.getState().worktreeMergePrompt).toBeNull();
  });

  it("keys the merge dialog so another session cannot reuse the last form", () => {
    expect(mergeWorktreeDialogKey(null)).toBe("closed");
    expect(mergeWorktreeDialogKey(undefined)).toBe("closed");
    expect(mergeWorktreeDialogKey({ sessionId: "s1" })).toBe("s1");
    expect(mergeWorktreeDialogKey({ sessionId: "s2" })).toBe("s2");
    expect(mergeWorktreeDialogKey({ sessionId: "  " })).toBe("closed");
    expect(mergeWorktreeDialogKey({ sessionId: "s1" })).not.toBe(
      mergeWorktreeDialogKey({ sessionId: "s2" }),
    );
  });

  it("builds a visible session prompt for conflict files", () => {
    const text = mergeConflictResolvePrompt(["README.md", "src/main.rs"]);
    expect(text).toContain("- README.md");
    expect(text).toContain("- src/main.rs");
    expect(text).toContain("ExitWorktree");
    expect(text).toContain("Write");
  });

  it("does not finish an AI merge when none is pending", async () => {
    await expect(maybeFinishAiMergeResolve({ sessionId: "s1", workspaceId: "ws-1" })).resolves.toBe(
      false,
    );
    expect(completeMerge).not.toHaveBeenCalled();
  });

  it("completes a pending AI merge after the session turn", async () => {
    completeMerge.mockResolvedValue({
      status: "resolved",
      conflicts: [],
      resolved: ["README.md"],
      failed: [],
      message: "done",
    });
    useSessionStore.getState().markPendingAiMergeResolve("s1");
    await expect(maybeFinishAiMergeResolve({ sessionId: "s1", workspaceId: "ws-1" })).resolves.toBe(
      true,
    );
    expect(completeMerge).toHaveBeenCalledWith("ws-1", "s1", "complete");
    expect(useSessionStore.getState().mergedWorktreeBySession.s1).toBe(true);
    expect(useSessionStore.getState().pendingAiMergeResolveBySession.s1).toBeUndefined();
  });

  it("reopens the conflict dialog when the session did not clear markers", async () => {
    completeMerge.mockResolvedValue({
      status: "partial",
      conflicts: ["README.md"],
      resolved: [],
      failed: ["README.md: 模型输出仍含冲突标记"],
      message: "still",
    });
    useSessionStore.getState().markPendingAiMergeResolve("s1");
    await maybeFinishAiMergeResolve({ sessionId: "s1", workspaceId: "ws-1" });
    expect(useSessionStore.getState().worktreeMergePrompt).toMatchObject({
      sessionId: "s1",
      workspaceId: "ws-1",
      phase: "conflict",
      conflicts: ["README.md"],
    });
  });

  it("still opens on process exit after a dismissed turn prompt", async () => {
    await maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "turn" });
    useSessionStore.getState().closeWorktreeMergePrompt();
    await expect(
      maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "exit" }),
    ).resolves.toBe(true);
    expect(useSessionStore.getState().worktreeMergePrompt?.sessionId).toBe("s1");
  });
});
