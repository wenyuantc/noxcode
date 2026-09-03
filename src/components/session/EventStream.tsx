import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef } from "react";

import { buildTurnBlocks, groupSessionLines, lineToneClass } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { WorkSummaryBar } from "./WorkSummaryBar";

export function EventStream({ sessionId }: { sessionId: string }) {
  const lines = useSessionStore((state) => state.lines[sessionId] ?? []);
  const stream = useSessionStore((state) => state.stream[sessionId]);
  const items = groupSessionLines(lines);
  const blocks = buildTurnBlocks(items);
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: blocks.length + (stream?.text ? 1 : 0),
    getScrollElement: () => parentRef.current,
    estimateSize: () => 96,
    overscan: 8,
  });

  useEffect(() => {
    if (blocks.length === 0) return;
    virtualizer.scrollToIndex(blocks.length - 1, { align: "end" });
  }, [blocks.length, stream?.text, virtualizer]);

  return (
    <div ref={parentRef} className="min-h-0 flex-1 overflow-auto px-6 py-4">
      <div className="relative mx-auto max-w-3xl" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtual) => {
          const isStream = virtual.index >= blocks.length;
          return (
            <div
              key={virtual.key}
              data-index={virtual.index}
              ref={virtualizer.measureElement}
              className="absolute top-0 left-0 w-full pb-4"
              style={{ transform: `translateY(${virtual.start}px)` }}
            >
              {isStream ? (
                <pre className="whitespace-pre-wrap text-sm text-muted-foreground">
                  {stream?.kind === "reasoning" ? `[思考] ${stream.text}` : stream?.text}
                </pre>
              ) : (
                <TurnBlockView block={blocks[virtual.index]!} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TurnBlockView({ block }: { block: ReturnType<typeof buildTurnBlocks>[number] }) {
  return (
    <div className="space-y-2">
      {block.user ? (
        <div className="ml-auto max-w-[80%] rounded-2xl bg-secondary px-4 py-2 text-sm">
          {block.user.text}
        </div>
      ) : null}
      <WorkSummaryBar block={block} tools={block.tools} />
      {block.system.map((item) => (
        <p key={item.id} className={cn("text-xs", lineToneClass(item.kind, item.text))}>
          {item.text}
        </p>
      ))}
      {block.assistant.map((item) => (
        <pre key={item.id} className="whitespace-pre-wrap text-sm leading-6">
          {item.text}
        </pre>
      ))}
    </div>
  );
}
