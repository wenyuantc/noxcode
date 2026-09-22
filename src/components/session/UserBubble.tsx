import { ArrowUp, Check, Copy, GitFork, Pencil, Undo2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  applyNativeFileRollback,
  applyNativeHistoryBoundary,
  previewNativeFileRollback,
} from "@/lib/backend";
import { fileRollbackChoices, summarizeRollbackPaths } from "@/lib/fileRollback";
import { submitSessionPrompt } from "@/lib/sessionSubmission";
import { resolveComposerPlanMode } from "@/lib/planMode";
import { resolveSessionSelection } from "@/lib/sessionModel";
import type { FileRollbackMode, FileRollbackPreview, NativeToolImage } from "@/lib/types";
import { cn, formatClockTime, formatDate } from "@/lib/utils";
import { SessionImageThumbs } from "./SessionImageThumbs";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

const COPIED_MS = 2000;

export function UserBubble({
  text,
  images,
  createdAt,
  sessionId,
  editable,
  working,
  boundary,
  onBranched,
}: {
  text: string;
  images?: NativeToolImage[];
  createdAt?: string;
  sessionId: string;
  editable: boolean;
  working: boolean;
  boundary?: {
    messageId: string;
    revision: number;
    selectableBefore: boolean;
    selectableAfter: boolean;
  };
  onBranched?: (sessionId: string) => void;
}) {
  const { t } = useTranslation(["sessions", "common"]);
  const [copied, setCopied] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(text);
  const [sending, setSending] = useState(false);
  const [branchError, setBranchError] = useState("");
  const [rollback, setRollback] = useState<FileRollbackPreview | null>(null);
  const [rollbackEdge, setRollbackEdge] = useState<"before" | "after">("after");
  const copiedTimer = useRef<number>(0);
  const branchInFlight = useRef(false);
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
    setDraft(text);
    setEditing(false);
  }, [text]);

  useEffect(() => {
    return () => window.clearTimeout(copiedTimer.current);
  }, []);

  const reloadBranchView = async (nextId: string) => {
    if (nextId === sessionId) {
      await useSessionStore.getState().refreshHistory(sessionId);
    }
    onBranched?.(nextId);
  };

  const loadRollback = async (edge: "before" | "after") => {
    if (!boundary || working || sending || archived || branchInFlight.current) return;
    branchInFlight.current = true;
    setBranchError("");
    setSending(true);
    setRollbackEdge(edge);
    try {
      const preview = await previewNativeFileRollback({
        session_record_id: sessionId,
        message_id: boundary.messageId,
        edge,
        mode: "both",
      });
      setRollback(preview);
    } catch (error) {
      setRollback(null);
      setBranchError(error instanceof Error ? error.message : String(error));
    } finally {
      branchInFlight.current = false;
      setSending(false);
    }
  };

  const confirmRollback = async (mode: FileRollbackMode) => {
    if (!boundary || !rollback || working || sending || archived || branchInFlight.current) return;
    if (!fileRollbackChoices(rollback)[mode]) return;
    branchInFlight.current = true;
    setBranchError("");
    setSending(true);
    try {
      const preview = await previewNativeFileRollback({
        session_record_id: sessionId,
        message_id: boundary.messageId,
        edge: rollbackEdge,
        mode,
      });
      const nextId = await applyNativeFileRollback({
        session_record_id: sessionId,
        message_id: boundary.messageId,
        edge: rollbackEdge,
        mode,
        expected_revision: preview.revision,
        token: preview.token,
        request_id: crypto.randomUUID(),
      });
      setRollback(null);
      await useWorkspaceStore.getState().refreshSessions();
      await reloadBranchView(nextId);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("不在当前分支") || message.includes("边界消息不存在")) {
        await useSessionStore.getState().refreshHistory(sessionId);
      }
      setBranchError(message);
    } finally {
      branchInFlight.current = false;
      setSending(false);
    }
  };

  const branchAt = async (action: "fork" | "rewind", edge: "before" | "after") => {
    if (!boundary || working || sending || archived || branchInFlight.current) return;
    branchInFlight.current = true;
    setBranchError("");
    setSending(true);
    try {
      const nextId = await applyNativeHistoryBoundary(action, {
        session_record_id: sessionId,
        message_id: boundary.messageId,
        edge,
        expected_revision: boundary.revision,
        request_id: crypto.randomUUID(),
      });
      await useWorkspaceStore.getState().refreshSessions();
      await reloadBranchView(nextId);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("不在当前分支") || message.includes("边界消息不存在")) {
        await useSessionStore.getState().refreshHistory(sessionId);
      }
      setBranchError(message);
    } finally {
      branchInFlight.current = false;
      setSending(false);
    }
  };

  const copy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    window.clearTimeout(copiedTimer.current);
    copiedTimer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  const resend = async () => {
    const prompt = draft.trim();
    if (!prompt || working || sending || archived) return;
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

  if (editing && !archived) {
    return (
      <div className="ml-auto flex w-full max-w-[80%] flex-col items-end gap-1.5">
        <SessionImageThumbs images={images} />
        <div className="w-full rounded-2xl border border-ring/40 bg-secondary/90 shadow-sm transition-all focus-within:ring-2 focus-within:ring-ring/20">
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
            className="w-full resize-none bg-transparent px-3.5 py-2.5 text-sm leading-relaxed outline-none"
          />
          <div className="flex items-center justify-end gap-1.5 px-3 pb-2.5">
            <button
              type="button"
              className="cursor-pointer rounded-lg p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
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
              className="flex size-7 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground shadow-2xs transition-all hover:opacity-90 disabled:opacity-40"
              title={t("sessions:send")}
              aria-label={t("sessions:send")}
              disabled={sending || working || !draft.trim()}
              onClick={() => void resend()}
            >
              <ArrowUp className="size-3.5" />
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="group ml-auto flex w-full max-w-[80%] flex-col items-end gap-1.5">
      <SessionImageThumbs images={images} />
      <div className="flex items-start justify-end gap-1.5">
        <div
          className={cn(
            "flex items-center pt-1 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100",
            copied && "opacity-100",
          )}
        >
          {createdAt && formatClockTime(createdAt) ? (
            <span
              className="px-1 font-mono text-meta text-muted-foreground/60 tabular-nums"
              title={formatDate(createdAt)}
            >
              {formatClockTime(createdAt)}
            </span>
          ) : null}
          <div className="inline-flex items-center gap-0.5 rounded-lg border border-border/40 bg-background/80 p-0.5 shadow-2xs backdrop-blur-xs">
            <button
              type="button"
              className="cursor-pointer rounded-md p-1 transition-colors hover:bg-muted hover:text-foreground"
              title={copied ? t("common:copied") : t("sessions:copy")}
              aria-label={copied ? t("common:copied") : t("sessions:copy")}
              onClick={() => void copy()}
            >
              {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
            </button>
            {editable && !archived ? (
              <button
                type="button"
                className="cursor-pointer rounded-md p-1 transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40"
                title={t("sessions:edit")}
                aria-label={t("sessions:edit")}
                disabled={working}
                onClick={() => setEditing(true)}
              >
                <Pencil className="size-3" />
              </button>
            ) : null}
          </div>
        </div>
        {text ? (
          <div className="rounded-2xl rounded-tr-xs border border-border/60 bg-secondary/80 px-3.5 py-2 text-sm leading-relaxed text-foreground shadow-2xs whitespace-pre-wrap select-text">
            {text}
          </div>
        ) : null}
      </div>
      {boundary ? (
        <div className="flex flex-wrap justify-end gap-1">
          <button
            type="button"
            className="inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            disabled={working || sending || archived || !boundary.selectableAfter}
            title={working ? t("sessions:branchWorking") : t("sessions:branchForkAfter")}
            onClick={() => void branchAt("fork", "after")}
          >
            <GitFork className="size-3" />
            {t("sessions:branchForkAfter")}
          </button>
          <button
            type="button"
            className="cursor-pointer rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            disabled={working || sending || archived || !boundary.selectableBefore}
            title={working ? t("sessions:branchWorking") : t("sessions:branchForkBefore")}
            onClick={() => void branchAt("fork", "before")}
          >
            {t("sessions:branchForkBefore")}
          </button>
          <button
            type="button"
            className="inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            disabled={working || sending || archived || !boundary.selectableAfter}
            title={working ? t("sessions:branchWorking") : t("sessions:branchRewindAfter")}
            onClick={() => void branchAt("rewind", "after")}
          >
            <Undo2 className="size-3" />
            {t("sessions:branchRewindAfter")}
          </button>
          <button
            type="button"
            className="cursor-pointer rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            disabled={working || sending || archived || !boundary.selectableBefore}
            title={working ? t("sessions:branchWorking") : t("sessions:branchRewindBefore")}
            onClick={() => void branchAt("rewind", "before")}
          >
            {t("sessions:branchRewindBefore")}
          </button>
          <button
            type="button"
            className="cursor-pointer rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            disabled={
              working ||
              sending ||
              archived ||
              (!boundary.selectableAfter && !boundary.selectableBefore)
            }
            title={working ? t("sessions:branchWorking") : t("sessions:fileRollback")}
            onClick={() => void loadRollback(boundary.selectableAfter ? "after" : "before")}
          >
            {t("sessions:fileRollback")}
          </button>
        </div>
      ) : null}
      {rollback ? (
        <div className="w-full max-w-sm rounded-lg border border-border/60 bg-background/90 p-2 text-left text-meta text-muted-foreground">
          <div className="mb-1 flex flex-wrap gap-1">
            {boundary?.selectableAfter ? (
              <button
                type="button"
                className={cn(
                  "cursor-pointer rounded-md px-1.5 py-0.5 hover:bg-muted hover:text-foreground",
                  rollbackEdge === "after" && "bg-muted text-foreground",
                )}
                disabled={sending}
                onClick={() => void loadRollback("after")}
              >
                {t("sessions:fileRollbackEdgeAfter")}
              </button>
            ) : null}
            {boundary?.selectableBefore ? (
              <button
                type="button"
                className={cn(
                  "cursor-pointer rounded-md px-1.5 py-0.5 hover:bg-muted hover:text-foreground",
                  rollbackEdge === "before" && "bg-muted text-foreground",
                )}
                disabled={sending}
                onClick={() => void loadRollback("before")}
              >
                {t("sessions:fileRollbackEdgeBefore")}
              </button>
            ) : null}
          </div>
          {rollback.unavailable_reason ? <p>{rollback.unavailable_reason}</p> : null}
          {[
            summarizeRollbackPaths(t("sessions:fileRollbackAdded"), rollback.added),
            summarizeRollbackPaths(t("sessions:fileRollbackModified"), rollback.modified),
            summarizeRollbackPaths(t("sessions:fileRollbackDeleted"), rollback.deleted),
            summarizeRollbackPaths(t("sessions:fileRollbackConflicts"), rollback.conflicts),
            summarizeRollbackPaths(t("sessions:fileRollbackUnsupported"), rollback.unsupported),
          ]
            .filter(Boolean)
            .map((line) => (
              <p key={line}>{line}</p>
            ))}
          {rollback.added.length +
            rollback.modified.length +
            rollback.deleted.length +
            rollback.conflicts.length +
            rollback.unsupported.length ===
          0 ? (
            <p>{t("sessions:fileRollbackEmpty")}</p>
          ) : null}
          {rollback.side_effects.map((line) => (
            <p key={line}>{line}</p>
          ))}
          <div className="mt-1 flex flex-wrap justify-end gap-1">
            {(["conversation", "files", "both"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                className="cursor-pointer rounded-md px-1.5 py-0.5 hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                disabled={sending || working || archived || !fileRollbackChoices(rollback)[mode]}
                onClick={() => void confirmRollback(mode)}
              >
                {t(
                  mode === "conversation"
                    ? "sessions:fileRollbackConversation"
                    : mode === "files"
                      ? "sessions:fileRollbackFiles"
                      : "sessions:fileRollbackBoth",
                )}
              </button>
            ))}
            <button
              type="button"
              className="cursor-pointer rounded-md px-1.5 py-0.5 hover:bg-muted hover:text-foreground"
              onClick={() => setRollback(null)}
            >
              {t("sessions:fileRollbackClose")}
            </button>
          </div>
        </div>
      ) : null}
      {branchError ? <p className="text-meta text-destructive">{branchError}</p> : null}
    </div>
  );
}
