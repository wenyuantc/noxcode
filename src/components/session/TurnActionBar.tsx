import { Copy, RotateCcw, ThumbsDown, ThumbsUp } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { sendNativeInput, startNativeSession } from "@/lib/backend";
import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

function formatClock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

export function TurnActionBar({
  sessionId,
  userText,
  assistantText,
  endedAt,
  working,
}: {
  sessionId: string;
  userText?: string;
  assistantText: string;
  endedAt: string;
  working?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const [vote, setVote] = useState<"up" | "down" | null>(null);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const modelId = useChannelStore((state) => state.activeModelId);
  const planMode = useUiStore((state) => state.composerPlanMode);
  const live = useSessionStore((state) => state.liveBySession[sessionId]);

  const retry = () => {
    const prompt = userText?.trim();
    if (!prompt || working) return;
    if (live && live.session_record_id === sessionId) {
      void sendNativeInput(sessionId, prompt);
      return;
    }
    if (!workspaceId || !channelId) return;
    void startNativeSession({
      ai_channel_id: channelId,
      workspace_id: workspaceId,
      prompt,
      model: modelId,
      plan_mode: planMode,
    });
  };

  return (
    <div className="flex items-center gap-1 text-muted-foreground">
      <button
        type="button"
        className="rounded p-1 hover:bg-muted"
        title={t("copy")}
        onClick={() => void navigator.clipboard.writeText(assistantText)}
      >
        <Copy className="size-3.5" />
      </button>
      <button
        type="button"
        className={cn("rounded p-1 hover:bg-muted", vote === "up" && "text-foreground")}
        title={t("like")}
        onClick={() => setVote((value) => (value === "up" ? null : "up"))}
      >
        <ThumbsUp className="size-3.5" />
      </button>
      <button
        type="button"
        className={cn("rounded p-1 hover:bg-muted", vote === "down" && "text-foreground")}
        title={t("dislike")}
        onClick={() => setVote((value) => (value === "down" ? null : "down"))}
      >
        <ThumbsDown className="size-3.5" />
      </button>
      <button
        type="button"
        className="rounded p-1 hover:bg-muted disabled:opacity-40"
        title={t("retry")}
        disabled={working || !userText?.trim()}
        onClick={retry}
      >
        <RotateCcw className="size-3.5" />
      </button>
      <span className="ml-1 text-[11px]">{formatClock(endedAt)}</span>
    </div>
  );
}
