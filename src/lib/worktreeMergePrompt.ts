import { getWorktreeMergeState } from "@/lib/backend";
import type { WorktreeMergePrompt } from "@/lib/types";
import { isManagedWorktreePath } from "@/lib/worktreePath";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function mergeWorktreeDialogKey(
  prompt: Pick<WorktreeMergePrompt, "sessionId"> | null | undefined,
): string {
  const sessionId = prompt?.sessionId.trim() ?? "";
  return sessionId || "closed";
}

export async function maybeOpenWorktreeMerge(input: {
  sessionId: string;
  workspaceId?: string | null;
  worktreePath?: string | null;
  reason: "exit" | "turn";
}): Promise<boolean> {
  const store = useSessionStore.getState();
  if (store.mergedWorktreeBySession[input.sessionId]) return false;
  if (store.worktreeMergePrompt?.sessionId === input.sessionId) return false;
  if (input.reason === "turn" && store.autoPromptedWorktreeBySession[input.sessionId]) {
    return false;
  }

  await useWorkspaceStore.getState().refreshSessions();

  const next = useSessionStore.getState();
  const session = useWorkspaceStore.getState().sessions.find((item) => item.id === input.sessionId);
  const runtime = next.configurationBySession[input.sessionId];
  const workspaceId = input.workspaceId ?? session?.workspace_id ?? null;
  const path = input.worktreePath ?? runtime?.worktree_path ?? session?.working_dir;
  const worktreeRoot = useSettingsStore.getState().native?.worktree_root;
  if (!workspaceId || !isManagedWorktreePath(path, input.sessionId, worktreeRoot)) return false;

  if (input.reason === "turn") {
    useSessionStore.getState().markWorktreeAutoPrompted(input.sessionId);
  }

  let phase: "choose" | "conflict" = "choose";
  let conflicts: string[] = [];
  try {
    const state = await getWorktreeMergeState(workspaceId);
    if (state.in_progress) {
      phase = "conflict";
      conflicts = state.conflicts;
    }
  } catch {
    // 查不到合并状态时仍弹出选择，避免结束时漏掉 worktree。
  }

  useSessionStore.getState().openWorktreeMergePrompt({
    sessionId: input.sessionId,
    workspaceId,
    phase,
    conflicts,
  });
  return true;
}
