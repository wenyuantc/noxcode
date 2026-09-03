import { GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";

import { prepareAgentSessionResume, resumeNativeSession } from "@/lib/backend";
import { displaySessionTitle } from "@/lib/sessionLines";
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
  const live = useSessionStore((state) => (selected ? state.liveBySession[selected] : undefined));
  const sessions = useWorkspaceStore((state) => state.sessions);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const session = sessions.find((item) => item.id === selected);
  const title = displaySessionTitle(session?.title);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const modelId = useChannelStore((state) => state.activeModelId);
  const toggleGit = useUiStore((state) => state.toggleGit);
  const canResume = Boolean(selected && workspaceId && channelId && !live);

  return (
    <div className="flex items-center gap-2 border-b px-4 py-2">
      <WorkspacePicker />
      <BranchPicker />
      <span className="min-w-0 flex-1 truncate px-2 text-center text-sm text-muted-foreground">
        {title}
      </span>
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
