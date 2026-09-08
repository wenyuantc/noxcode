import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseToolHeader } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";

export function ToolCallLine({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const parsed = parseToolHeader(item);

  return (
    <div className="rounded-lg border border-border/40 bg-muted/20 text-xs transition-colors">
      <button
        type="button"
        className="flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-left"
        onClick={() => setOpen((value) => !value)}
      >
        <span
          className={cn(
            "inline-flex shrink-0 items-center rounded border px-1.5 py-0.5 text-[10px] font-medium leading-none",
            parsed.badgeClass,
          )}
        >
          {parsed.badge}
        </span>
        <span
          className={cn(
            "min-w-0 flex-1 truncate font-mono text-[11px]",
            parsed.failed ? "text-red-600 dark:text-red-400" : "text-foreground/90",
          )}
          title={parsed.detail}
        >
          {parsed.detail}
        </span>
        {parsed.failed ? (
          <span className="shrink-0 text-[10px] font-medium text-red-600 dark:text-red-400">
            {t("toolFailed")}
          </span>
        ) : null}
        <ChevronRight
          className={cn(
            "size-3 shrink-0 text-muted-foreground/60 transition-transform duration-150",
            open && "rotate-90",
          )}
        />
      </button>
      {open && item.result !== undefined ? (
        <pre className="border-t border-border/30 bg-black/20 p-2 max-h-80 overflow-auto whitespace-pre-wrap font-mono text-[11px] text-foreground/80">
          {item.result}
        </pre>
      ) : null}
      {open && item.result === undefined ? (
        <p className="border-t border-border/30 p-2 text-[11px] text-muted-foreground">
          {t("toolResult")}
        </p>
      ) : null}
    </div>
  );
}
