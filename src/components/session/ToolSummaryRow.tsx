import { Search } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { summarizeTools, toolTitle } from "@/lib/sessionLines";
import { LookupResultCard } from "./LookupResultCard";
import { SegmentCard } from "./SegmentCard";

export function ToolSummaryRow({
  items,
  running,
}: {
  items: GroupedSessionItem[];
  running?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const summary = summarizeTools(items);
  const parts = [
    summary.queries ? t("lookupQuery", { count: summary.queries }) : null,
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
    <SegmentCard
      icon={<Search className="size-3" />}
      iconClassName="bg-amber-500/15 text-amber-500/90 dark:text-amber-400"
      badge={t("lookup")}
      title={label}
      running={running}
      contentClassName="space-y-2"
    >
      {items.map((item) => (
        <LookupResultCard key={item.id} item={item} />
      ))}
    </SegmentCard>
  );
}
