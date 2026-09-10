import { Dialog as SheetPrimitive } from "@base-ui/react/dialog";
import {
  AlertCircle,
  ArrowDown,
  Bot,
  Check,
  CheckCircle2,
  ChevronRight,
  Clock,
  Coins,
  Copy,
  FileText,
  Loader2,
  Wrench,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { isNearBottom } from "@/lib/sessionScroll";
import type { GroupedSessionItem, RawSessionLine, TurnSegment } from "@/lib/sessionLines";
import {
  aggregateUsages,
  buildTurnBlocks,
  formatSessionDuration,
  groupSessionLines,
  isParentAgentSpawn,
  isThinkingItem,
  isUsageItem,
  parseSubagentResult,
  parseSubagentTag,
  parseToolHeader,
  parseUsageLine,
  segmentDurationSeconds,
  sessionLineBody,
  subagentSegmentIdentity,
  toolsStillRunning,
} from "@/lib/sessionLines";
import { cn, formatTokenCount } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { AssistantMarkdown } from "./AssistantMarkdown";
import { LookupResultCard } from "./LookupResultCard";
import { ThinkingRow } from "./ThinkingRow";
import { UsageChips } from "./UsageRow";

const EMPTY_LINES: RawSessionLine[] = [];

export function SubagentDrawer({ sessionId }: { sessionId: string }) {
  const activeSubagent = useUiStore((state) => state.activeSubagent);
  const setActiveSubagent = useUiStore((state) => state.setActiveSubagent);
  const lines = useSessionStore((state) => state.lines[sessionId]) ?? EMPTY_LINES;
  const turnState = useSessionStore((state) => state.turnState[sessionId]);
  const isWorking = turnState === "working";

  const isOpen = Boolean(activeSubagent && activeSubagent.sessionId === sessionId);

  // Group lines into turn blocks and find current segment and peers
  const items = useMemo(() => groupSessionLines(lines), [lines]);
  const blocks = useMemo(() => buildTurnBlocks(items), [items]);

  const { currentSegment, peerSegments } = useMemo(() => {
    if (!activeSubagent || activeSubagent.sessionId !== sessionId) {
      return { currentSegment: null, peerSegments: [] };
    }

    // Try finding in turn blocks first
    for (const block of blocks) {
      const subSegments = block.segments.filter((s) => s.kind === "subagent");
      const found = subSegments.find(
        (s) => subagentSegmentIdentity(s.items[0] ?? {}) === activeSubagent.identity,
      );
      if (found) {
        return { currentSegment: found, peerSegments: subSegments };
      }
    }

    // Fallback across all subagent segments
    const all = blocks.flatMap((b) => b.segments.filter((s) => s.kind === "subagent"));
    const found = all.find(
      (s) => subagentSegmentIdentity(s.items[0] ?? {}) === activeSubagent.identity,
    );
    return { currentSegment: found ?? null, peerSegments: found ? [found] : [] };
  }, [activeSubagent, blocks, sessionId]);

  const handleOpenChange = (open: boolean) => {
    if (!open) {
      setActiveSubagent(null);
    }
  };

  if (!isOpen || !currentSegment || !activeSubagent) {
    return null;
  }

  return (
    <SheetPrimitive.Root open={isOpen} onOpenChange={handleOpenChange}>
      <SheetPrimitive.Portal>
        {/* Backdrop Overlay */}
        <SheetPrimitive.Backdrop
          className={cn(
            "fixed inset-0 z-50 bg-black/20 backdrop-blur-xs transition-opacity duration-200",
            "data-ending-style:opacity-0 data-starting-style:opacity-0 dark:bg-black/45",
          )}
        />

        {/* Slide-out Drawer Panel */}
        <SheetPrimitive.Popup
          className={cn(
            "fixed inset-y-0 right-0 z-50 flex h-full w-[min(94vw,580px)] xl:w-[min(92vw,640px)] flex-col",
            "border-l border-border/80 bg-background/95 text-foreground shadow-2xl backdrop-blur-md",
            "transition-transform duration-200 ease-out outline-none",
            "data-ending-style:translate-x-full data-starting-style:translate-x-full",
          )}
        >
          <SubagentDrawerContent
            segment={currentSegment}
            peerSegments={peerSegments}
            sessionId={sessionId}
            activeIdentity={activeSubagent.identity}
            isWorking={isWorking}
            onClose={() => setActiveSubagent(null)}
            onSelectPeer={(identity) => setActiveSubagent({ sessionId, identity })}
          />
        </SheetPrimitive.Popup>
      </SheetPrimitive.Portal>
    </SheetPrimitive.Root>
  );
}

export function SubagentDrawerContent({
  segment,
  peerSegments,
  sessionId: _sessionId,
  activeIdentity,
  isWorking,
  onClose,
  onSelectPeer,
}: {
  segment: TurnSegment;
  peerSegments: TurnSegment[];
  sessionId: string;
  activeIdentity: string;
  isWorking: boolean;
  onClose: () => void;
  onSelectPeer: (identity: string) => void;
}) {
  const { t } = useTranslation("sessions");
  const items = segment.items;

  // Subagent tag info
  const rawTag = items[0]?.subagentTag ?? "";
  const parsedTag = useMemo(() => parseSubagentTag(rawTag), [rawTag]);

  // Determine status (running, completed, failed, stopped)
  const endItem = useMemo(
    () =>
      items.find((item) => {
        const body = sessionLineBody(item.text);
        return /^(?:后台任务 \S+ )?结束 (?:成功|失败|停止|已停止)/.test(body);
      }),
    [items],
  );

  const isCompleted = Boolean(endItem);
  const isFailed = endItem ? sessionLineBody(endItem.text).includes("失败") : false;
  const isStopped = endItem
    ? sessionLineBody(endItem.text).includes("停止")
    : !isWorking && !isCompleted;
  const isRunning = Boolean(isWorking && !isCompleted);

  // Partition items
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

  // Delivery report
  const deliveryReport = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i -= 1) {
      const body = sessionLineBody(items[i]?.text ?? "");
      const parsed = parseSubagentResult(items[i]?.result ?? body);
      if (parsed?.report) return parsed.report;
    }
    if (isCompleted && processItems.length > 0) {
      const last = processItems[processItems.length - 1];
      const body = sessionLineBody(last?.text ?? "").trim();
      if (body.length > 40) return body;
    }
    return null;
  }, [isCompleted, items, processItems]);

  // Duration
  const durationSec = segmentDurationSeconds(items);
  const durationText = durationSec > 0 ? formatSessionDuration(t, durationSec) : null;

  // Styling based on kind
  const kind = (parsedTag?.kind ?? "general").toLowerCase();
  const isExplore = kind === "explore";
  const isGeneral = kind === "general";

  const badgeColorClass = isExplore
    ? "bg-teal-500/15 text-teal-700 dark:text-teal-300 border-teal-500/25"
    : isGeneral
      ? "bg-indigo-500/15 text-indigo-700 dark:text-indigo-300 border-indigo-500/25"
      : "bg-violet-500/15 text-violet-700 dark:text-violet-300 border-violet-500/25";

  const botIconClass = isExplore
    ? "text-teal-600 dark:text-teal-400 bg-teal-500/10 border-teal-500/20"
    : isGeneral
      ? "text-indigo-600 dark:text-indigo-400 bg-indigo-500/10 border-indigo-500/20"
      : "text-violet-600 dark:text-violet-400 bg-violet-500/10 border-violet-500/20";

  // Copy state for delivery report
  const [copied, setCopied] = useState(false);
  const handleCopyReport = async () => {
    if (!deliveryReport) return;
    try {
      await navigator.clipboard.writeText(deliveryReport);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore clipboard error
    }
  };

  // Scroll management: Pin to bottom and "Scroll to bottom" button
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(isRunning);
  const programmaticRef = useRef(false);
  const [showScrollBottom, setShowScrollBottom] = useState(false);

  const scrollToBottom = useCallback((smooth = true) => {
    const node = scrollRef.current;
    if (!node) return;
    programmaticRef.current = true;
    pinnedRef.current = true;
    setShowScrollBottom(false);
    node.scrollTo({
      top: node.scrollHeight,
      behavior: smooth ? "smooth" : "auto",
    });
    window.setTimeout(() => {
      programmaticRef.current = false;
    }, 350);
  }, []);

  const prevIdentityRef = useRef<string | null>(null);

  // Handle scroll position on mount or switching subagents
  useEffect(() => {
    const isSwitching =
      prevIdentityRef.current !== null && prevIdentityRef.current !== activeIdentity;
    prevIdentityRef.current = activeIdentity;

    const snapTop = () => {
      const node = scrollRef.current;
      if (node) {
        programmaticRef.current = true;
        node.scrollTop = 0;
        programmaticRef.current = false;
      }
    };

    const snapBottom = () => {
      const node = scrollRef.current;
      if (node) {
        programmaticRef.current = true;
        node.scrollTop = node.scrollHeight;
        programmaticRef.current = false;
      }
    };

    // 切换子 Agent 后：自动回到顶部
    if (isSwitching) {
      pinnedRef.current = false;
      setShowScrollBottom(false);
      snapTop();
      const frame = window.requestAnimationFrame(snapTop);
      const timer = window.setTimeout(snapTop, 60);
      return () => {
        window.cancelAnimationFrame(frame);
        window.clearTimeout(timer);
      };
    }

    // 初次点开：如果是运行中的子 Agent，默认展示最新输出（滚到底部）
    if (isRunning) {
      pinnedRef.current = true;
      setShowScrollBottom(false);
      snapBottom();
      const frame = window.requestAnimationFrame(snapBottom);
      const timer = window.setTimeout(snapBottom, 60);
      return () => {
        window.cancelAnimationFrame(frame);
        window.clearTimeout(timer);
      };
    }

    // 初次点开已完成的子 Agent：默认在顶部
    pinnedRef.current = false;
    setShowScrollBottom(false);
    snapTop();
    const frame = window.requestAnimationFrame(snapTop);
    const timer = window.setTimeout(snapTop, 60);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  }, [activeIdentity, isRunning]);

  // When pinned to bottom and new updates arrive, keep following to bottom
  const itemsCount = items.length;
  const lastItemTextLength = items[items.length - 1]?.text.length ?? 0;
  const lastItemResultLength = items[items.length - 1]?.result?.length ?? 0;

  useEffect(() => {
    if (!pinnedRef.current) return;
    const node = scrollRef.current;
    if (!node) return;
    programmaticRef.current = true;
    node.scrollTop = node.scrollHeight;
    const frame = window.requestAnimationFrame(() => {
      node.scrollTop = node.scrollHeight;
      programmaticRef.current = false;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [itemsCount, lastItemTextLength, lastItemResultLength]);

  const handleScroll = useCallback(() => {
    if (programmaticRef.current) return;
    const node = scrollRef.current;
    if (!node) return;
    const metrics = {
      scrollHeight: node.scrollHeight,
      scrollTop: node.scrollTop,
      clientHeight: node.clientHeight,
    };
    const nearBottom = isNearBottom(metrics, 60);
    pinnedRef.current = nearBottom;
    setShowScrollBottom(!nearBottom);
  }, []);

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      {/* 1. Header */}
      <div className="shrink-0 border-b border-border/70 bg-card/60 px-5 pt-4 pb-3 backdrop-blur-sm">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 flex-1 items-start gap-3">
            <div className="relative mt-0.5 shrink-0">
              <div
                className={cn(
                  "flex size-9 items-center justify-center rounded-xl border",
                  botIconClass,
                )}
              >
                <Bot className="size-5" />
              </div>
              {isRunning ? (
                <span className="absolute -bottom-0.5 -right-0.5 flex size-2.5">
                  <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400 opacity-75" />
                  <span className="relative inline-flex size-2.5 rounded-full bg-amber-500" />
                </span>
              ) : null}
            </div>

            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="shrink-0 rounded-md border border-border/70 bg-muted/60 px-1.5 py-0.5 font-mono text-[11px] font-semibold text-muted-foreground">
                  #{parsedTag?.index ?? 1}
                </span>
                <span
                  className={cn(
                    "shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-[10.5px] font-semibold uppercase tracking-wider",
                    badgeColorClass,
                  )}
                >
                  {parsedTag?.kind ?? "general"}
                </span>
                {isRunning ? (
                  <span className="inline-flex items-center gap-1 rounded-md border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400">
                    <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
                    {t("subagentRunning")}
                  </span>
                ) : isStopped ? (
                  <span className="inline-flex items-center rounded-md border border-border/70 bg-muted/50 px-1.5 py-0.5 text-[11px] text-muted-foreground">
                    已停止
                  </span>
                ) : isFailed ? (
                  <span className="inline-flex items-center gap-1 rounded-md border border-rose-500/25 bg-rose-500/10 px-1.5 py-0.5 text-[11px] font-medium text-rose-600 dark:text-rose-400">
                    <AlertCircle className="size-3" />
                    {t("subagentFailed")}
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 rounded-md border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="size-3" />
                    {t("subagentCompleted")}
                  </span>
                )}
              </div>

              <h3 className="mt-1 font-semibold text-base tracking-tight text-foreground line-clamp-2">
                {parsedTag?.description || t("subagentDrawerTitle")}
              </h3>
            </div>
          </div>

          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            className="shrink-0 text-muted-foreground hover:text-foreground"
          >
            <X className="size-4" />
            <span className="sr-only">Close</span>
          </Button>
        </div>

        {/* Multi-subagent Switcher Bar */}
        {peerSegments.length > 1 ? (
          <div className="mt-3 flex items-center gap-1.5 overflow-x-auto pt-1 pb-0.5 scrollbar-none">
            <span className="shrink-0 text-[11px] text-muted-foreground/80 font-medium mr-0.5">
              {t("subagentBatchSwitch")}:
            </span>
            {peerSegments.map((peer, idx) => {
              const peerTag = parseSubagentTag(peer.items[0]?.subagentTag);
              const peerIdentity =
                subagentSegmentIdentity(peer.items[0] ?? {}) ?? String(peerTag?.index ?? idx + 1);
              const isSelected = peerIdentity === activeIdentity;
              const peerCompleted = peer.items.some((it) =>
                /^(?:后台任务 \S+ )?结束/.test(sessionLineBody(it.text)),
              );

              return (
                <button
                  key={peerIdentity}
                  type="button"
                  onClick={() => onSelectPeer(peerIdentity)}
                  className={cn(
                    "flex items-center gap-1.5 shrink-0 rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-150 cursor-pointer border",
                    isSelected
                      ? "bg-primary text-primary-foreground border-primary shadow-xs font-semibold"
                      : "bg-muted/40 text-muted-foreground hover:bg-muted hover:text-foreground border-border/50",
                  )}
                >
                  <span className="font-mono text-[10.5px] opacity-80">
                    #{peerTag?.index ?? idx + 1}
                  </span>
                  <span className="max-w-[130px] truncate">
                    {peerTag?.description || peerTag?.kind || `Agent ${idx + 1}`}
                  </span>
                  {isRunning && !peerCompleted ? (
                    <span className="size-1.5 rounded-full bg-amber-400 animate-pulse" />
                  ) : null}
                </button>
              );
            })}
          </div>
        ) : null}
      </div>

      {/* 2. Scrollable Body */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto px-5 py-4 space-y-4"
      >
        {/* Quick Overview Grid */}
        <div className="grid grid-cols-3 gap-2.5">
          <div className="flex items-center gap-2 rounded-xl border border-border/60 bg-muted/20 p-2.5">
            <Clock className="size-4 shrink-0 text-muted-foreground/70" />
            <div className="min-w-0 flex-1">
              <p className="text-[11px] text-muted-foreground leading-none">耗时</p>
              <p className="mt-1 font-mono text-xs font-semibold text-foreground truncate">
                {durationText ?? "--"}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 rounded-xl border border-border/60 bg-muted/20 p-2.5">
            <Wrench className="size-4 shrink-0 text-muted-foreground/70" />
            <div className="min-w-0 flex-1">
              <p className="text-[11px] text-muted-foreground leading-none">工具操作</p>
              <p className="mt-1 font-mono text-xs font-semibold text-foreground truncate">
                {toolItems.length} 项
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 rounded-xl border border-border/60 bg-muted/20 p-2.5">
            <Coins className="size-4 shrink-0 text-muted-foreground/70" />
            <div className="min-w-0 flex-1">
              <p className="text-[11px] text-muted-foreground leading-none">Tokens</p>
              <p className="mt-1 font-mono text-xs font-semibold text-foreground truncate">
                {totalUsage?.total ? `${formatTokenCount(totalUsage.total)}` : "--"}
              </p>
            </div>
          </div>
        </div>

        {/* Priority: Delivery Report (置顶交付成果) */}
        {deliveryReport ? (
          <div className="overflow-hidden rounded-2xl border border-emerald-500/30 bg-emerald-500/5 shadow-xs dark:border-emerald-500/25 dark:bg-emerald-500/5">
            <div className="flex items-center justify-between border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-2.5">
              <div className="flex items-center gap-2 text-xs font-semibold text-emerald-700 dark:text-emerald-300">
                <FileText className="size-4" />
                <span>{t("subagentReport")}</span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={handleCopyReport}
                className="h-7 gap-1.5 px-2 text-xs text-emerald-700 hover:bg-emerald-500/20 hover:text-emerald-800 dark:text-emerald-300 dark:hover:bg-emerald-500/20"
              >
                {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
                <span>{copied ? t("subagentReportCopied") : t("subagentCopyReport")}</span>
              </Button>
            </div>
            <div className="p-4 text-xs leading-relaxed text-foreground/95">
              <AssistantMarkdown text={deliveryReport} />
            </div>
          </div>
        ) : null}

        {/* Thinking Process */}
        {thinkingItems.length > 0 ? (
          <div className="space-y-1.5">
            <h4 className="text-xs font-semibold text-muted-foreground px-0.5">
              {t("subagentThinkingProcess")}
            </h4>
            <ThinkingRow items={thinkingItems} />
          </div>
        ) : null}

        {/* Tool Execution Details (逐项可折叠明细) */}
        {toolItems.length > 0 ? (
          <div className="space-y-2">
            <div className="flex items-center justify-between px-0.5">
              <h4 className="text-xs font-semibold text-muted-foreground flex items-center gap-1.5">
                <Wrench className="size-3.5" />
                <span>{t("subagentToolDetails")}</span>
                <span className="font-mono text-[11px] font-normal text-muted-foreground/75">
                  ({toolItems.length})
                </span>
              </h4>
              {isRunning && toolsStillRunning(toolItems) ? (
                <span className="flex items-center gap-1 text-[11px] font-medium text-amber-500">
                  <Loader2 className="size-3 animate-spin" />
                  {t("subagentRunning")}
                </span>
              ) : null}
            </div>

            <div className="space-y-2">
              {toolItems.map((toolItem) => (
                <CollapsibleToolItem
                  key={toolItem.id}
                  item={toolItem}
                  isParentRunning={isRunning}
                />
              ))}
            </div>
          </div>
        ) : null}

        {/* Process Messages / Dialogue */}
        {processItems.length > 0 ? (
          <div className="space-y-2">
            <h4 className="text-xs font-semibold text-muted-foreground px-0.5">
              {t("subagentProcessMessages")}
            </h4>
            <div className="space-y-2">
              {processItems.map((item) => {
                const cleanText = sessionLineBody(item.text).trim();
                if (!cleanText) return null;
                if (deliveryReport && cleanText === deliveryReport) return null;
                return (
                  <div
                    key={item.id}
                    className="rounded-xl border border-border/50 bg-muted/15 p-3 text-xs leading-relaxed text-foreground/90"
                  >
                    <AssistantMarkdown text={cleanText} />
                  </div>
                );
              })}
            </div>
          </div>
        ) : null}

        {/* Token Usage Chips */}
        {totalUsage ? (
          <div className="pt-2 border-t border-border/40">
            <UsageChips usage={totalUsage} />
          </div>
        ) : null}
      </div>

      {/* Floating Scroll to Bottom Button */}
      {showScrollBottom ? (
        <button
          type="button"
          onClick={() => scrollToBottom(true)}
          className="absolute right-5 bottom-5 z-20 flex size-9 cursor-pointer items-center justify-center rounded-full border border-border/80 bg-background/90 text-foreground/80 shadow-lg backdrop-blur-md transition-all duration-150 hover:bg-background hover:text-foreground hover:scale-105 active:scale-95 hover:border-primary/60"
          title={t("scrollToLatest")}
          aria-label={t("scrollToLatest")}
        >
          <ArrowDown className="size-4 text-foreground/90" strokeWidth={2} />
        </button>
      ) : null}
    </div>
  );
}

function CollapsibleToolItem({
  item,
  isParentRunning,
}: {
  item: GroupedSessionItem;
  isParentRunning: boolean;
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const parsed = parseToolHeader(item);
  const isRunning = isParentRunning && !item.result && !parsed.failed;

  return (
    <div className="overflow-hidden rounded-xl border border-border/60 bg-muted/20 transition-all duration-150 hover:border-border/90">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full cursor-pointer items-center justify-between gap-2.5 px-3 py-2.5 text-left text-xs transition-colors hover:bg-muted/30"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {isRunning ? (
            <Loader2 className="size-3.5 shrink-0 animate-spin text-amber-500" />
          ) : parsed.failed ? (
            <AlertCircle className="size-3.5 shrink-0 text-rose-500" />
          ) : (
            <CheckCircle2 className="size-3.5 shrink-0 text-emerald-500" />
          )}

          <span
            className={cn(
              "inline-flex shrink-0 items-center rounded-md border px-1.5 py-0.5 text-[10.5px] font-medium leading-none",
              parsed.badgeClass,
            )}
          >
            {parsed.badge}
          </span>

          <span
            className="min-w-0 flex-1 truncate font-mono text-xs font-medium text-foreground/90 select-text"
            title={parsed.detail}
          >
            {parsed.detail || item.toolName || item.tool?.name || "Tool"}
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {item.tool?.duration_ms != null ? (
            <span className="font-mono text-[10.5px] text-muted-foreground">
              {item.tool.duration_ms}ms
            </span>
          ) : null}
          <ChevronRight
            className={cn(
              "size-3.5 text-muted-foreground/60 transition-transform duration-150",
              open && "rotate-90",
            )}
          />
        </div>
      </button>

      {open ? (
        <div className="border-t border-border/50 bg-background/50 p-3 space-y-2">
          {/* Tool Args Summary if available and different from detail */}
          {item.tool?.args_summary && item.tool.args_summary !== parsed.detail ? (
            <div className="rounded-md border border-border/40 bg-muted/30 p-2 font-mono text-[11px] text-muted-foreground whitespace-pre-wrap break-all">
              <span className="font-semibold text-foreground/80">参数: </span>
              {item.tool.args_summary}
            </div>
          ) : null}

          {/* Result view */}
          {item.result ? (
            <LookupResultCard item={item} />
          ) : isRunning ? (
            <div className="flex items-center gap-2 py-1 text-xs text-muted-foreground">
              <Loader2 className="size-3.5 animate-spin text-amber-500" />
              <span>正在执行中，请稍候...</span>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">{t("toolResult")}</p>
          )}
        </div>
      ) : null}
    </div>
  );
}
