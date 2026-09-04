import { useEffect } from "react";

import {
  onNativeContextUsage,
  onNativeExit,
  onNativePermissionRequest,
  onNativePlanMode,
  onNativePlanApprovalRequest,
  onNativePlanQuestion,
  onNativeSession,
  onNativeStdout,
  onNativeTextDelta,
  onNativeTurnState,
} from "@/lib/backend";
import { useSessionStore } from "@/stores/sessionStore";
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
    track(onNativeStdout((output) => useSessionStore.getState().onStdout(output)));
    track(onNativeTextDelta((delta) => useSessionStore.getState().onDelta(delta)));
    track(onNativeContextUsage((usage) => useSessionStore.getState().onUsage(usage)));
    track(
      onNativeTurnState((payload) =>
        useSessionStore.getState().onTurnState(payload.session_record_id, payload.state),
      ),
    );
    track(
      onNativePlanMode((payload) =>
        useSessionStore.getState().onPlanMode(payload.session_record_id, payload.plan_mode),
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
