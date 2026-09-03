import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, Loader2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  buildTurnBlocks,
  changedFilesFromItems,
  groupSessionLines,
  lineToneClass,
  parseTodoList,
  type GroupedSessionItem,
  type RawSessionLine,
  type SessionTurnBlock,
  type TurnSegment,
} from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";
import { FileChangeRow } from "./FileChangeRow";
import { AgentStatusRow, McpStatusRow, PermissionStatusRow } from "./SessionStatusRows";
import { TerminalRow } from "./TerminalRow";
import { ThinkingRow } from "./ThinkingRow";
import { ToolSummaryRow } from "./ToolSummaryRow";
import { TurnActionBar } from "./TurnActionBar";
import { TurnFilesChanged } from "./TurnFilesChanged";
import { UsageRow } from "./UsageRow";
import { WorkSummaryBar } from "./WorkSummaryBar";

const EMPTY_LINES: RawSessionLine[] = [];
const BOTTOM_THRESHOLD = 80;

function attachLiveStream(
  block: SessionTurnBlock,
  stream?: { kind: string; text: string },
): SessionTurnBlock {
  if (!stream?.text) return block;
  const now = new Date().toISOString();
  const item: GroupedSessionItem = {
    id: `${block.id}-stream`,
    kind: stream.kind === "reasoning" ? "system" : "assistant",
    text: stream.text,
    createdAt: now,
  };
  const segments = [...block.segments];
  const targetKind = stream.kind === "reasoning" ? "thinking" : "assistant";
  const last = segments[segments.length - 1];
  if (last?.kind === targetKind) {
    segments[segments.length - 1] = { ...last, items: [...last.items, item] };
  } else {
    segments.push({ kind: targetKind, items: [item] });
  }
  return {
    ...block,
    endedAt: now,
    assistant: targetKind === "assistant" ? [...block.assistant, item] : block.assistant,
    segments,
  };
}

