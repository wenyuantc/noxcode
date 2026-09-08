import { useEffect } from "react";

import {
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
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

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
