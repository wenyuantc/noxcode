import {
  AlertCircle,
  Boxes,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Copy,
  Loader2,
  Send,
  Square,
} from "lucide-react";
import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  listNativeBackgroundTasks,
  sendNativeBackgroundMessage,
  stopNativeBackgroundTask,
} from "@/lib/backend";
import type { NativeBackgroundTask } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";

function taskStatusBadge(
  status: NativeBackgroundTask["status"],
  t: (key: string) => string,
): { label: string; class: string } {
  switch (status) {
    case "running":
      return {
        label: t("backgroundTaskRunning"),
        class: "border-amber-500/30 bg-amber-500/15 text-amber-700 dark:text-amber-300",
      };
    case "queued":
      return {
        label: t("backgroundTaskQueued"),
        class: "border-amber-500/20 bg-amber-500/10 text-amber-600 dark:text-amber-400",
      };
    case "done":
      return {
        label: t("backgroundTaskCompleted"),
        class: "border-emerald-500/30 bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
      };
    case "failed":
      return {
        label: t("backgroundTaskFailed"),
        class: "border-rose-500/30 bg-rose-500/15 text-rose-700 dark:text-rose-300",
      };
    case "stopped":
      return {
        label: t("backgroundTaskStopped"),
        class: "border-border/60 bg-muted/50 text-muted-foreground",
      };
    default:
      return {
        label: status,
        class: "border-border/60 bg-muted/40 text-muted-foreground",
      };
  }
}

