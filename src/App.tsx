import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { NativePermissionDialog } from "@/components/session/NativePermissionDialog";
import { NativePlanQuestionDialog } from "@/components/session/NativePlanQuestionDialog";
import { SshHostTrustDialog } from "@/components/ssh/SshHostTrustDialog";
import { useNativeEvents } from "@/hooks/useNativeEvents";
import ApiCallLogsPage from "@/pages/ApiCallLogsPage";
import SettingsPage from "@/pages/SettingsPage";
import WorkspacePage from "@/pages/WorkspacePage";

function AppEffects() {
  useNativeEvents();
  return (
    <>
      <NativePermissionDialog />
      <NativePlanQuestionDialog />
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
