import { Layers } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseCompactBoundary } from "@/lib/sessionLines";

function formatTokens(value: number): string {
  if (value >= 1000) return `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)}K`;
  return String(value);
}

export function CompactBoundaryRow({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const boundary = parseCompactBoundary(item.text);
  if (!boundary) return null;
  const saved = Math.max(0, boundary.pre_tokens - boundary.post_tokens);
  const percent = boundary.pre_tokens > 0 ? Math.round((saved / boundary.pre_tokens) * 100) : 0;
  return (
    <div className="my-2 flex items-center gap-3 text-xs text-muted-foreground">
      <div className="h-px flex-1 bg-border" />
      <div className="flex items-center gap-1.5 whitespace-nowrap">
        <Layers className="size-3.5 shrink-0" />
        <span>
          {t("compactBoundary", {
            trigger: t(`compactTrigger.${boundary.trigger}`, { defaultValue: boundary.trigger }),
            source: t(`compactSource.${boundary.source}`, { defaultValue: boundary.source }),
          })}
        </span>
        <span>
          {formatTokens(boundary.pre_tokens)} → {formatTokens(boundary.post_tokens)}
          {percent > 0 ? ` (-${percent}%)` : ""}
        </span>
        {boundary.instructions ? (
          <span className="max-w-64 truncate" title={boundary.instructions}>
            · {boundary.instructions}
          </span>
        ) : null}
      </div>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}
