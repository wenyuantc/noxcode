import { useEffect } from "react";

import {
  getWorktreeMergeState,
  onNativeContextUsage,
  onNativeExit,
  onNativePermissionRequest,
  onNativePlanMode,
  onNativePlanApprovalRequest,
  onNativePlanQuestion,
  onNativeSession,
  onNativeSessionConfiguration,
  onNativeSessionTitle,
  onNativeStdout,
  onNativeTextDelta,
  onNativeTurnState,
  onNativeBackgroundTasks,
  onNativeBackgroundProcesses,
  onNativeRequestResolved,
  onNativeInputQueue,
} from "@/lib/backend";
import { isManagedWorktreePath } from "@/lib/worktreePath";
import type { AgentSessionExit } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

async function maybeOpenWorktreeMerge(exit: AgentSessionExit) {
  const store = useSessionStore.getState();
  if (store.mergedWorktreeBySession[exit.session_record_id]) return;
  const session = useWorkspaceStore
    .getState()
    .sessions.find((item) => item.id === exit.session_record_id);
  const runtime = store.configurationBySession[exit.session_record_id];
  const workspaceId = exit.workspace_id ?? session?.workspace_id ?? null;
  const path = runtime?.worktree_path ?? session?.working_dir;
  if (!workspaceId || !isManagedWorktreePath(path, exit.session_record_id)) return;
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
    sessionId: exit.session_record_id,
    workspaceId,
    phase,
    conflicts,
  });
}

export function useNativeEvents() {
  useEffect(() => {
    const unlistens: Array<() => void> = [];
    let cancelled = false;

    const track = (promise: Promise<() => void>) => {
      void promise.then((fn) => {
        if (cancelled) fn();
        else unlistens.push(fn);
      });
    };

    const store = useSessionStore.getState();
    track(
      onNativeSession((session) => {
        store.onStarted(session);
        void useWorkspaceStore.getState().refreshSessions();
      }),
    );
    track(
      onNativeSessionTitle(() => {
        void useWorkspaceStore.getState().refreshSessions();
      }),
    );
    track(
      onNativeSessionConfiguration((payload) => {
        useSessionStore.getState().onConfiguration(payload);
        if (payload.runtime && !payload.error) {
          if (useSessionStore.getState().selectedSessionId === payload.session_record_id) {
            useChannelStore
              .getState()
              .setSelection(payload.runtime.ai_channel_id, payload.runtime.model);
            if (payload.runtime.reasoning_effort) {
              useUiStore.getState().setComposerThinkingLevel(payload.runtime.reasoning_effort);
            }
          }
          void useWorkspaceStore.getState().refreshSessions();
        }
      }),
    );
    track(onNativeStdout((output) => useSessionStore.getState().onStdout(output)));
    track(onNativeInputQueue((payload) => useSessionStore.getState().onInputQueue(payload)));
    track(
      onNativeBackgroundTasks((payload) => useSessionStore.getState().onBackgroundTasks(payload)),
    );
    track(
      onNativeBackgroundProcesses((payload) =>
        useSessionStore.getState().onBackgroundProcesses(payload),
      ),
    );
    track(onNativeRequestResolved((payload) => useSessionStore.getState().resolveRequest(payload)));
    track(onNativeTextDelta((delta) => useSessionStore.getState().onDelta(delta)));
    track(onNativeContextUsage((usage) => useSessionStore.getState().onUsage(usage)));
    track(
      onNativeTurnState((payload) =>
        useSessionStore.getState().onTurnState(payload.session_record_id, payload.state),
      ),
    );
    track(
      onNativePlanMode((payload) =>
        useSessionStore
          .getState()
          .onPlanMode(payload.session_record_id, payload.plan_mode, payload.input_queue_id),
      ),
    );
    track(
      onNativeExit((exit) => {
        useSessionStore.getState().onExit(exit);
        void useWorkspaceStore.getState().refreshSessions();
        void maybeOpenWorktreeMerge(exit);
      }),
    );
    track(
      onNativePermissionRequest((request) => useSessionStore.getState().setPermission(request)),
    );
    track(onNativePlanQuestion((request) => useSessionStore.getState().setPlanQuestion(request)));
    track(
      onNativePlanApprovalRequest((request) => useSessionStore.getState().setPlanApproval(request)),
    );

    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
    };
  }, []);
}
