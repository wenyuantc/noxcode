import { AlertCircle, Bot, CheckCircle2, ChevronRight, Loader2, Square } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { formatSessionDuration, subagentSegmentIdentity } from "@/lib/sessionLines";
import type { SessionSubagentInfo } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";

const EMPTY_SUBAGENTS: SessionSubagentInfo[] = [];

interface SubagentPopoverProps {
  sessionId: string;
  disabled?: boolean;
}

export function SubagentPopover({ sessionId, disabled = false }: SubagentPopoverProps) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const subagents = useSessionStore(
    (state) => state.subagentsBySession[sessionId] ?? EMPTY_SUBAGENTS,
  );
  const setActiveSubagent = useUiStore((state) => state.setActiveSubagent);

  if (subagents.length === 0) {
    return null;
  }

  const totalCount = subagents.length;
  const hasRunning = subagents.some((s) => s.status === "running");
  const hasFailed = subagents.some((s) => s.status === "failed");

  const handleSelect = (subagent: SessionSubagentInfo) => {
    const identity =
      subagentSegmentIdentity({ subagentTag: subagent.id }) ??
      (typeof subagent.index === "number" ? `index:${subagent.index}` : subagent.id);
    setActiveSubagent({ sessionId, identity });
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={disabled}
        className={cn(
          "inline-flex h-7 cursor-pointer items-center justify-between gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40",
          hasRunning &&
            "border-emerald-500/50 bg-emerald-500/10 text-emerald-600 dark:border-emerald-500/40 dark:text-emerald-400",
          !hasRunning &&
            hasFailed &&
            "border-destructive/40 bg-destructive/10 text-destructive dark:border-destructive/30",
        )}
        title={t("subagentsPopoverTitle", { defaultValue: "子智能体运行状态" })}
        aria-label={t("subagentsPopoverTitle", { defaultValue: "子智能体运行状态" })}
      >
        <Bot className={cn("size-3.5 shrink-0", hasRunning && "animate-pulse")} />
        <span
          className={cn(
            "inline-flex min-w-4 items-center justify-center rounded-full px-1.5 py-0.5 text-[10px] font-semibold leading-none",
            hasRunning
              ? "bg-emerald-500 text-white dark:bg-emerald-600"
              : hasFailed
                ? "bg-destructive text-white"
                : "bg-muted text-muted-foreground",
          )}
        >
          {totalCount}
        </span>
      </PopoverTrigger>
      <PopoverContent side="top" align="start" sideOffset={6} className="w-80 p-3">
        <div className="flex items-center justify-between border-b border-border/50 pb-2">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
            <Bot className="size-4 text-muted-foreground" />
            <span>{t("subagentsPopoverTitle", { defaultValue: "子智能体运行状态" })}</span>
          </div>
          <span className="text-[11px] text-muted-foreground">
            {t("subagentsCount", { count: totalCount, defaultValue: "{{count}} 个子智能体" })}
          </span>
        </div>

        <div className="mt-2 max-h-64 space-y-1.5 overflow-y-auto pr-0.5">
          {subagents.map((item) => {
            const durationSec = item.duration_ms ? Math.round(item.duration_ms / 1000) : 0;
            const durationText = durationSec > 0 ? formatSessionDuration(t, durationSec) : null;

            return (
              <button
                key={item.id}
                type="button"
                onClick={() => handleSelect(item)}
                className="group flex w-full cursor-pointer flex-col gap-1 rounded-lg border border-border/50 bg-background/60 p-2 text-left transition-colors hover:border-border hover:bg-muted/40"
              >
                <div className="flex items-center justify-between gap-1.5">
                  <span
                    className="truncate text-xs font-medium text-foreground group-hover:text-primary"
                    title={item.description}
                  >
                    {item.description || item.id}
                  </span>
                  <span className="shrink-0 rounded bg-muted/80 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                    {item.kind}
                  </span>
                </div>

                <div className="flex items-center justify-between text-[11px]">
                  <div className="flex items-center gap-1.5">
                    {item.status === "running" ? (
                      <>
                        <Loader2 className="size-3 animate-spin text-emerald-500" />
                        <span className="font-medium text-emerald-600 dark:text-emerald-400">
                          {t("subagentRunning", { defaultValue: "正在执行" })}
                        </span>
                      </>
                    ) : item.status === "completed" ? (
                      <>
                        <CheckCircle2 className="size-3 text-emerald-500" />
                        <span className="text-muted-foreground">
                          {t("subagentCompleted", { defaultValue: "已完成" })}
                        </span>
                      </>
                    ) : item.status === "failed" ? (
                      <>
                        <AlertCircle className="size-3 text-destructive" />
                        <span className="font-medium text-destructive">
                          {t("subagentFailed", { defaultValue: "执行失败" })}
                        </span>
                      </>
                    ) : (
                      <>
                        <Square className="size-2.5 text-muted-foreground" />
                        <span className="text-muted-foreground">
                          {t("subagentStopped", { defaultValue: "已停止" })}
                        </span>
                      </>
                    )}
                  </div>
                  <div className="flex items-center gap-1 text-muted-foreground">
                    {durationText ? <span>{durationText}</span> : null}
                    <ChevronRight className="size-3 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5" />
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
