import { useTranslation } from "react-i18next";

import type { GroupedSessionItem, ParsedUsage } from "@/lib/sessionLines";
import { parseUsageLine } from "@/lib/sessionLines";
import { cn, formatTokenCount } from "@/lib/utils";

export function UsageChips({ usage, className }: { usage: ParsedUsage; className?: string }) {
  const { t } = useTranslation("sessions");

  const items = [
    usage.input != null ? { label: t("usageInput"), count: formatTokenCount(usage.input) } : null,
    usage.output != null
      ? { label: t("usageOutput"), count: formatTokenCount(usage.output) }
      : null,
    usage.reasoning != null
      ? { label: t("usageReasoning"), count: formatTokenCount(usage.reasoning) }
      : null,
    usage.cache != null ? { label: t("usageCache"), count: formatTokenCount(usage.cache) } : null,
    usage.total != null ? { label: t("usageTotal"), count: formatTokenCount(usage.total) } : null,
  ].filter(Boolean) as { label: string; count: string }[];

  if (items.length === 0) return null;

  return (
    <div className={cn("flex flex-wrap items-center gap-1.5 py-1", className)}>
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

export function UsageRow({ item, className }: { item: GroupedSessionItem; className?: string }) {
  const parsed = parseUsageLine(item.text);
  if (!parsed) return null;
  return <UsageChips usage={parsed} className={className} />;
}
