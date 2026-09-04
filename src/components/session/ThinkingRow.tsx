import { Brain, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { formatSessionDuration, thinkingDurationSeconds, thinkingText } from "@/lib/sessionLines";

import { cn } from "@/lib/utils";

export function ThinkingRow({ items, nowMs }: { items: GroupedSessionItem[]; nowMs?: number }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(true);
  const seconds = thinkingDurationSeconds(items, nowMs);
  const label =
    seconds < 1
      ? t("thinkingForBrief")
      : t("thinkingFor", { duration: formatSessionDuration(t, seconds) });
  const body = thinkingText(items);

  return (
    <div className="rounded-xl border border-border/60 bg-muted/15 transition-all duration-150 hover:border-border/80">
      <button
        type="button"
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <Brain className="size-3.5 shrink-0 text-purple-500/80 dark:text-purple-400" />
        <span className="min-w-0 flex-1 truncate text-left tracking-tight">{label}</span>
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground/70 transition-transform duration-150",
            open && "rotate-90",
          )}
        />
      </button>
      {open ? (
        <div className="border-t border-border/40 px-3.5 py-2.5">
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap font-mono text-[11.5px] leading-relaxed text-muted-foreground/90 select-text">
            {body}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
