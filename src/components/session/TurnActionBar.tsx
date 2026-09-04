import { Check, Copy, RotateCcw, ThumbsDown, ThumbsUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { resolveComposerPlanMode } from "@/lib/planMode";
import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

const COPIED_MS = 2000;

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
  const { t } = useTranslation(["sessions", "common"]);
  const [vote, setVote] = useState<"up" | "down" | null>(null);
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number>(0);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const modelId = useChannelStore((state) => state.activeModelId);
  const defaultPlanMode = useUiStore((state) => state.composerPlanMode);
  const planModeBySession = useSessionStore((state) => state.planModeBySession);
  const planMode = resolveComposerPlanMode(sessionId, planModeBySession, defaultPlanMode);

  useEffect(() => {
    return () => window.clearTimeout(copiedTimer.current);
  }, []);

  const copy = async () => {
    await navigator.clipboard.writeText(assistantText);
    setCopied(true);
    window.clearTimeout(copiedTimer.current);
    copiedTimer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  const retry = () => {
    const prompt = userText?.trim();
    if (!prompt || working) return;
    if (!workspaceId || !channelId) return;
    void submitSessionPrompt({
      sessionId,
      workspaceId,
      channelId,
      prompt,
      model: modelId,
      planMode,
    });
  };

  return (
    <div className="flex items-center gap-1 text-muted-foreground">
      <button
        type="button"
        className="rounded p-1 hover:bg-muted"
        title={copied ? t("common:copied") : t("sessions:copy")}
        aria-label={copied ? t("common:copied") : t("sessions:copy")}
        onClick={() => void copy()}
      >
        {copied ? <Check className="size-3.5 text-emerald-500" /> : <Copy className="size-3.5" />}
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
