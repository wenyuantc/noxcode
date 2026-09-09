import { lazy, Suspense, useEffect } from "react";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { MergeWorktreeDialog } from "@/components/session/MergeWorktreeDialog";
import { NativePermissionDialog } from "@/components/session/NativePermissionDialog";
import { SshHostTrustDialog } from "@/components/ssh/SshHostTrustDialog";
import { useNativeEvents } from "@/hooks/useNativeEvents";
import { watchSystemTheme } from "@/lib/theme";
import { mergeWorktreeDialogKey } from "@/lib/worktreeMergePrompt";
import WorkspacePage from "@/pages/WorkspacePage";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useUpdateStore } from "@/stores/updateStore";

const ApiCallLogsPage = lazy(() => import("@/pages/ApiCallLogsPage"));
const SettingsPage = lazy(() => import("@/pages/SettingsPage"));

function AppEffects() {
  const mergeDialogKey = useSessionStore((state) =>
    mergeWorktreeDialogKey(state.worktreeMergePrompt),
  );
  useNativeEvents();
  useEffect(() => {
    void useUpdateStore.getState().checkOnStartup();
  }, []);
  useEffect(() => {
    return watchSystemTheme(
      () => useUiStore.getState().theme,
      (isDark) => useUiStore.getState().setIsDark(isDark),
    );
  }, []);
  return (
    <>
      <NativePermissionDialog />
      <MergeWorktreeDialog key={mergeDialogKey} />
      <SshHostTrustDialog />
    </>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <AppEffects />
      <Suspense
        fallback={
          <div
            role="status"
            className="flex h-screen items-center justify-center text-sm text-muted-foreground"
          >
            加载中...
          </div>
        }
      >
        <Routes>
          <Route path="/" element={<WorkspacePage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/settings/:section" element={<SettingsPage />} />
          <Route path="/api-logs" element={<ApiCallLogsPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </Suspense>
    </BrowserRouter>
  );
}
