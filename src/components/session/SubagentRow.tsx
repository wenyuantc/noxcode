import { AlertCircle, Bot, CheckCircle2, ChevronRight, FileText } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { TurnSegment } from "@/lib/sessionLines";
import {
  aggregateUsages,
  formatSessionDuration,
  isParentAgentSpawn,
  isThinkingItem,
  isUsageItem,
  parseSubagentResult,
  parseSubagentTag,
  parseUsageLine,
  segmentDurationSeconds,
  sessionLineBody,
} from "@/lib/sessionLines";
import { cn, formatTokenCount } from "@/lib/utils";
import { AssistantMarkdown } from "./AssistantMarkdown";
import { ThinkingRow } from "./ThinkingRow";
import { ToolSummaryRow } from "./ToolSummaryRow";
import { UsageChips } from "./UsageRow";

interface SubagentRowProps {
  segment: TurnSegment;
  running?: boolean;
  nowMs?: number;
}

export function SubagentRow({ segment, running, nowMs }: SubagentRowProps) {
  const { t } = useTranslation("sessions");
  const items = segment.items;

  // Extract subagent tag info from first item
  const rawTag = items[0]?.subagentTag ?? "";
  const parsedTag = useMemo(() => parseSubagentTag(rawTag), [rawTag]);

  // Determine status (running, completed, failed)
  const endItem = useMemo(
    () =>
      items.find((item) => {
        const body = sessionLineBody(item.text);
        return body.startsWith("结束 成功") || body.startsWith("结束 失败");
      }),
    [items],
  );

  const isCompleted = Boolean(endItem);
  const isFailed = endItem ? sessionLineBody(endItem.text).includes("失败") : false;
  const isRunning = Boolean(running && !isCompleted);

  const [open, setOpen] = useState(false);

  // Partition items: thinking, tools, process text, report, usage
  const thinkingItems = useMemo(() => items.filter(isThinkingItem), [items]);
  const toolItems = useMemo(
    () => items.filter((item) => item.kind === "tool" && !isParentAgentSpawn(item)),
    [items],
  );
  const usageItems = useMemo(() => items.filter(isUsageItem), [items]);

  const totalUsage = useMemo(() => {
    const usages = usageItems.map((item) => parseUsageLine(item.text));
    return aggregateUsages(usages);
  }, [usageItems]);

  // Filter regular assistant / process text
  const processItems = useMemo(() => {
    return items.filter((item) => {
      if (item.kind !== "assistant" && item.kind !== "system") return false;
      const body = sessionLineBody(item.text).trim();
      if (body.startsWith("启动（") || body.startsWith("结束 ")) return false;
      if (isThinkingItem(item)) return false;
      if (isUsageItem(item)) return false;
      return body.length > 0;
    });
  }, [items]);

  // Try to find delivery report (either from tool result or last assistant message when completed)
  const deliveryReport = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i -= 1) {
      const parsed = parseSubagentResult(items[i]?.result ?? items[i]?.text ?? "");
      if (parsed?.report) return parsed.report;
    }
    // If completed and last process item has significant content, treat as report
    if (isCompleted && processItems.length > 0) {
      const last = processItems[processItems.length - 1];
      const body = sessionLineBody(last?.text ?? "").trim();
      if (body.length > 40) return body;
    }
    return null;
  }, [isCompleted, items, processItems]);

  // Duration
  const durationSec = segmentDurationSeconds(items, nowMs);
  const durationText = durationSec > 0 ? formatSessionDuration(t, durationSec) : null;

  // Semantic styles based on kind
  const kind = (parsedTag?.kind ?? "general").toLowerCase();
  const isExplore = kind === "explore";
  const isGeneral = kind === "general";

  const badgeColorClass = isExplore
    ? "bg-teal-500/15 text-teal-700 dark:text-teal-300 border-teal-500/25"
    : isGeneral
      ? "bg-indigo-500/15 text-indigo-700 dark:text-indigo-300 border-indigo-500/25"
      : "bg-violet-500/15 text-violet-700 dark:text-violet-300 border-violet-500/25";

  const cardBorderClass = isExplore
    ? "border-teal-500/30 hover:border-teal-500/50"
    : isGeneral
      ? "border-indigo-500/30 hover:border-indigo-500/50"
      : "border-violet-500/30 hover:border-violet-500/50";

  const botIconClass = isExplore
    ? "text-teal-600 dark:text-teal-400 bg-teal-500/10"
    : isGeneral
      ? "text-indigo-600 dark:text-indigo-400 bg-indigo-500/10"
      : "text-violet-600 dark:text-violet-400 bg-violet-500/10";

  // Summary preview for collapsed state
  const summarySnippet = useMemo(() => {
    if (deliveryReport) {
      return deliveryReport.split("\n")[0]?.slice(0, 48) ?? "";
    }
    if (processItems.length > 0) {
      const first = sessionLineBody(processItems[0]?.text ?? "").trim();
      return first.split("\n")[0]?.slice(0, 48) ?? "";
    }
    if (toolItems.length > 0) {
      return t("subagentOperations", { count: toolItems.length });
    }
    return "";
  }, [deliveryReport, processItems, toolItems.length, t]);

  const toggleOpen = () => {
    setOpen((prev) => !prev);
  };

  return (
    <div
      className={cn(
        "group my-2 overflow-hidden rounded-xl border bg-card/50 shadow-2xs backdrop-blur-xs transition-all duration-200",
        cardBorderClass,
        open && "shadow-xs",
      )}
    >
      {/* Card Header */}
      <button
        type="button"
        onClick={toggleOpen}
        className="flex w-full cursor-pointer items-center justify-between gap-2.5 px-3.5 py-2.5 text-left transition-colors hover:bg-muted/30"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {/* Bot Icon with status indicator */}
          <div className="relative shrink-0">
            <div className={cn("flex size-6 items-center justify-center rounded-lg", botIconClass)}>
              <Bot className="size-3.5" />
            </div>
            {isRunning ? (
              <span className="absolute -bottom-0.5 -right-0.5 flex size-2">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400 opacity-75" />
                <span className="relative inline-flex size-2 rounded-full bg-amber-500" />
              </span>
            ) : null}
          </div>

          {/* Index badge */}
          <span className="rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-muted-foreground">
            #{parsedTag?.index ?? 1}
          </span>

          {/* Kind badge */}
          <span
            className={cn(
              "rounded-md border px-1.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wider",
              badgeColorClass,
            )}
          >
            {parsedTag?.kind ?? "general"}
          </span>

          {/* Title / Description */}
          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            {parsedTag?.description || t("subagentTag")}
          </span>

          {/* Collapsed Snippet Preview */}
          {!open && summarySnippet ? (
            <span className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground/75 sm:inline">
              · {summarySnippet}
            </span>
          ) : null}
        </div>

        {/* Right side status / duration / chevron */}
        <div className="flex shrink-0 items-center gap-2">
          {isRunning ? (
            <span className="flex items-center gap-1 text-[11px] font-medium text-amber-600 dark:text-amber-400">
              <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
              {t("subagentRunning")}
            </span>
          ) : isFailed ? (
            <span className="flex items-center gap-1 text-[11px] font-medium text-rose-600 dark:text-rose-400">
              <AlertCircle className="size-3" />
              {t("subagentFailed")}
            </span>
          ) : (
            <span className="flex items-center gap-1 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
              <CheckCircle2 className="size-3" />
              {t("subagentCompleted")}
            </span>
          )}

          {durationText ? (
            <span className="rounded-md bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
              {durationText}
            </span>
          ) : null}

          {totalUsage?.total != null ? (
            <span
              className="hidden rounded-md bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground sm:inline"
              title={`${t("usageTotal")}: ${formatTokenCount(totalUsage.total)}`}
            >
              {formatTokenCount(totalUsage.total)} tokens
            </span>
          ) : null}

          <ChevronRight
            className={cn(
              "size-3.5 text-muted-foreground/60 transition-transform duration-200 group-hover:text-foreground",
              open && "rotate-90",
            )}
          />
        </div>
      </button>

      {/* Card Body */}
      {open ? (
        <div className="space-y-2.5 border-t border-border/40 bg-muted/10 p-3 pt-2.5">
          {/* Thinking items */}
          {thinkingItems.length > 0 ? (
            <div className="pl-1">
              <ThinkingRow items={thinkingItems} nowMs={nowMs} />
            </div>
          ) : null}

          {/* Tool calls */}
          {toolItems.length > 0 ? (
            <div className="pl-1">
              <ToolSummaryRow
                items={toolItems}
                running={isRunning && !toolItems[toolItems.length - 1]?.result}
              />
            </div>
          ) : null}

          {/* Process Messages */}
          {processItems.length > 0 ? (
            <div className="space-y-1.5 pl-1 text-xs">
              {processItems.map((item) => {
                const cleanText = sessionLineBody(item.text).trim();
                if (!cleanText) return null;
                // If this is the delivery report shown below, avoid duplicate
                if (deliveryReport && cleanText === deliveryReport) return null;
                return (
                  <div key={item.id} className="text-foreground/90 leading-relaxed">
                    <AssistantMarkdown text={cleanText} />
                  </div>
                );
              })}
            </div>
          ) : null}

          {/* Final Delivery Report Section */}
          {deliveryReport ? (
            <div className="mt-2 rounded-xl border border-emerald-500/25 bg-emerald-500/5 p-3 dark:border-emerald-500/20 dark:bg-emerald-500/5">
              <div className="mb-1.5 flex items-center gap-1.5 text-xs font-semibold text-emerald-700 dark:text-emerald-300">
                <FileText className="size-3.5" />
                <span>{t("subagentReport")}</span>
              </div>
              <div className="text-xs text-foreground/90 leading-relaxed">
                <AssistantMarkdown text={deliveryReport} />
              </div>
            </div>
          ) : null}

          {/* Usage chips */}
          {totalUsage ? (
            <div className="pt-0.5 pl-1">
              <UsageChips usage={totalUsage} />
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
