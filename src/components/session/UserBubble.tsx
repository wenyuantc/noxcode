import { ArrowUp, Check, Copy, Pencil, X } from "lucide-react";
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

export function UserBubble({
  text,
  sessionId,
  editable,
  working,
}: {
  text: string;
  sessionId: string;
  editable: boolean;
  working: boolean;
}) {
  const { t } = useTranslation(["sessions", "common"]);
  const [copied, setCopied] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(text);
  const [sending, setSending] = useState(false);
  const copiedTimer = useRef<number>(0);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const modelId = useChannelStore((state) => state.activeModelId);
  const defaultPlanMode = useUiStore((state) => state.composerPlanMode);
  const planModeBySession = useSessionStore((state) => state.planModeBySession);
  const planMode = resolveComposerPlanMode(sessionId, planModeBySession, defaultPlanMode);

  useEffect(() => {
    setDraft(text);
    setEditing(false);
  }, [text]);

  useEffect(() => {
    return () => window.clearTimeout(copiedTimer.current);
  }, []);

  const copy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    window.clearTimeout(copiedTimer.current);
    copiedTimer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  const resend = async () => {
    const prompt = draft.trim();
    if (!prompt || working || sending) return;
    setSending(true);
    try {
      if (!workspaceId || !channelId) return;
      await submitSessionPrompt({
        sessionId,
        workspaceId,
        channelId,
        prompt,
        model: modelId,
        planMode,
      });
      setEditing(false);
    } finally {
      setSending(false);
    }
  };

  if (editing) {
    return (
      <div className="ml-auto w-full max-w-[80%] rounded-xl border bg-secondary">
        <textarea
          value={draft}
          autoFocus
          rows={Math.min(8, Math.max(2, draft.split("\n").length))}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setDraft(text);
              setEditing(false);
            }
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void resend();
            }
          }}
          className="w-full resize-none bg-transparent px-3 py-2 text-sm outline-none"
        />
        <div className="flex items-center justify-end gap-1 px-2 pb-2">
          <button
            type="button"
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            title={t("common:cancel")}
            aria-label={t("common:cancel")}
            onClick={() => {
              setDraft(text);
              setEditing(false);
            }}
          >
            <X className="size-3.5" />
          </button>
          <button
            type="button"
            className="flex size-7 items-center justify-center rounded-full bg-foreground text-background disabled:opacity-40"
            title={t("sessions:send")}
            aria-label={t("sessions:send")}
            disabled={sending || working || !draft.trim()}
            onClick={() => void resend()}
          >
            <ArrowUp className="size-3.5" />
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="group flex items-start justify-end gap-1.5">
      <div className="max-w-[80%] rounded-xl bg-secondary px-3 py-1.5 text-sm whitespace-pre-wrap">
        {text}
      </div>
      <div
        className={cn(
          "flex items-center pt-0.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100",
          copied && "opacity-100",
        )}
      >
        <button
          type="button"
          className="rounded p-1 hover:bg-muted hover:text-foreground"
          title={copied ? t("common:copied") : t("sessions:copy")}
          aria-label={copied ? t("common:copied") : t("sessions:copy")}
          onClick={() => void copy()}
        >
          {copied ? <Check className="size-3.5 text-emerald-500" /> : <Copy className="size-3.5" />}
        </button>
        {editable ? (
          <button
            type="button"
            className="rounded p-1 hover:bg-muted hover:text-foreground disabled:opacity-40"
            title={t("sessions:edit")}
            aria-label={t("sessions:edit")}
            disabled={working}
            onClick={() => setEditing(true)}
          >
            <Pencil className="size-3.5" />
          </button>
        ) : null}
      </div>
    </div>
  );
}