function renderSegment(segment: TurnSegment, running: boolean, nowMs?: number) {
  switch (segment.kind) {
    case "thinking":
      return <ThinkingRow items={segment.items} nowMs={nowMs} />;
    case "tools":
      return (
        <ToolSummaryRow
          items={segment.items}
          running={running && !segment.items[segment.items.length - 1]?.result}
        />
      );
    case "terminal":
      return (
        <TerminalRow item={segment.items[0]!} running={running && !segment.items[0]?.result} />
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
    case "assistant":
      return <AssistantMarkdown text={segment.items.map((item) => item.text).join("\n\n")} />;
    case "usage":
      return <UsageRow item={segment.items[0]!} />;
    default:
      return (
        <div className="space-y-1">
          {segment.items.map((item) => {
            if (item.text.startsWith("[PERMISSION]")) {
              return <PermissionStatusRow key={item.id} text={item.text} />;
            }
            if (item.text.startsWith("[内置 Agent]")) {
              return <AgentStatusRow key={item.id} text={item.text} />;
            }
            if (item.text.startsWith("[MCP]")) {
              return <McpStatusRow key={item.id} text={item.text} />;
            }
            return (
              <p key={item.id} className={cn("text-xs", lineToneClass(item.kind, item.text))}>
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

function isNearBottom(node: HTMLElement) {
  return node.scrollHeight - node.scrollTop - node.clientHeight <= BOTTOM_THRESHOLD;
}

export function EventStream({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const lines = useSessionStore((state) => state.lines[sessionId]) ?? EMPTY_LINES;
  const stream = useSessionStore((state) => state.stream[sessionId]);
  const turnState = useSessionStore((state) => state.turnState[sessionId]);
  const items = groupSessionLines(lines);
  const blocks = buildTurnBlocks(items);
  const working = turnState === "working";
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [showLatest, setShowLatest] = useState(false);
  const pinnedRef = useRef(true);
  const parentRef = useRef<HTMLDivElement>(null);
  const prevWorkingRef = useRef(working);
  const prevHasStreamRef = useRef(Boolean(stream?.text));
  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 64,
    overscan: 8,
    gap: 16,
    getItemKey: (index) => blocks[index]?.id ?? index,
    measureElement: (element) => (element as HTMLElement).offsetHeight,
  });

  const syncScrollState = useCallback(() => {
    const node = parentRef.current;
    if (!node) return;
    const nearBottom = isNearBottom(node);
    pinnedRef.current = nearBottom;
    setShowLatest(!nearBottom);
  }, []);

  const scrollToLatest = () => {
    if (blocks.length === 0) return;
    pinnedRef.current = true;
    setShowLatest(false);
    virtualizer.scrollToIndex(blocks.length - 1, { align: "end" });
  };

  useEffect(() => {
    pinnedRef.current = true;
    setShowLatest(false);
  }, [sessionId]);

  useEffect(() => {
    if (!working) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [working]);

  useEffect(() => {
    const hasStream = Boolean(stream?.text);
    const streamGone = prevHasStreamRef.current && !hasStream;
    const workingEnded = prevWorkingRef.current && !working;
    prevHasStreamRef.current = hasStream;
    prevWorkingRef.current = working;
    if (streamGone || workingEnded) virtualizer.measure();
  }, [stream?.text, working, virtualizer]);

  useEffect(() => {
    if (blocks.length === 0 || !pinnedRef.current) return;
    virtualizer.scrollToIndex(blocks.length - 1, { align: "end" });
  }, [blocks.length, stream?.text, virtualizer]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(syncScrollState);
    return () => window.cancelAnimationFrame(frame);
  }, [blocks.length, syncScrollState, virtualizer.getTotalSize()]);

  return (
    <div className="relative h-full min-h-0">
      <div ref={parentRef} className="h-full overflow-auto px-6 py-4" onScroll={syncScrollState}>
        <div className="relative mx-auto max-w-3xl" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((virtual) => {
            const isLast = virtual.index === blocks.length - 1;
            const block = isLast
              ? attachLiveStream(blocks[virtual.index]!, stream)
              : blocks[virtual.index]!;
            return (
              <div
                key={virtual.key}
                data-index={virtual.index}
                ref={virtualizer.measureElement}
                className="absolute top-0 left-0 w-full"
                style={{ transform: `translateY(${virtual.start}px)` }}
              >
                <TurnBlockView
                  block={block}
                  sessionId={sessionId}
                  working={isLast && working}
                  nowMs={nowMs}
                />
              </div>
            );
          })}
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
}

function TurnBlockView({
  block,
  sessionId,
  working,
  nowMs,
}: {
  block: SessionTurnBlock;
  sessionId: string;
  working: boolean;
  nowMs: number;
}) {
  const showWork = working || block.segments.length > 0 || block.tools.length > 0;
  const assistantText = block.assistant.map((item) => item.text).join("\n\n");
  const changedPaths = changedFilesFromItems(block.tools);

  return (
    <div className="space-y-2">
      {block.user ? (
        <div className="ml-auto max-w-[80%] rounded-2xl bg-secondary px-4 py-2 text-sm">
          {block.user.text}
        </div>
      ) : null}
      {showWork ? (
        <WorkSummaryBar block={block} tools={block.tools} working={working} nowMs={nowMs} />
      ) : null}
      {block.segments.map((segment, index) => (
        <div key={`${segment.kind}-${segment.items[0]?.id ?? index}`}>
          {renderSegment(segment, working, working ? nowMs : undefined)}
        </div>
      ))}
      {!working && assistantText ? (
        <TurnActionBar
          sessionId={sessionId}
          userText={block.user?.text}
          assistantText={assistantText}
          endedAt={block.endedAt}
        />
      ) : null}
      {!working && changedPaths.length > 0 ? <TurnFilesChanged paths={changedPaths} /> : null}
      {working ? <Loader2 className="size-4 animate-spin text-muted-foreground" /> : null}
    </div>
  );
}