const BackgroundTaskRow = memo(function BackgroundTaskRow({
  sessionId,
  task,
  live,
  open,
  onToggle,
}: {
  sessionId: string;
  task: NativeBackgroundTask;
  live: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation("sessions");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const active = live && (task.status === "queued" || task.status === "running");

  const act = async (stop: boolean) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (stop) {
        await stopNativeBackgroundTask(sessionId, task.task_id);
      } else {
        await sendNativeBackgroundMessage(sessionId, task.task_id, message.trim());
        setMessage("");
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const handleCopy = async (event: React.MouseEvent) => {
    event.stopPropagation();
    if (!task.report) return;
    try {
      await navigator.clipboard.writeText(task.report);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore
    }
  };

  const badge = taskStatusBadge(task.status, t);

  return (
    <div className="border-t border-border/50 transition-colors">
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full cursor-pointer flex-wrap items-center justify-between gap-2 px-3.5 py-2.5 text-left transition-colors hover:bg-muted/30"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {task.status === "running" ? (
            <Loader2 className="size-3.5 shrink-0 animate-spin text-amber-500" />
          ) : task.status === "queued" ? (
            <Clock className="size-3.5 shrink-0 text-amber-500/80" />
          ) : task.status === "done" ? (
            <CheckCircle2 className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
          ) : task.status === "failed" ? (
            <AlertCircle className="size-3.5 shrink-0 text-rose-600 dark:text-rose-400" />
          ) : (
            <Square className="size-3 shrink-0 text-muted-foreground" />
          )}

          <span className="shrink-0 rounded-md border border-border/60 bg-background/60 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-muted-foreground">
            #{task.task_id}
          </span>

          <span className="shrink-0 rounded-md border border-border/40 bg-muted/40 px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider text-muted-foreground">
            {task.kind}
          </span>

          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            {task.description || task.task_id}
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <span
            className={cn("rounded-md border px-1.5 py-0.5 text-[10px] font-medium", badge.class)}
          >
            {badge.label}
          </span>

          {task.report ? (
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

      {open ? (
        <div className="space-y-2 border-t border-border/40 bg-background/50 px-3.5 py-3 text-xs leading-relaxed">
          {task.report ? (
            <div className="rounded-lg border border-border/40 bg-muted/15 p-3">
              <AssistantMarkdown text={task.report} />
            </div>
          ) : active ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-3 animate-spin" />
              <span>{t("backgroundTaskRunning")}</span>
            </p>
          ) : null}

          {active ? (
            <form
              className="flex items-center gap-2 pt-1"
              onSubmit={(event) => {
                event.preventDefault();
                if (message.trim()) void act(false);
              }}
            >
              <Input
                aria-label="后台任务消息"
                placeholder={t("backgroundTaskSendHint")}
                value={message}
                onChange={(event) => setMessage(event.target.value)}
                disabled={busy}
                className="h-8 text-xs bg-background/80"
              />
              <Button
                size="sm"
                className="h-8 gap-1.5 px-3 text-xs"
                type="submit"
                title="发送消息"
                aria-label="发送消息"
                disabled={busy || !message.trim()}
              >
                <Send className="size-3.5" />
                <span className="hidden sm:inline">{t("send")}</span>
              </Button>
              <Button
                size="sm"
                variant="outline"
                className="h-8 gap-1.5 px-3 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
                type="button"
                title="停止后台任务"
                aria-label="停止后台任务"
                disabled={busy}
                onClick={() => {
                  void act(true);
                }}
              >
                <Square className="size-3.5" />
                <span className="hidden sm:inline">{t("stop")}</span>
              </Button>
            </form>
          ) : null}

          {error ? (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
});

export function BackgroundTasks({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const live = useSessionStore((state) => Boolean(state.liveBySession[sessionId]));
  const tasks = useSessionStore((state) => state.backgroundBySession[sessionId]);
  const [error, setError] = useState<string | null>(null);

  // Master panel collapse state
  const [panelOpen, setPanelOpen] = useState(true);

  // Set of opened individual tasks
  const [openTasks, setOpenTasks] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!live) return;
    let cancelled = false;
    const initial = useSessionStore.getState().backgroundBySession[sessionId];
    void listNativeBackgroundTasks(sessionId)
      .then((next) => {
        if (!cancelled && useSessionStore.getState().backgroundBySession[sessionId] === initial)
          useSessionStore
            .getState()
            .onBackgroundTasks({ session_record_id: sessionId, tasks: next });
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [live, sessionId]);

  const runningCount = useMemo(
    () => tasks?.filter((t) => t.status === "running" || t.status === "queued").length ?? 0,
    [tasks],
  );
  const doneCount = useMemo(() => tasks?.filter((t) => t.status === "done").length ?? 0, [tasks]);

  const toggleTask = (taskId: string) => {
    setOpenTasks((prev) => {
      const next = new Set(prev);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  };

  const toggleAll = (event: React.MouseEvent) => {
    event.stopPropagation();
    if (!tasks) return;
    if (openTasks.size === tasks.length) {
      setOpenTasks(new Set());
    } else {
      setOpenTasks(new Set(tasks.map((task) => task.task_id)));
    }
  };

  if (!tasks?.length && !error) return null;

  return (
    <section
      aria-label="后台任务"
      className="overflow-hidden rounded-xl border border-border/70 bg-card/50 shadow-xs backdrop-blur-xs transition-all duration-200"
    >
      {/* Master Card Header */}
      <div
        className="flex cursor-pointer select-none flex-wrap items-center justify-between gap-2.5 px-3.5 py-2.5 transition-colors hover:bg-muted/20"
        onClick={() => setPanelOpen((prev) => !prev)}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <div className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Boxes className="size-3.5" />
          </div>

          <h3 className="text-xs font-semibold tracking-tight text-foreground">
            {t("backgroundTasksTitle")}
          </h3>

          <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {t("backgroundTasksCount", { count: tasks?.length ?? 0 })}
          </span>

          {runningCount > 0 ? (
            <span className="flex items-center gap-1.5 rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              <span className="size-1.5 animate-ping rounded-full bg-amber-500" />
              {t("backgroundTasksRunningCount", { count: runningCount })}
            </span>
          ) : null}

          {doneCount > 0 ? (
            <span className="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
              {t("backgroundTasksDoneCount", { count: doneCount })}
            </span>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {panelOpen && tasks && tasks.length > 1 ? (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[11px] text-muted-foreground hover:text-foreground"
              onClick={toggleAll}
            >
              {openTasks.size === tasks.length
                ? t("backgroundTaskCollapseAll")
                : t("backgroundTaskExpandAll")}
            </Button>
          ) : null}

          <Button
            variant="ghost"
            size="icon"
            className="size-6 text-muted-foreground hover:text-foreground"
            title={panelOpen ? t("backgroundTaskCollapseReport") : t("backgroundTaskExpandReport")}
            aria-label={
              panelOpen ? t("backgroundTaskCollapseReport") : t("backgroundTaskExpandReport")
            }
          >
            {panelOpen ? (
              <ChevronDown className="size-3.5" />
            ) : (
              <ChevronRight className="size-3.5" />
            )}
          </Button>
        </div>
      </div>

      {/* Task Rows List */}
      {panelOpen ? (
        <div>
          {tasks?.map((task) => (
            <BackgroundTaskRow
              key={task.task_id}
              sessionId={sessionId}
              task={task}
              live={live}
              open={openTasks.has(task.task_id)}
              onToggle={() => toggleTask(task.task_id)}
            />
          ))}
        </div>
      ) : null}

      {error ? (
        <p role="alert" className="p-3 text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
