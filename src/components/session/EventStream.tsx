import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, History, Loader2 } from "lucide-react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  buildTurnBlocks,
  changedFilesFromItems,
  groupSessionLines,
  hasToolResult,
  lineToneClass,
  parseTodoList,
  sessionLineBody,
  toolsStillRunning,
  type RawSessionLine,
  type SessionTurnBlock,
  type TurnSegment,
} from "@/lib/sessionLines";
import {
  captureScrollAnchor,
  isNearBottom,
  pinAfterUserScroll,
  scrollDeltaForAnchor,
} from "@/lib/sessionScroll";
import { attachLiveFragments, EMPTY_STREAM, turnSegmentReactKey } from "@/lib/sessionStream";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";
import { BackgroundNoticeRow } from "./BackgroundNoticeRow";
import { CompactBoundaryRow } from "./CompactBoundaryRow";
import { FileChangeRow } from "./FileChangeRow";
import { GoalRow } from "./GoalRow";
import { RetryRow } from "./RetryRow";
import {
  AgentStatusRow,
  McpStatusRow,
  PermissionStatusRow,
  WorktreeStatusRow,
} from "./SessionStatusRows";
import { PlanAskCard } from "./PlanAskCard";
import { BackgroundProcesses } from "./BackgroundProcesses";
import { BackgroundTasks } from "./BackgroundTasks";
import { PendingPlanApproval, PlanRow } from "./PlanRow";
import { SubagentRow } from "./SubagentRow";
import { TerminalRow } from "./TerminalRow";
import { ThinkingRow } from "./ThinkingRow";
import { ToolSummaryRow } from "./ToolSummaryRow";
import { TurnActionBar } from "./TurnActionBar";
import { TurnFilesChanged } from "./TurnFilesChanged";
import { UsageRow } from "./UsageRow";
import { UserBubble } from "./UserBubble";
import { WorkSummaryBar } from "./WorkSummaryBar";

const EMPTY_LINES: RawSessionLine[] = [];
const VIRTUALIZE_AFTER = 24;

function renderSegment(
  segment: TurnSegment,
  running: boolean,
  nowMs?: number,
  live?: boolean,
  sessionId?: string,
) {
  switch (segment.kind) {
    case "thinking":
      return <ThinkingRow items={segment.items} nowMs={nowMs} />;
    case "plan":
      return sessionId ? <PlanRow item={segment.items[0]!} sessionId={sessionId} /> : null;
    case "retry":
      return <RetryRow items={segment.items} live={live && running} />;
    case "compact":
      return <CompactBoundaryRow item={segment.items[0]!} />;
    case "goal":
      return <GoalRow item={segment.items[0]!} />;
    case "tools":
      return (
        <ToolSummaryRow
          items={segment.items}
          running={running && toolsStillRunning(segment.items)}
        />
      );
    case "terminal":
      return (
        <TerminalRow
          item={segment.items[0]!}
          running={running && !hasToolResult(segment.items[0]!)}
        />
      );
    case "file":
      return <FileChangeRow items={segment.items} />;
    case "changes":
      return <FileChangeRow items={segment.items} grouped />;
    case "todo": {
      const todos = parseTodoList(segment.items[0]?.text ?? "");
      if (!todos) return null;
      return (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          {running ? <Loader2 className="size-3.5 animate-spin" /> : null}
          <TodoLineText todos={todos} />
        </p>
      );
    }
    case "subagent":
      return (
        <SubagentRow segment={segment} running={running} nowMs={nowMs} sessionId={sessionId} />
      );
    case "assistant":
      return <AssistantMarkdown text={segment.items.map((item) => item.text).join("\n\n")} />;
    case "usage":
      return <UsageRow item={segment.items[0]!} />;
    case "background_notice":
      return <BackgroundNoticeRow items={segment.items} sessionId={sessionId} />;
    default:
      return (
        <div className="space-y-1">
          {segment.items.map((item) => {
            const body = sessionLineBody(item.text);
            if (body.startsWith("[PERMISSION]")) {
              return <PermissionStatusRow key={item.id} text={item.text} />;
            }
            if (body.startsWith("[内置 Agent]")) {
              return <AgentStatusRow key={item.id} text={item.text} />;
            }
            if (body.startsWith("[MCP]")) {
              return <McpStatusRow key={item.id} text={item.text} />;
            }
            if (body.startsWith("[WORKTREE]")) {
              return <WorktreeStatusRow key={item.id} text={item.text} />;
            }
            return (
              <p
                key={item.id}
                className={cn(
                  "text-xs whitespace-pre-wrap font-mono break-words",
                  lineToneClass(item.kind, item.text, item.ok),
                )}
              >
                {item.text}
              </p>
            );
          })}
        </div>
      );
  }
}

