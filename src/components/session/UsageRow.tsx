import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseUsageLine } from "@/lib/sessionLines";
import { formatTokenCount } from "@/lib/utils";

export function UsageRow({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const parsed = parseUsageLine(item.text);
  if (!parsed) return null;
  const parts = [
    parsed.input != null ? `${t("usageInput")} ${formatTokenCount(parsed.input)}` : null,
    parsed.output != null ? `${t("usageOutput")} ${formatTokenCount(parsed.output)}` : null,
    parsed.reasoning != null
      ? `${t("usageReasoning")} ${formatTokenCount(parsed.reasoning)}`
      : null,
    parsed.cache != null ? `${t("usageCache")} ${formatTokenCount(parsed.cache)}` : null,
    parsed.total != null ? `${t("usageTotal")} ${formatTokenCount(parsed.total)}` : null,
  ].filter(Boolean);

  return <p className="text-xs text-muted-foreground tabular-nums">{parts.join(" · ")}</p>;
}
