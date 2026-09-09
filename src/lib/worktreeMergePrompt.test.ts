import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend", () => ({
  getWorktreeMergeState: vi.fn(async () => ({ in_progress: false, conflicts: [] })),
}));

import { getWorktreeMergeState } from "@/lib/backend";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { maybeOpenWorktreeMerge } from "./worktreeMergePrompt";

const getState = vi.mocked(getWorktreeMergeState);

describe("maybeOpenWorktreeMerge", () => {
  beforeEach(() => {
    getState.mockReset();
    getState.mockResolvedValue({ in_progress: false, conflicts: [] });
    useSessionStore.setState({
      worktreeMergePrompt: null,
      mergedWorktreeBySession: {},
      autoPromptedWorktreeBySession: {},
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

  it("still opens on process exit after a dismissed turn prompt", async () => {
    await maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "turn" });
    useSessionStore.getState().closeWorktreeMergePrompt();
    await expect(
      maybeOpenWorktreeMerge({ sessionId: "s1", workspaceId: "ws-1", reason: "exit" }),
    ).resolves.toBe(true);
    expect(useSessionStore.getState().worktreeMergePrompt?.sessionId).toBe("s1");
  });
});
