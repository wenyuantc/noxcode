import { Check, Copy, RotateCcw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { resolveComposerPlanMode } from "@/lib/planMode";
import { resolveSessionSelection } from "@/lib/sessionModel";
import type { ParsedUsage } from "@/lib/sessionLines";
import { cn, formatClockTime, formatDate } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { UsageChips } from "./UsageRow";

const COPIED_MS = 2000;

export function TurnFooter({
  sessionId,
  userText,
  assistantText,
  usage,
  endedAt,
  working,
}: {
  sessionId: string;
  userText?: string;
  assistantText: string;
  usage?: ParsedUsage | null;
  endedAt: string;
  working?: boolean;
}) {
  const { t } = useTranslation(["sessions", "common"]);
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number>(0);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const session = useWorkspaceStore((state) =>
    state.sessions.find((item) => item.id === sessionId),
  );
  const archived = Boolean(session?.archived);
  const fallbackChannelId = useChannelStore((state) => state.activeChannelId);
  const fallbackModelId = useChannelStore((state) => state.activeModelId);
  const runtime = useSessionStore((state) => state.configurationBySession[sessionId]);
  const { channelId, modelId } = resolveSessionSelection({
    sessionId,
    runtime,
    session,
    fallbackChannelId,
    fallbackModelId,
  });
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
    if (!prompt || working || archived) return;
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

  const clock = formatClockTime(endedAt);

  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 pt-1 text-muted-foreground">
      {assistantText ? (
        <div className="inline-flex items-center gap-0.5 rounded-lg border border-border/40 bg-muted/20 p-0.5 shadow-2xs backdrop-blur-xs">
          <button
            type="button"
            className="cursor-pointer rounded-md p-1 transition-colors hover:bg-muted hover:text-foreground"
            title={copied ? t("common:copied") : t("sessions:copy")}
            aria-label={copied ? t("common:copied") : t("sessions:copy")}
            onClick={() => void copy()}
          >
            {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
          </button>
          <button
            type="button"
            className="cursor-pointer rounded-md p-1 transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40"
            title={t("retry")}
            aria-label={t("retry")}
            disabled={working || archived || !userText?.trim()}
            onClick={retry}
          >
            <RotateCcw className="size-3" />
          </button>
        </div>
      ) : null}
      {usage ? <UsageChips usage={usage} className="py-0" /> : null}
      {clock ? (
        <span
          className={cn(
            "font-mono text-meta text-muted-foreground/60 tabular-nums",
            (assistantText || usage) && "ml-auto",
          )}
          title={formatDate(endedAt)}
        >
          {clock}
        </span>
      ) : null}
    </div>
  );
}
