import { ChevronDown, ChevronRight, Square, Terminal } from "lucide-react";
import { memo, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { listNativeBackgroundProcesses, stopNativeBackgroundProcess } from "@/lib/backend";
import type { NativeBackgroundProcess } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";

function processStatusBadge(
  status: NativeBackgroundProcess["status"],
  t: (key: string) => string,
): { label: string; class: string } {
  switch (status) {
    case "running":
      return {
        label: t("backgroundProcessRunning"),
        class: "border-amber-500/30 bg-amber-500/15 text-amber-700 dark:text-amber-300",
      };
    case "exited":
      return {
        label: t("backgroundProcessExited"),
        class: "border-emerald-500/30 bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
      };
    case "failed":
      return {
        label: t("backgroundProcessFailed"),
        class: "border-rose-500/30 bg-rose-500/15 text-rose-700 dark:text-rose-300",
      };
    case "stopped":
      return {
        label: t("backgroundProcessStopped"),
        class: "border-border/60 bg-muted/50 text-muted-foreground",
      };
    default:
      return {
        label: status,
        class: "border-border/60 bg-muted/40 text-muted-foreground",
      };
  }
}

const BackgroundProcessRow = memo(function BackgroundProcessRow({
  sessionId,
  process,
  live,
}: {
  sessionId: string;
  process: NativeBackgroundProcess;
  live: boolean;
}) {
  const { t } = useTranslation("sessions");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const badge = processStatusBadge(process.status, t);
  const active = live && process.status === "running";

  const stop = async () => {
    if (busy || !active) return;
    setBusy(true);
    setError(null);
    try {
      await stopNativeBackgroundProcess(sessionId, process.process_id);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="border-t border-border/60 px-3.5 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <button
          type="button"
          className="min-w-0 flex-1 text-left"
          onClick={() => setOpen((value) => !value)}
        >
          <div className="flex flex-wrap items-center gap-2">
            <span className="truncate text-xs font-medium text-foreground">
              {process.description || process.command}
            </span>
            <span className={cn("rounded-full border px-1.5 py-0.5 text-[10px]", badge.class)}>
              {badge.label}
            </span>
          </div>
          <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
            {process.command}
          </p>
        </button>
        {active ? (
          <Button
            variant="ghost"
            size="icon"
            className="size-7 text-muted-foreground hover:text-destructive"
            title={t("backgroundProcessStopHint")}
            aria-label={t("backgroundProcessStopHint")}
            disabled={busy}
            onClick={() => void stop()}
          >
            <Square className="size-3.5" />
          </Button>
        ) : null}
      </div>
      {open && process.output_preview ? (
        <pre className="mt-2 max-h-40 overflow-auto rounded-md bg-muted/40 p-2 font-mono text-[11px] text-muted-foreground">
          {process.output_preview}
        </pre>
      ) : null}
      {error ? (
        <p role="alert" className="mt-2 text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
});

export function BackgroundProcesses({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const live = useSessionStore((state) => Boolean(state.liveBySession[sessionId]));
  const processes = useSessionStore((state) => state.processesBySession[sessionId]);
  const [error, setError] = useState<string | null>(null);
  const [panelOpen, setPanelOpen] = useState(true);

  useEffect(() => {
    if (!live) return;
    let cancelled = false;
    const initial = useSessionStore.getState().processesBySession[sessionId];
    void listNativeBackgroundProcesses(sessionId)
      .then((next) => {
        if (!cancelled && useSessionStore.getState().processesBySession[sessionId] === initial) {
          useSessionStore.getState().onBackgroundProcesses({
            session_record_id: sessionId,
            processes: next,
          });
        }
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [live, sessionId]);

  const runningCount = useMemo(
    () => processes?.filter((item) => item.status === "running").length ?? 0,
    [processes],
  );

  if (!processes?.length && !error) return null;

  return (
    <section
      aria-label={t("backgroundProcessesTitle")}
      className="overflow-hidden rounded-xl border border-border/70 bg-card/50 shadow-xs backdrop-blur-xs"
    >
      <div
        className="flex cursor-pointer select-none items-center justify-between gap-2.5 px-3.5 py-2.5 hover:bg-muted/20"
        onClick={() => setPanelOpen((value) => !value)}
      >
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Terminal className="size-3.5" />
          </div>
          <h3 className="text-xs font-semibold tracking-tight text-foreground">
            {t("backgroundProcessesTitle")}
          </h3>
          <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {t("backgroundProcessesCount", { count: processes?.length ?? 0 })}
          </span>
          {runningCount > 0 ? (
            <span className="rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              {t("backgroundProcessesRunningCount", { count: runningCount })}
            </span>
          ) : null}
        </div>
        {panelOpen ? (
          <ChevronDown className="size-3.5 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-3.5 text-muted-foreground" />
        )}
      </div>
      {panelOpen
        ? processes?.map((process) => (
            <BackgroundProcessRow
              key={process.process_id}
              sessionId={sessionId}
              process={process}
              live={live}
            />
          ))
        : null}
      {error ? (
        <p role="alert" className="p-3 text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
