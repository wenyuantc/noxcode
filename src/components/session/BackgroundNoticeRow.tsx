import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronRight,
  Copy,
  MessageSquare,
  Square,
} from "lucide-react";
import { memo, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseBackgroundNotice, type ParsedBackgroundNoticeItem } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";

function previewSnippet(text: string): string {
  if (!text) return "";
  const cleaned = text
    .replace(/^#+\s+/gm, "")
    .replace(/\*\*/g, "")
    .replace(/`+/g, "")
    .split(/\n+/)
    .map((line) => line.trim())
    .filter(Boolean)
    .join(" · ");
  return cleaned.length > 90 ? `${cleaned.slice(0, 89)}…` : cleaned;
}

const MessageNoticeCard = memo(function MessageNoticeCard({
  notice,
  taskKind,
}: {
  notice: ParsedBackgroundNoticeItem;
  taskKind?: string;
}) {
  const { t } = useTranslation("sessions");
  const [copied, setCopied] = useState(false);

  const handleCopy = async (event: React.MouseEvent) => {
    event.stopPropagation();
    if (!notice.content) return;
    try {
      await navigator.clipboard.writeText(notice.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore clipboard error
    }
  };

  return (
    <div className="group my-2 overflow-hidden rounded-xl border border-indigo-500/25 bg-indigo-500/5 shadow-2xs backdrop-blur-xs transition-all duration-150 hover:border-indigo-500/40 dark:bg-indigo-500/[0.08]">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-2 px-3.5 py-2.5">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <div className="flex size-5 shrink-0 items-center justify-center rounded-md bg-indigo-500/15 text-indigo-600 dark:text-indigo-400">
            <MessageSquare className="size-3" />
          </div>

          <span className="shrink-0 rounded-md border border-indigo-500/30 bg-background/60 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-indigo-700 dark:text-indigo-300">
            #{notice.taskId}
          </span>

          {taskKind ? (
            <span className="shrink-0 rounded-md border border-indigo-500/20 bg-indigo-500/10 px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider text-indigo-600 dark:text-indigo-400">
              {taskKind}
            </span>
          ) : null}

          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            {notice.description || notice.taskId}
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <span className="rounded-md border border-indigo-500/30 bg-indigo-500/15 px-1.5 py-0.5 text-[10px] font-medium text-indigo-700 dark:text-indigo-300">
            {t("backgroundTaskMessage")}
          </span>

          <Button
            variant="ghost"
            size="icon"
            className="size-6 text-muted-foreground hover:text-foreground"
            title={copied ? t("backgroundTaskCopied") : t("copy")}
            aria-label={copied ? t("backgroundTaskCopied") : t("copy")}
            onClick={handleCopy}
          >
            {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
          </Button>
        </div>
      </div>

      {/* Content */}
      <div className="border-t border-indigo-500/15 bg-background/50 px-3.5 py-2.5 text-xs text-foreground/90">
        <AssistantMarkdown text={notice.content} />
      </div>
    </div>
  );
});

const DeliveryNoticeCard = memo(function DeliveryNoticeCard({
  notice,
  fullReport,
  taskKind,
}: {
  notice: ParsedBackgroundNoticeItem;
  fullReport?: string | null;
  taskKind?: string;
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  const isDone = notice.kind === "done";
  const isFailed = notice.kind === "failed";

  const effectiveReport = fullReport?.trim() || notice.content?.trim() || "";

  const handleCopy = async (event: React.MouseEvent) => {
    event.stopPropagation();
    if (!effectiveReport) return;
    try {
      await navigator.clipboard.writeText(effectiveReport);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore clipboard error
    }
  };

  const borderClass = isDone
    ? "border-emerald-500/30 hover:border-emerald-500/50 bg-emerald-500/5 dark:bg-emerald-500/[0.06]"
    : isFailed
      ? "border-rose-500/30 hover:border-rose-500/50 bg-rose-500/5 dark:bg-rose-500/[0.06]"
      : "border-border/70 hover:border-border bg-muted/20";

  const statusBadge = isDone
    ? {
        label: t("backgroundTaskCompleted"),
        class: "border-emerald-500/30 bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
      }
    : isFailed
      ? {
          label: t("backgroundTaskFailed"),
          class: "border-rose-500/30 bg-rose-500/15 text-rose-700 dark:text-rose-300",
        }
      : {
          label: t("backgroundTaskStopped"),
          class: "border-border/60 bg-muted/50 text-muted-foreground",
        };

  const snippet = useMemo(() => previewSnippet(notice.content), [notice.content]);

  return (
    <div
      className={cn(
        "group my-2 overflow-hidden rounded-xl border shadow-2xs backdrop-blur-xs transition-all duration-200",
        borderClass,
        open && "shadow-xs",
      )}
    >
      {/* Header clickable summary */}
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="flex w-full cursor-pointer flex-wrap items-center justify-between gap-2.5 px-3.5 py-2.5 text-left transition-colors hover:bg-muted/30"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {isDone ? (
            <CheckCircle2 className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
          ) : isFailed ? (
            <AlertCircle className="size-4 shrink-0 text-rose-600 dark:text-rose-400" />
          ) : (
            <Square className="size-3.5 shrink-0 text-muted-foreground" />
          )}

          <span className="shrink-0 rounded-md border border-border/60 bg-background/60 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-muted-foreground">
            #{notice.taskId}
          </span>

          {taskKind ? (
            <span className="shrink-0 rounded-md border border-border/40 bg-muted/40 px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider text-muted-foreground">
              {taskKind}
            </span>
          ) : null}

          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            {notice.description || notice.taskId}
          </span>

          {!open && snippet ? (
            <span className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground/75 sm:inline">
              · {snippet}
            </span>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <span
            className={cn(
              "rounded-md border px-1.5 py-0.5 text-[10px] font-medium",
              statusBadge.class,
            )}
          >
            {statusBadge.label}
          </span>

          {effectiveReport ? (
            <Button
              variant="ghost"
              size="icon"
              className="size-6 text-muted-foreground hover:text-foreground"
              title={copied ? t("backgroundTaskCopied") : t("backgroundTaskCopyReport")}
              aria-label={copied ? t("backgroundTaskCopied") : t("backgroundTaskCopyReport")}
              onClick={handleCopy}
            >
              {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
            </Button>
          ) : null}

          <ChevronRight
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
              open && "rotate-90",
            )}
          />
        </div>
      </button>

      {/* Expanded Markdown Report */}
      {open ? (
        <div className="border-t border-border/40 bg-background/50 px-4 py-3 text-xs leading-relaxed">
          {effectiveReport ? (
            <AssistantMarkdown text={effectiveReport} />
          ) : (
            <p className="text-muted-foreground">{t("subagentReport")}</p>
          )}
        </div>
      ) : null}
    </div>
  );
});

export const BackgroundNoticeRow = memo(function BackgroundNoticeRow({
  items,
  sessionId,
}: {
  items: GroupedSessionItem[];
  sessionId?: string;
}) {
  const tasks = useSessionStore((state) => (sessionId ? state.backgroundBySession[sessionId] : []));

  const allNotices = useMemo(() => {
    return items.flatMap((item) => {
      const parsed = parseBackgroundNotice(item.text);
      if (parsed.length > 0) return parsed;
      // Fallback if parsing didn't find items
      return [
        {
          taskId: "task",
          description: item.text.slice(0, 40),
          kind: "message" as const,
          content: item.text,
        },
      ];
    });
  }, [items]);

  return (
    <div className="space-y-1">
      {allNotices.map((notice, index) => {
        const key = `${notice.taskId}-${index}`;
        const matchedTask = tasks?.find((task) => task.task_id === notice.taskId);

        if (notice.kind === "message") {
          return <MessageNoticeCard key={key} notice={notice} taskKind={matchedTask?.kind} />;
        }

        return (
          <DeliveryNoticeCard
            key={key}
            notice={notice}
            fullReport={matchedTask?.report}
            taskKind={matchedTask?.kind}
          />
        );
      })}
    </div>
  );
});
