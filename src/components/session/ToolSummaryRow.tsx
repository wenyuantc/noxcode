import { ChevronRight, Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { summarizeTools, toolTitle } from "@/lib/sessionLines";
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
      ? t("lookupSummary", { parts: parts.join(", ") })
      : items.map((item) => item.toolName ?? toolTitle(item.text)).join(" · ");

  return (
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-2 text-sm text-muted-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <Search className="size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-left">{label}</span>
        <ChevronRight className={`size-3.5 shrink-0 transition ${open ? "rotate-90" : ""}`} />
      </button>
      {open ? items.map((item) => <LookupResultCard key={item.id} item={item} />) : null}
    </div>
  );
}
