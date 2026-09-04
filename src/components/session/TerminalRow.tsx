import { ChevronRight, SquareTerminal } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { commandText } from "@/lib/sessionLines";

import { cn } from "@/lib/utils";

export function TerminalRow({ item, running }: { item: GroupedSessionItem; running?: boolean }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const command = commandText(item);

  return (
    <div className="rounded-xl border border-border/60 bg-muted/15 transition-all duration-150 hover:border-border/80">
      <button
        type="button"
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left text-xs"
        onClick={() => setOpen((value) => !value)}
      >
        <SquareTerminal className="size-3.5 shrink-0 text-sky-500 dark:text-sky-400" />
        <span className="inline-flex items-center rounded border border-border/40 bg-background/50 px-1.5 py-0.5 font-medium text-[10px] text-muted-foreground">
          {t("terminal")}
        </span>
        {command ? (
          <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-foreground/90">
            {command}
          </span>
        ) : (
          <span className="min-w-0 flex-1" />
        )}
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
        <div className="border-t border-border/40 bg-black/25 p-3">
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap font-mono text-[11.5px] leading-relaxed text-foreground/90 select-text">
            <span className="text-emerald-500">$ </span>
            {command}
            {item.result ? `\n${item.result}` : ""}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
