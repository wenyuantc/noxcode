import { ChevronRight, Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { summarizeTools, toolTitle } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { LookupResultCard } from "./LookupResultCard";

export function ToolSummaryRow({
  items,
  running,
}: {
  items: GroupedSessionItem[];
  running?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const summary = summarizeTools(items);
  const parts = [
    summary.lists ? t("lookupList", { count: summary.lists }) : null,
    summary.searches ? t("lookupSearch", { count: summary.searches }) : null,
    summary.files ? t("lookupFile", { count: summary.files }) : null,
  ].filter(Boolean);
  const hasLookup = parts.length > 0;
  const label = running
    ? t("lookupRunning")
    : hasLookup
      ? parts.join(" · ")
      : items.map((item) => item.toolName ?? toolTitle(item.text)).join(" · ");

  return (
    <div className="rounded-xl border border-border/60 bg-muted/15 transition-all duration-150 hover:border-border/80">
      <button
        type="button"
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-xs"
        onClick={() => setOpen((value) => !value)}
      >
        <Search className="size-3.5 shrink-0 text-amber-500/90 dark:text-amber-400" />
        <span className="inline-flex items-center rounded border border-border/40 bg-background/50 px-1.5 py-0.5 font-medium text-[10px] text-muted-foreground">
          {t("lookupSummary", { parts: "" })
            .trim()
            .replace(/[:：]$/, "") || "查阅"}
        </span>
        <span className="min-w-0 flex-1 truncate text-left font-medium text-foreground/80">
          {label}
        </span>
        {running ? (
          <span className="flex shrink-0 items-center gap-1.5 text-[11px] font-medium text-amber-500">
            <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
            {t("running")}
          </span>
        ) : null}
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground/70 transition-transform duration-150",
            open && "rotate-90",
          )}
        />
      </button>
      {open ? (
        <div className="border-t border-border/40 p-3 space-y-2">
          {items.map((item) => (
            <LookupResultCard key={item.id} item={item} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