function TodoLineText({ todos }: { todos: NonNullable<ReturnType<typeof parseTodoList>> }) {
  const { t } = useTranslation("sessions");
  return (
    <>
      {t("todoLine", {
        current: todos.current?.content ?? "",
        done: todos.completed,
        total: todos.total,
      })}
    </>
  );
}

export const EventStream = memo(function EventStream({
  sessionId,
  active = true,
}: {
  sessionId: string;
  active?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const lines = useSessionStore((state) => state.lines[sessionId]) ?? EMPTY_LINES;
  const stream = useSessionStore((state) => state.stream[sessionId]) ?? EMPTY_STREAM;
  const turnState = useSessionStore((state) => state.turnState[sessionId]);
  const planQuestion = useSessionStore(
    (state) => Object.values(state.planQuestions[sessionId] ?? {})[0],
  );
  const hasAsk = planQuestion?.session_record_id === sessionId;
  const hasMoreEarlier = useSessionStore((state) => state.hasMoreEarlier[sessionId]);
  const loadingEarlier = useSessionStore((state) => state.loadingEarlier[sessionId]);
  const loadEarlierHistory = useSessionStore((state) => state.loadEarlierHistory);
  const items = useMemo(() => groupSessionLines(lines), [lines]);
  const blocks = useMemo(() => buildTurnBlocks(items), [items]);
  const lastUserBlockId = useMemo(() => {
    for (let index = blocks.length - 1; index >= 0; index -= 1) {
      if (blocks[index]?.user) return blocks[index]?.id;
    }
    return undefined;
  }, [blocks]);
  const working = turnState === "working";
  const virtualize = blocks.length > VIRTUALIZE_AFTER;
  const layoutSignature = `${blocks
    .map((block) => `${block.id}:${block.segments.length}:${block.endedAt}`)
    .join("|")}${hasAsk ? ":ask" : ""}`;
  const streamSignature = stream.map((part) => `${part.id}:${part.text.length}`).join("|");
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [showLatest, setShowLatest] = useState(false);
  const pinnedRef = useRef(true);
  const programmaticRef = useRef(false);
  const followFrameRef = useRef<number | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const blocksRef = useRef(blocks);
  blocksRef.current = blocks;
  const virtualizer = useVirtualizer({
    count: virtualize ? blocks.length : 0,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 240,
    overscan: 4,
    gap: 16,
    getItemKey: (index) => blocks[index]?.id ?? index,
    measureElement: (element, entry) => {
      const fromEntry = entry?.borderBoxSize?.[0]?.blockSize;
      if (typeof fromEntry === "number" && fromEntry > 0) return fromEntry;
      return (element as HTMLElement).getBoundingClientRect().height;
    },
  });
  const totalSize = virtualizer.getTotalSize();

  const applyScrollToLatest = useCallback(() => {
    const snap = () => {
      const node = parentRef.current;
      if (!node || node.clientHeight === 0 || !pinnedRef.current) return;
      programmaticRef.current = true;
      node.scrollTop = node.scrollHeight;
    };
    snap();
    if (followFrameRef.current != null) window.cancelAnimationFrame(followFrameRef.current);
    followFrameRef.current = window.requestAnimationFrame(() => {
      followFrameRef.current = null;
      snap();
      programmaticRef.current = false;
    });
  }, []);

  const syncScrollState = useCallback(() => {
    if (!active) return;
    const node = parentRef.current;
    if (!node) return;
    const metrics = {
      scrollHeight: node.scrollHeight,
      scrollTop: node.scrollTop,
      clientHeight: node.clientHeight,
    };
    const next = pinAfterUserScroll({
      programmatic: programmaticRef.current,
      clientHeight: metrics.clientHeight,
      nearBottom: isNearBottom(metrics),
      previous: pinnedRef.current,
    });
    pinnedRef.current = next;
    setShowLatest(!next);
  }, [active]);

  const scrollToLatest = useCallback(() => {
    pinnedRef.current = true;
    setShowLatest(false);
    applyScrollToLatest();
  }, [applyScrollToLatest]);

  const handleLoadEarlier = useCallback(async () => {
    const node = parentRef.current;
    if (!node) return;
    const viewportTop = node.getBoundingClientRect().top;
    const anchor = captureScrollAnchor(
      viewportTop,
      Array.from(node.querySelectorAll("[data-block-id]")).map((element) => {
        const rect = element.getBoundingClientRect();
        return {
          key: (element as HTMLElement).dataset.blockId ?? "",
          top: rect.top,
          bottom: rect.bottom,
        };
      }),
    );
    let userMoved = false;
    const markMoved = () => {
      userMoved = true;
    };
    node.addEventListener("wheel", markMoved);
    node.addEventListener("pointerdown", markMoved);
    programmaticRef.current = true;
    pinnedRef.current = false;
    try {
      await loadEarlierHistory(sessionId);
    } finally {
      node.removeEventListener("wheel", markMoved);
      node.removeEventListener("pointerdown", markMoved);
    }
    const restore = () => {
      const parent = parentRef.current;
      if (!parent || userMoved || !anchor) {
        programmaticRef.current = false;
        return;
      }
      const selector = `[data-block-id="${CSS.escape(anchor.key)}"]`;
      const applyOffset = () => {
        const target = parent.querySelector(selector);
        if (target) {
          parent.scrollTop += scrollDeltaForAnchor(
            target.getBoundingClientRect().top,
            parent.getBoundingClientRect().top,
            anchor.offset,
          );
        }
        programmaticRef.current = false;
      };
      const index = blocksRef.current.findIndex((item) => item.id === anchor.key);
      if (virtualize && index >= 0 && !parent.querySelector(selector)) {
        virtualizer.scrollToIndex(index, { align: "start" });
        requestAnimationFrame(applyOffset);
        return;
      }
      applyOffset();
    };
    requestAnimationFrame(() => requestAnimationFrame(restore));
  }, [loadEarlierHistory, sessionId, virtualize, virtualizer]);

  useEffect(() => {
    if (!active || !working) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active, working]);

  useLayoutEffect(() => {
    if (!active) return;
    pinnedRef.current = true;
    setShowLatest(false);
    applyScrollToLatest();
  }, [active, sessionId, applyScrollToLatest]);

  useLayoutEffect(() => {
    if (!active || !pinnedRef.current) return;
    applyScrollToLatest();
  }, [
    active,
    layoutSignature,
    streamSignature,
    blocks.length,
    hasAsk,
    totalSize,
    applyScrollToLatest,
  ]);

  useEffect(() => {
    if (!active) return;
    const node = parentRef.current;
    const content = contentRef.current;
    if (!node || !content) return;
    const observer = new ResizeObserver(() => {
      if (pinnedRef.current) {
        applyScrollToLatest();
        return;
      }
      setShowLatest(
        !isNearBottom({
          scrollHeight: node.scrollHeight,
          scrollTop: node.scrollTop,
          clientHeight: node.clientHeight,
        }),
      );
    });
    observer.observe(content);
    observer.observe(node);
    return () => observer.disconnect();
  }, [active, applyScrollToLatest, sessionId, virtualize]);

  useEffect(() => {
    return () => {
      if (followFrameRef.current != null) window.cancelAnimationFrame(followFrameRef.current);
    };
  }, []);

  const renderBlock = (block: SessionTurnBlock, index: number) => {
    const isLast = index === blocks.length - 1;
    const view = isLast ? attachLiveFragments(block, stream) : block;
    return (
      <TurnBlockView
        block={view}
        sessionId={sessionId}
        working={isLast && working}
        nowMs={isLast && working ? nowMs : undefined}
        editableUser={view.id === lastUserBlockId}
        showAsk={isLast && hasAsk}
      />
    );
  };

  return (
    <div className="relative h-full min-h-0">
      <div
        ref={parentRef}
        className="h-full overflow-auto overscroll-y-contain px-6 py-4"
        onScroll={syncScrollState}
        onWheel={() => {
          programmaticRef.current = false;
        }}
      >
        <div ref={contentRef}>
          {hasMoreEarlier ? (
            <div className="mx-auto mb-4 flex max-w-3xl justify-center">
              <button
                type="button"
                disabled={loadingEarlier}
                onClick={handleLoadEarlier}
                className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs text-muted-foreground transition hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
              >
                {loadingEarlier ? (
                  <>
                    <Loader2 className="size-3.5 animate-spin" />
                    <span>{t("loadingEarlier")}</span>
                  </>
                ) : (
                  <>
                    <History className="size-3.5" />
                    <span>{t("loadEarlier")}</span>
                  </>
                )}
              </button>
            </div>
          ) : null}
          {virtualize ? (
            <div
              className="relative mx-auto max-w-3xl"
              style={{ height: virtualizer.getTotalSize() }}
            >
              {virtualizer.getVirtualItems().map((virtual) => (
                <div
                  key={virtual.key}
                  data-index={virtual.index}
                  data-block-id={blocks[virtual.index]?.id}
                  ref={virtualizer.measureElement}
                  className="absolute top-0 left-0 w-full"
                  style={{ transform: `translateY(${virtual.start}px)` }}
                >
                  {renderBlock(blocks[virtual.index]!, virtual.index)}
                </div>
              ))}
            </div>
          ) : (
            <div className="mx-auto flex max-w-3xl flex-col gap-4">
              {blocks.map((block, index) => (
                <div key={block.id} data-block-id={block.id}>
                  {renderBlock(block, index)}
                </div>
              ))}
              {blocks.length === 0 && hasAsk ? <PlanAskCard sessionId={sessionId} /> : null}
            </div>
          )}
          <div className="mx-auto mt-4 max-w-3xl space-y-4">
            <PendingPlanApproval sessionId={sessionId} />
            <BackgroundTasks sessionId={sessionId} />
            <BackgroundProcesses sessionId={sessionId} />
          </div>
        </div>
      </div>
      {showLatest ? (
        <button
          type="button"
          className="absolute bottom-3 left-1/2 z-10 flex size-8 -translate-x-1/2 items-center justify-center rounded-full border border-foreground/25 bg-background/50 text-muted-foreground backdrop-blur-sm hover:bg-background/80"
          title={t("scrollToLatest")}
          aria-label={t("scrollToLatest")}
          onClick={scrollToLatest}
        >
          <ArrowDown className="size-4" strokeWidth={1.5} />
        </button>
      ) : null}
    </div>
  );
});

const TurnBlockView = memo(function TurnBlockView({
  block,
  sessionId,
  working,
  nowMs,
  editableUser,
  showAsk,
}: {
  block: SessionTurnBlock;
  sessionId: string;
  working: boolean;
  nowMs?: number;
  editableUser: boolean;
  showAsk?: boolean;
}) {
  const showWork =
    (Boolean(block.user) || working) &&
    (working || block.segments.length > 0 || block.tools.length > 0);
  const assistantText = block.assistant.map((item) => item.text).join("\n\n");
  const changedPaths = changedFilesFromItems(block.tools);

  return (
    <div className="space-y-2">
      {block.user ? (
        <UserBubble
          text={block.user.text}
          images={block.user.images}
          sessionId={sessionId}
          editable={editableUser}
          working={working}
        />
      ) : null}
      {showWork ? (
        <WorkSummaryBar block={block} tools={block.tools} working={working} nowMs={nowMs} />
      ) : null}
      {block.segments.map((segment, index) => (
        <div key={turnSegmentReactKey(block.id, segment, index, block.segments)}>
          {renderSegment(
            segment,
            working,
            working ? nowMs : undefined,
            index === block.segments.length - 1,
            sessionId,
          )}
        </div>
      ))}
      {showAsk ? <PlanAskCard sessionId={sessionId} /> : null}
      {!working && assistantText ? (
        <TurnActionBar
          sessionId={sessionId}
          userText={block.user?.text}
          assistantText={assistantText}
          endedAt={block.endedAt}
        />
      ) : null}
      {!working && changedPaths.length > 0 ? <TurnFilesChanged paths={changedPaths} /> : null}
      {working && !showAsk ? (
        <Loader2 className="size-4 animate-spin text-muted-foreground" />
      ) : null}
    </div>
  );
});
