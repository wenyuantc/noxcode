import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseUsageLine } from "@/lib/sessionLines";
import { formatTokenCount } from "@/lib/utils";

export function UsageRow({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const parsed = parseUsageLine(item.text);
  if (!parsed) return null;

  const items = [
    parsed.input != null ? { label: t("usageInput"), count: formatTokenCount(parsed.input) } : null,
    parsed.output != null
      ? { label: t("usageOutput"), count: formatTokenCount(parsed.output) }
      : null,
    parsed.reasoning != null
      ? { label: t("usageReasoning"), count: formatTokenCount(parsed.reasoning) }
      : null,
    parsed.cache != null ? { label: t("usageCache"), count: formatTokenCount(parsed.cache) } : null,
    parsed.total != null ? { label: t("usageTotal"), count: formatTokenCount(parsed.total) } : null,
  ].filter(Boolean) as { label: string; count: string }[];

  return (
    <div className="flex flex-wrap items-center gap-1.5 py-1">
      {items.map((entry) => (
        <span
          key={entry.label}
          className="inline-flex items-center gap-1 rounded-md border border-border/40 bg-muted/20 px-1.5 py-0.5 font-mono text-[10.5px] text-muted-foreground/80"
        >
          <span className="text-[10px] font-sans text-muted-foreground/60">{entry.label}</span>
          <span className="font-medium text-foreground/80 tabular-nums">{entry.count}</span>
        </span>
      ))}
    </div>
  );
}
