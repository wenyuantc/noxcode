import { GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";

import { prepareAgentSessionResume, resumeNativeSession } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { WorkspacePicker } from "./WorkspacePicker";

export function SessionHeader() {
  const { t } = useTranslation("sessions");
  const selected = useSessionStore((state) => state.selectedSessionId);
  const liveByWorkspace = useSessionStore((state) => state.liveByWorkspace);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const modelId = useChannelStore((state) => state.activeModelId);
  const toggleGit = useUiStore((state) => state.toggleGit);
  const live = workspaceId ? liveByWorkspace[workspaceId] : undefined;
  const canResume = Boolean(selected && workspaceId && channelId && !live);

  return (
    <div className="flex items-center gap-2 border-b px-4 py-2">
      <WorkspacePicker />
      <BranchPicker />
      <span className="flex-1" />
      {canResume ? (
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            if (!selected || !workspaceId || !channelId) return;
            void prepareAgentSessionResume(selected).then((info) => {
              if (!info.resumable) return;
              return resumeNativeSession(
                {
                  ai_channel_id: channelId,
                  workspace_id: workspaceId,
                  prompt: "继续",
                  model: modelId,
                  resume_session_id: selected,
                },
                selected,
              );
            });
          }}
        >
          {t("resume")}
        </Button>
      ) : null}
      <Button size="sm" variant="ghost" onClick={toggleGit}>
        <GitBranch className="size-4" />
        Git
      </Button>
    </div>
  );
}
