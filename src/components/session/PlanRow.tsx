import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronUp,
  ClipboardList,
  Compass,
  Copy,
  Loader2,
  MessageSquarePlus,
  Play,
  Undo2,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
import {
  authorizedPlanRetry,
  parseApprovedPlan,
  parsePendingPlan,
  submitPlanApproval,
} from "@/lib/planApproval";
import {
  planApprovalModelArgs,
  resolvePlanApprovalThinking,
  resolveSessionSelection,
} from "@/lib/sessionModel";
import type { GroupedSessionItem, PlanLineStatus } from "@/lib/sessionLines";
import { parsePlanLine, planTitleFromBody } from "@/lib/sessionLines";
import type { NativePlanApprovalRequest } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { AssistantMarkdown } from "./AssistantMarkdown";
import { ChannelModelPicker } from "./ChannelModelPicker";
import { ThinkingLevelPicker } from "./ThinkingLevelPicker";

export function PlanPillButton({
  children,
  disabled,
  onClick,
}: {
  children: ReactNode;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="inline-flex h-8 items-center justify-center rounded-lg bg-primary px-4 text-xs font-medium text-primary-foreground shadow-xs transition hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
    >
      {children}
    </button>
  );
}

function statusLabel(
  t: (key: string, options?: Record<string, string>) => string,
  status: PlanLineStatus | null,
  body: string,
  questionSummary: string | null,
): string {
  switch (status) {
    case "entered":
      return t("planEntered");
    case "waiting_approval":
      return t("planWaitingApproval");
    case "waiting_question":
      return questionSummary
        ? t("planWaitingQuestionDetail", { summary: questionSummary })
        : t("planWaitingQuestion");
    case "execute":
      return t("planStartExecute");
    default:
      return body || t("planDocument");
  }
}

