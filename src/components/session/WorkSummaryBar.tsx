import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem, SessionTurnBlock } from "@/lib/sessionLines";
import { formatSessionDuration, workDurationSeconds } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { ToolCallLine } from "./ToolCallLine";

export function WorkSummaryBar({
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

  return (
    <div className="py-0.5">
      <button
        type="button"
        className={cn(
          "inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-border/40 bg-muted/20 px-2 py-0.5 text-xs font-medium text-muted-foreground/85 shadow-2xs transition-colors hover:bg-muted/40 hover:text-foreground",
          tools.length === 0 && "cursor-default",
        )}
        onClick={() => {
          if (tools.length === 0) return;
          setOpen((value) => !value);
        }}
      >
        <span className="tabular-nums">{label}</span>
        {tools.length > 0 ? (
          <ChevronDown
            className={cn(
              "size-3 shrink-0 text-muted-foreground/70 transition-transform duration-150",
              !open && "-rotate-90",
            )}
          />
        ) : null}
      </button>
      {open ? (
        <div className="mt-1.5 space-y-1 rounded-xl border border-border/40 bg-muted/10 p-2">
          {tools.map((item) => (
            <ToolCallLine key={item.id} item={item} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
