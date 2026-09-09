import {
  getWorktreeMergeState,
  resolveSessionWorktreeMerge,
  restoreSessionWorktree,
} from "@/lib/backend";
import type { WorktreeMergePrompt } from "@/lib/types";
import { promptT } from "@/lib/promptI18n";
import { isManagedWorktreePath } from "@/lib/worktreePath";
import { useGitStore } from "@/stores/gitStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function mergeWorktreeDialogKey(
  prompt: Pick<WorktreeMergePrompt, "sessionId"> | null | undefined,
): string {
  const sessionId = prompt?.sessionId.trim() ?? "";
  return sessionId || "closed";
}

export function mergeConflictResolvePrompt(conflicts: string[]): string {
  const list = conflicts
    .map((path) => path.trim())
    .filter((path) => path.length > 0)
    .map((path) => `- ${path}`)
    .join("\n");
  return [
    promptT("worktree.intro"),
    "",
    list || `- ${promptT("worktree.missing")}`,
    "",
    promptT("worktree.steps"),
    promptT("worktree.step1"),
    promptT("worktree.step2"),
    promptT("worktree.step3"),
    promptT("worktree.step4"),
  ].join("\n");
}

async function restoreIsolationAfterAiMerge(sessionId: string): Promise<void> {
  try {
    await restoreSessionWorktree(sessionId);
  } catch {
    // 会话已结束时忽略，后续对话若恢复会按 working_dir 回到隔离树。
  }
  useGitStore.getState().bumpRevision();
}

export async function maybeFinishAiMergeResolve(input: {
  sessionId: string;
  workspaceId?: string | null;
}): Promise<boolean> {
  const store = useSessionStore.getState();
  if (!store.pendingAiMergeResolveBySession[input.sessionId]) return false;
  store.clearPendingAiMergeResolve(input.sessionId);

  await useWorkspaceStore.getState().refreshSessions();
  const session = useWorkspaceStore.getState().sessions.find((item) => item.id === input.sessionId);
  const workspaceId = input.workspaceId ?? session?.workspace_id ?? null;
  if (!workspaceId) {
    await restoreIsolationAfterAiMerge(input.sessionId);
    return true;
  }

  try {
    const result = await resolveSessionWorktreeMerge(workspaceId, input.sessionId, "complete");
    if (result.status === "resolved") {
      store.markWorktreeMerged(input.sessionId);
      return true;
    }
    store.openWorktreeMergePrompt({
      sessionId: input.sessionId,
      workspaceId,
      phase: "conflict",
      conflicts: result.conflicts,
      message: result.message,
    });
    return true;
  } catch {
    try {
      const state = await getWorktreeMergeState(workspaceId);
      if (state.in_progress) {
        store.openWorktreeMergePrompt({
          sessionId: input.sessionId,
          workspaceId,
          phase: "conflict",
          conflicts: state.conflicts,
        });
      }
    } catch {
      // 查不到合并状态时不再弹窗，避免挡住会话。
    }
    return true;
  } finally {
    await restoreIsolationAfterAiMerge(input.sessionId);
  }
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
