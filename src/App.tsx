import { useEffect } from "react";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { NativePermissionDialog } from "@/components/session/NativePermissionDialog";
import { SshHostTrustDialog } from "@/components/ssh/SshHostTrustDialog";
import { useNativeEvents } from "@/hooks/useNativeEvents";
import ApiCallLogsPage from "@/pages/ApiCallLogsPage";
import SettingsPage from "@/pages/SettingsPage";
import WorkspacePage from "@/pages/WorkspacePage";
import { useUpdateStore } from "@/stores/updateStore";

function AppEffects() {
  useNativeEvents();
  useEffect(() => {
    void useUpdateStore.getState().checkOnStartup();
  }, []);
  return (
    <>
      <NativePermissionDialog />
      <SshHostTrustDialog />
    </>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <AppEffects />
      <Routes>
        <Route path="/" element={<WorkspacePage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="/settings/:section" element={<SettingsPage />} />
        <Route path="/api-logs" element={<ApiCallLogsPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