function cleanPlanBody(body: string, title?: string | null): string {
  if (!body) return "";
  const trimmed = body.trim();
  if (title) {
    const match = trimmed.match(/^#{1,6}\s+(.+?)(?:\r?\n|$)/);
    if (match && match[1]?.trim() === title.trim()) {
      const rest = trimmed.slice(match[0].length).trim();
      return rest.length > 0 ? rest : trimmed;
    }
  }
  return trimmed;
}

function isLongContent(text: string): boolean {
  const lines = text.split("\n").length;
  return lines > 14 || text.length > 600;
}

/**
 * 当前待批准的计划：优先用 live 会话的挂起请求；会话已结束时回落到落库的快照，
 * 这样停止或重开应用后仍能继续实施。
 */
function usePlanApproval(sessionId: string): NativePlanApprovalRequest | undefined {
  const live = useSessionStore((state) => Object.values(state.planApprovals[sessionId] ?? {})[0]);
  const isLive = useSessionStore((state) => Boolean(state.liveBySession[sessionId]));
  const session = useWorkspaceStore((state) =>
    state.sessions.find((item) => item.id === sessionId),
  );
  const persisted = useMemo(() => parsePendingPlan(session) ?? undefined, [session]);
  if (live) return live;
  return isLive ? undefined : persisted;
}

export function PlanRow({ item, sessionId }: { item: GroupedSessionItem; sessionId: string }) {
  const { t } = useTranslation("sessions");
  const parsed = parsePlanLine(item.text);
  const pendingApproval = usePlanApproval(sessionId);
  const planSession = useWorkspaceStore((state) =>
    state.sessions.find((session) => session.id === sessionId),
  );
  const savedPlan = useMemo(() => parseApprovedPlan(planSession), [planSession]);
  const planPath =
    savedPlan &&
    savedPlan.cwd_resolved !== false &&
    (!savedPlan.saved_path || savedPlan.saved_path === savedPlan.path) &&
    savedPlan.body.trim() === parsed?.body.trim() &&
    savedPlan.saved_hash === savedPlan.content_hash
      ? savedPlan.path
      : null;
  const pendingAsk = useSessionStore(
    (state) => Object.values(state.planQuestions[sessionId] ?? {})[0],
  );
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const copyTimerRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    return () => window.clearTimeout(copyTimerRef.current);
  }, []);

  if (!parsed) return null;

  if (parsed.kind === "status") {
    if (parsed.status === "waiting_question" && pendingAsk) return null;
    if (parsed.status === "waiting_approval" && pendingApproval) return null;
    return (
      <div className="flex items-center gap-2 py-0.5 text-xs text-muted-foreground">
        <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-cyan-500/10 text-cyan-600 dark:text-cyan-400">
          <ClipboardList className="size-3" />
        </span>
        <span className="font-medium">
          {statusLabel(t, parsed.status, parsed.body, parsed.questionSummary)}
        </span>
      </div>
    );
  }

  if (pendingApproval && parsed.body.trim() === pendingApproval.plan.trim()) return null;

  const title = parsed.title ?? t("planDocument");
  const cleanBody = cleanPlanBody(parsed.body, title);
  const isLong = isLongContent(cleanBody || parsed.body);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(parsed.body);
      setCopied(true);
      window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  };

  return (
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/85 text-card-foreground shadow-xs backdrop-blur-md transition-all dark:border-border/60 dark:bg-card/50">
      <div className="flex items-center justify-between border-b border-border/50 bg-muted/20 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-cyan-500/10 text-cyan-600 dark:text-cyan-400">
            <Compass className="size-4" strokeWidth={2} />
          </div>
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-semibold tracking-tight text-foreground">
              {title}
            </span>
            <Badge
              variant="secondary"
              className="h-5 shrink-0 px-1.5 text-[11px] font-medium text-muted-foreground"
            >
              {t("planHistoricalBadge")}
            </Badge>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={handleCopy}
            className="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            title={copied ? t("planCopied") : t("planCopy")}
            aria-label={copied ? t("planCopied") : t("planCopy")}
          >
            {copied ? (
              <Check className="size-3.5 text-emerald-500" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </button>
        </div>
      </div>

      <div className="p-4">
        {planPath ? (
          <p className="mb-3 break-all text-xs text-muted-foreground">{planPath}</p>
        ) : null}
        <div className={cn(!expanded && isLong && "relative max-h-[360px] overflow-hidden")}>
          <AssistantMarkdown text={cleanBody || parsed.body} variant="plan" />
          {!expanded && isLong ? (
            <div className="pointer-events-none absolute bottom-0 left-0 right-0 flex h-24 items-end justify-center bg-gradient-to-t from-card via-card/85 to-transparent pb-2">
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="pointer-events-auto inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background/95 px-3.5 py-1 text-xs font-medium text-foreground shadow-xs backdrop-blur-sm transition-all hover:bg-muted"
              >
                <span>{t("planExpand")}</span>
                <ChevronDown className="size-3 text-muted-foreground" />
              </button>
            </div>
          ) : null}
        </div>
        {expanded && isLong ? (
          <div className="mt-3 flex justify-center border-t border-border/40 pt-2">
            <button
              type="button"
              onClick={() => setExpanded(false)}
              className="inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              <span>{t("planCollapse")}</span>
              <ChevronUp className="size-3" />
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function PendingPlanApproval({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const pendingApproval = usePlanApproval(sessionId);
  const runtime = useSessionStore((state) => state.configurationBySession[sessionId]);
  const session = useWorkspaceStore((state) =>
    state.sessions.find((item) => item.id === sessionId),
  );
  const savedPlan = useMemo(() => parseApprovedPlan(session), [session]);
  const retryPlan = authorizedPlanRetry(pendingApproval, savedPlan);
  const channels = useChannelStore((state) => state.channels);
  const activeChannelId = useChannelStore((state) => state.activeChannelId);
  const activeModelId = useChannelStore((state) => state.activeModelId);
  const setChannelSelection = useChannelStore((state) => state.setSelection);
  const composerThinkingLevel = useUiStore((state) => state.composerThinkingLevel);
  const setComposerThinkingLevel = useUiStore((state) => state.setComposerThinkingLevel);
  const defaultSelection = retryPlan
    ? { channelId: retryPlan.ai_channel_id, modelId: retryPlan.model }
    : resolveSessionSelection({
        sessionId,
        runtime,
        session,
        fallbackChannelId: activeChannelId,
        fallbackModelId: activeModelId,
      });
  const [feedback, setFeedback] = useState(retryPlan?.feedback ?? "");
  const [showFeedback, setShowFeedback] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [selection, setSelection] = useState(defaultSelection);
  const [thinkingLevel, setThinkingLevel] = useState(
    () =>
      resolvePlanApprovalThinking({
        channels,
        selection: defaultSelection,
        preferredEffort:
          retryPlan?.reasoning_effort ?? runtime?.reasoning_effort ?? composerThinkingLevel,
      }).effort,
  );
  const copyTimerRef = useRef<number | undefined>(undefined);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const defaultChannelId = defaultSelection.channelId;
  const defaultModelId = defaultSelection.modelId;

  useEffect(() => {
    setFeedback(retryPlan?.feedback ?? "");
    setShowFeedback(false);
    setError(null);
    setExpanded(false);
    setSelection({
      channelId: retryPlan?.ai_channel_id ?? defaultChannelId,
      modelId: retryPlan?.model ?? defaultModelId,
    });
  }, [
    pendingApproval?.request_id,
    defaultChannelId,
    defaultModelId,
    retryPlan?.feedback,
    retryPlan?.ai_channel_id,
    retryPlan?.model,
  ]);

  useEffect(() => {
    setThinkingLevel(
      resolvePlanApprovalThinking({
        channels,
        selection: { channelId: defaultChannelId, modelId: defaultModelId },
        preferredEffort:
          retryPlan?.reasoning_effort ?? runtime?.reasoning_effort ?? composerThinkingLevel,
      }).effort,
    );
  }, [
    pendingApproval?.request_id,
    defaultChannelId,
    defaultModelId,
    channels,
    runtime?.reasoning_effort,
    retryPlan?.reasoning_effort,
    composerThinkingLevel,
  ]);

  useEffect(() => {
    return () => window.clearTimeout(copyTimerRef.current);
  }, []);

  if (!pendingApproval) return null;

  const planText = pendingApproval.plan;
  const title = planTitleFromBody(planText) ?? t("planDocument");
  const cleanBody = cleanPlanBody(planText, title);
  const isLong = isLongContent(cleanBody || planText);
  const thinking = resolvePlanApprovalThinking({
    channels,
    selection,
    preferredEffort: thinkingLevel,
  });

  const canRetry = Boolean(
    retryPlan &&
    retryPlan.feedback === feedback.trim() &&
    retryPlan.ai_channel_id === selection.channelId &&
    retryPlan.model === selection.modelId &&
    (!thinking.enabled || retryPlan.reasoning_effort === thinking.effort),
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(planText);
      setCopied(true);
      window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  };

  const resolve = async (approved: boolean) => {
    if (busy) return;
    const current = pendingApproval;
    const modelArgs = planApprovalModelArgs(
      approved,
      selection,
      thinking.enabled ? thinking.effort : null,
      Boolean(current.detached),
    );
    setBusy(true);
    setError(null);
    try {
      await resolveSessionRequest({ ...current, kind: "plan_approval" }, async () => {
        const started = await submitPlanApproval(current, approved, feedback, modelArgs);
        if (started) useSessionStore.getState().onStarted(started);
      });
      if (approved && modelArgs.aiChannelId && modelArgs.model) {
        setChannelSelection(modelArgs.aiChannelId, modelArgs.model);
      }
      if (approved && modelArgs.reasoningEffort) {
        setComposerThinkingLevel(modelArgs.reasoningEffort);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      await useWorkspaceStore
        .getState()
        .refreshSessions()
        .catch(() => undefined);
      setBusy(false);
    }
  };

  const handleReject = () => {
    if (!showFeedback && !feedback.trim()) {
      setShowFeedback(true);
      setTimeout(() => textareaRef.current?.focus(), 50);
      return;
    }
    // detached 退回要靠反馈文本组成续聊指令，空反馈无法发起新一轮。
    if (pendingApproval.detached && !feedback.trim()) {
      setError(t("planContinueNeedFeedback"));
      textareaRef.current?.focus();
      return;
    }
    void resolve(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void resolve(true);
    }
  };

  return (
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/85 text-card-foreground shadow-xs backdrop-blur-md transition-all dark:border-border/60 dark:bg-card/50">
      <div className="h-0.5 w-full bg-gradient-to-r from-cyan-500 via-primary/50 to-cyan-500/20" />

      <div className="flex items-center justify-between border-b border-border/50 bg-muted/20 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-cyan-500/10 text-cyan-600 dark:text-cyan-400">
            <Compass className="size-4" strokeWidth={2} />
          </div>
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-semibold tracking-tight text-foreground">
              {title}
            </span>
            <Badge
              variant="outline"
              className="h-5 shrink-0 gap-1 border-amber-500/40 bg-amber-500/10 px-1.5 text-[11px] font-medium text-amber-600 dark:text-amber-400"
            >
              <span
                className={cn(
                  "size-1.5 rounded-full bg-amber-500",
                  !pendingApproval.detached && "animate-pulse",
                )}
              />
              <span>
                {pendingApproval.detached ? t("planContinueBadge") : t("planWaitingApproval")}
              </span>
            </Badge>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={handleCopy}
            className="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            title={copied ? t("planCopied") : t("planCopy")}
            aria-label={copied ? t("planCopied") : t("planCopy")}
          >
            {copied ? (
              <Check className="size-3.5 text-emerald-500" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </button>
        </div>
      </div>

      <div className="p-4">
        <div className={cn(!expanded && isLong && "relative max-h-[360px] overflow-hidden")}>
          <AssistantMarkdown text={cleanBody || planText} variant="plan" />
          {!expanded && isLong ? (
            <div className="pointer-events-none absolute bottom-0 left-0 right-0 flex h-24 items-end justify-center bg-gradient-to-t from-card via-card/85 to-transparent pb-2">
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="pointer-events-auto inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background/95 px-3.5 py-1 text-xs font-medium text-foreground shadow-xs backdrop-blur-sm transition-all hover:bg-muted"
              >
                <span>{t("planExpand")}</span>
                <ChevronDown className="size-3 text-muted-foreground" />
              </button>
            </div>
          ) : null}
        </div>
        {expanded && isLong ? (
          <div className="mt-3 flex justify-center border-t border-border/40 pt-2">
            <button
              type="button"
              onClick={() => setExpanded(false)}
              className="inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              <span>{t("planCollapse")}</span>
              <ChevronUp className="size-3" />
            </button>
          </div>
        ) : null}

        {pendingApproval.detached ? (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-border/60 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
            <AlertCircle className="size-4 shrink-0" />
            <span>{t("planContinueHint")}</span>
          </div>
        ) : null}

        {retryPlan?.path ? (
          <p className="mt-3 break-all text-xs text-muted-foreground">{retryPlan.path}</p>
        ) : null}
        {error || retryPlan?.error ? (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            <AlertCircle className="size-4 shrink-0" />
            <span>{error ?? retryPlan?.error}</span>
          </div>
        ) : null}

        {showFeedback ? (
          <div className="mt-3 space-y-1.5 rounded-xl border border-border/60 bg-muted/20 p-2.5">
            <div className="flex items-center justify-between px-1 text-xs text-muted-foreground">
              <span className="font-medium text-foreground/90">{t("planAddFeedback")}</span>
              <span className="text-[11px] text-muted-foreground/70">
                ⌘/Ctrl + Enter {t("planApprovalApprove")}
              </span>
            </div>
            <Textarea
              ref={textareaRef}
              value={feedback}
              rows={3}
              placeholder={t("planApprovalFeedbackPlaceholder")}
              disabled={busy}
              onChange={(event) => setFeedback(event.target.value)}
              onKeyDown={handleKeyDown}
              className="min-h-[70px] resize-y bg-background/70 text-xs"
            />
          </div>
        ) : null}

        <div className="mt-3 flex items-center justify-between gap-2 border-t border-border/50 pt-3">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              const next = !showFeedback;
              setShowFeedback(next);
              if (next) {
                setTimeout(() => textareaRef.current?.focus(), 50);
              }
            }}
            className="h-8 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
          >
            <MessageSquarePlus className="size-3.5" />
            <span>{showFeedback ? t("planHideFeedback") : t("planAddFeedback")}</span>
            {feedback.trim().length > 0 ? (
              <span className="size-1.5 rounded-full bg-cyan-500" />
            ) : null}
          </Button>

          <div className="flex min-w-0 items-center gap-2">
            <ChannelModelPicker
              disabled={busy}
              onError={setError}
              selection={selection}
              onSelectionChange={(channelId, modelId) => setSelection({ channelId, modelId })}
              className="max-w-[min(16rem,40vw)]"
            />
            {thinking.enabled ? (
              <ThinkingLevelPicker
                value={thinking.effort}
                levels={thinking.levels}
                disabled={busy}
                onChange={setThinkingLevel}
              />
            ) : null}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={handleReject}
              className="h-8 gap-1.5 text-xs border-border/80 hover:border-destructive/30 hover:bg-destructive/10 hover:text-destructive"
            >
              <Undo2 className="size-3.5" />
              <span>{t("planApprovalReject")}</span>
            </Button>
            <Button
              type="button"
              variant="default"
              size="sm"
              disabled={busy}
              onClick={() => void resolve(true)}
              className="h-8 gap-1.5 text-xs font-medium bg-primary text-primary-foreground shadow-xs hover:bg-primary/90"
            >
              {busy ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Play className="size-3.5 fill-current" />
              )}
              <span>{canRetry ? t("planApprovalRetry") : t("planApprovalApprove")}</span>
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
