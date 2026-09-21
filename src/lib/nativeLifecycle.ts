import type { AgentSessionExit, NativeTurnState } from "@/lib/types";
import { maybeFinishAiMergeResolve, maybeOpenWorktreeMerge } from "@/lib/worktreeMergePrompt";
import { useSessionStore } from "@/stores/sessionStore";
import { useSteerStore } from "@/stores/steerStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export async function handleNativeTurnState(event: NativeTurnState): Promise<void> {
  if (!useSessionStore.getState().onTurnState(event) || event.state !== "waiting_input") return;
  const isCurrent = () => useSteerStore.getState().isCurrentTurn(event);
  const session = useWorkspaceStore
    .getState()
    .sessions.find((item) => item.id === event.session_record_id);
  const runtime = useSessionStore.getState().configurationBySession[event.session_record_id];
  const finished = await maybeFinishAiMergeResolve({
    sessionId: event.session_record_id,
    workspaceId: session?.workspace_id,
    isCurrent,
  });
  if (finished || !isCurrent()) return;
  await maybeOpenWorktreeMerge({
    sessionId: event.session_record_id,
    workspaceId: session?.workspace_id,
    worktreePath: runtime?.worktree_path ?? session?.working_dir,
    reason: "turn",
    isCurrent,
  });
}

export async function handleNativeExit(event: AgentSessionExit): Promise<void> {
  if (!useSessionStore.getState().onExit(event)) return;
  const isCurrent = () =>
    useSteerStore.getState().isCurrentExit(event.session_record_id, event.instance_id) &&
    !useSessionStore.getState().liveBySession[event.session_record_id];
  await useWorkspaceStore.getState().refreshSessions();
  if (!isCurrent()) return;
  const finished = await maybeFinishAiMergeResolve({
    sessionId: event.session_record_id,
    workspaceId: event.workspace_id,
    isCurrent,
  });
  if (finished || !isCurrent()) return;
  await maybeOpenWorktreeMerge({
    sessionId: event.session_record_id,
    workspaceId: event.workspace_id,
    worktreePath: event.worktree_path,
    reason: "exit",
    isCurrent,
  });
}
