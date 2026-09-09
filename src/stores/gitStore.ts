import { create } from "zustand";

import { pullGitBranch } from "@/lib/backend";
import type { GitPullResult } from "@/lib/types";

export type GitPullState =
  | { status: "pulling" }
  | { status: "success"; result: GitPullResult }
  | { status: "error"; error: string };

interface GitState {
  pulls: Record<string, GitPullState | undefined>;
  revision: number;
  bumpRevision: () => void;
  pull: (workspaceId: string) => Promise<void>;
}

// Pulls outlive the sidebar so reopening it cannot start a duplicate operation.
export const useGitStore = create<GitState>((set, get) => ({
  pulls: {},
  revision: 0,
  bumpRevision: () => set((state) => ({ revision: state.revision + 1 })),
  pull: async (workspaceId) => {
    if (get().pulls[workspaceId]?.status === "pulling") return;
    const update = (value: GitPullState) =>
      set((state) => ({ pulls: { ...state.pulls, [workspaceId]: value } }));
    update({ status: "pulling" });
    try {
      update({ status: "success", result: await pullGitBranch(workspaceId) });
    } catch (error) {
      update({ status: "error", error: error instanceof Error ? error.message : String(error) });
    }
  },
}));
