import { ArrowRight, Check, Circle, Loader2, Minimize2 } from "lucide-react";
import { useEffect, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import { latestTodos, type RawSessionLine } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";

const EMPTY_LINES: RawSessionLine[] = [];
const NARROW = 720;

export function TodoProcessPanel({
  sessionId,
  containerRef,
}: {
  sessionId: string;
  containerRef: RefObject<HTMLElement | null>;
}) {
  const { t } = useTranslation("sessions");
  const lines = useSessionStore((state) => state.lines[sessionId]) ?? EMPTY_LINES;
  const todos = latestTodos(lines);
  const [width, setWidth] = useState(NARROW);
  const [collapsed, setCollapsed] = useState(false);
  const [pinnedOpen, setPinnedOpen] = useState(false);

  useEffect(() => {
    setCollapsed(false);
    setPinnedOpen(false);
  }, [sessionId]);

  useEffect(() => {
    const node = containerRef.current;
    if (!node) return;
    const observer = new ResizeObserver((entries) => {
      const next = entries[0]?.contentRect.width;
      if (typeof next === "number") setWidth(next);
    });
    observer.observe(node);
    setWidth(node.getBoundingClientRect().width);
    return () => observer.disconnect();
  }, [containerRef]);

  if (!todos) return null;

  const narrow = width < NARROW;
  const expanded = pinnedOpen || (!collapsed && !narrow);
  const current = todos.current;
  const allDone = todos.completed === todos.total && todos.total > 0;

  if (!expanded) {
    return (
      <button
        type="button"
        className="absolute top-3 right-3 z-20 flex max-w-[min(22rem,calc(100%-1.5rem))] items-center gap-2 rounded-full border bg-zinc-900/80 px-3 py-1.5 text-xs text-zinc-100 shadow-sm backdrop-blur"
        onClick={() => {
          setCollapsed(false);
          setPinnedOpen(true);
        }}
      >
        {allDone ? (
          <Check className="size-3.5 shrink-0 text-emerald-400" />
        ) : current?.status === "in_progress" ? (
          <ArrowRight className="size-3.5 shrink-0" />
        ) : (
          <Loader2 className="size-3.5 shrink-0 animate-spin" />
        )}
        <span className="truncate">{current?.content ?? t("process")}</span>
      </button>
    );
  }

  return (
    <div className="absolute top-3 right-3 z-20 w-[min(20rem,calc(100%-1.5rem))] rounded-xl border border-white/10 bg-zinc-900/85 p-3 text-zinc-100 shadow-lg backdrop-blur">
      <div className="mb-2 flex items-center gap-2 text-sm">
        <span>{t("process")}</span>
        <span className={cn("text-xs", allDone ? "text-emerald-400" : "text-zinc-400")}>
          {todos.completed}/{todos.total}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          className="rounded p-1 text-zinc-400 hover:bg-white/10 hover:text-zinc-100"
          title={t("collapseProcess")}
          onClick={() => {
            setPinnedOpen(false);
            setCollapsed(true);
          }}
        >
          <Minimize2 className="size-3.5" />
        </button>
      </div>
      <ul className="space-y-1.5 text-sm">
        {todos.items.map((item, index) => (
          <li key={`${item.content}-${index}`} className="flex items-start gap-2">
            {item.status === "completed" ? (
              <Check className="mt-0.5 size-3.5 shrink-0 text-emerald-400" />
            ) : item.status === "in_progress" ? (
              <ArrowRight className="mt-0.5 size-3.5 shrink-0" />
            ) : (
              <Circle className="mt-0.5 size-3.5 shrink-0 text-zinc-500" />
            )}
            <span
              className={cn(
                "min-w-0 leading-5",
                item.status === "completed" && "text-zinc-400 line-through",
              )}
            >
              {item.content}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
