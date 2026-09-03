import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem, SessionTurnBlock } from "@/lib/sessionLines";
import { formatSessionDuration, workDurationSeconds } from "@/lib/sessionLines";
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
    <div>
      <button
        type="button"
        className="flex items-center gap-1 text-sm text-muted-foreground"
        onClick={() => {
          if (tools.length === 0) return;
          setOpen((value) => !value);
        }}
      >
        <span>{label}</span>
        {tools.length > 0 ? (
          <ChevronDown className={`size-3.5 transition ${open ? "" : "-rotate-90"}`} />
        ) : null}
      </button>
      {open ? tools.map((item) => <ToolCallLine key={item.id} item={item} />) : null}
    </div>
  );
}
