import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem, SessionTurnBlock } from "@/lib/sessionLines";
import { workDurationSeconds } from "@/lib/sessionLines";
import { ToolCallLine } from "./ToolCallLine";

export function WorkSummaryBar({
  block,
  tools,
}: {
  block: SessionTurnBlock;
  tools: GroupedSessionItem[];
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  if (tools.length === 0) return null;
  return (
    <div className="rounded-lg border bg-muted/30">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2 text-sm"
        onClick={() => setOpen((value) => !value)}
      >
        <ChevronDown className={`size-3.5 transition ${open ? "" : "-rotate-90"}`} />
        {t("workSummary", { seconds: workDurationSeconds(block), count: tools.length })}
      </button>
      {open ? tools.map((item) => <ToolCallLine key={item.id} item={item} />) : null}
    </div>
  );
}
