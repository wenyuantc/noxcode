import { Bot, ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem, SessionTurnBlock } from "@/lib/sessionLines";
import { formatSessionDuration, workDurationSeconds } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { ToolCallLine } from "./ToolCallLine";

export function TurnHeader({
  block,
  tools,
  working,
  nowMs,
}: {
  block: SessionTurnBlock;
  tools: GroupedSessionItem[];
  working?: boolean;
  nowMs?: number;
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const seconds = workDurationSeconds(block, working ? nowMs : undefined);
  const duration = formatSessionDuration(t, seconds);
  const label = working ? t("workingFor", { duration }) : t("workedFor", { duration });
  const expandable = tools.length > 0;

  return (
    <div className="py-0.5">
      <div className="flex items-center gap-2">
        <span
          className={cn(
            "flex size-5 shrink-0 items-center justify-center rounded-md",
            working
              ? "bg-amber-500/15 text-amber-600 dark:text-amber-400"
              : "bg-primary/10 text-primary",
          )}
        >
          <Bot className="size-3" />
        </span>
        <span className="shrink-0 text-xs font-medium text-muted-foreground">
          {t("assistantRole")}
        </span>
        <button
          type="button"
          aria-expanded={expandable ? open : undefined}
          className={cn(
            "inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-border/40 bg-muted/20 px-2 py-0.5 text-xs font-medium text-muted-foreground/85 shadow-2xs transition-colors hover:bg-muted/40 hover:text-foreground",
            !expandable && "cursor-default",
          )}
          onClick={() => {
            if (!expandable) return;
            setOpen((value) => !value);
          }}
        >
          {working ? <span className="size-1.5 animate-pulse rounded-full bg-amber-500" /> : null}
          <span className="tabular-nums">{label}</span>
          {expandable ? (
            <ChevronDown
              className={cn(
                "size-3 shrink-0 text-muted-foreground/70 transition-transform duration-150",
                !open && "-rotate-90",
              )}
            />
          ) : null}
        </button>
      </div>
      {open && expandable ? (
        <div className="mt-1.5 space-y-1 rounded-xl border border-border/40 bg-muted/10 p-2">
          {tools.map((item) => (
            <ToolCallLine key={item.id} item={item} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
