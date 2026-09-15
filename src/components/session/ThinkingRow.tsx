import { Brain } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { formatSessionDuration, thinkingDurationSeconds, thinkingText } from "@/lib/sessionLines";
import { SegmentCard } from "./SegmentCard";

export function ThinkingRow({ items, nowMs }: { items: GroupedSessionItem[]; nowMs?: number }) {
  const { t } = useTranslation("sessions");
  const seconds = thinkingDurationSeconds(items, nowMs);
  const label =
    seconds < 1
      ? t("thinkingForBrief")
      : t("thinkingFor", { duration: formatSessionDuration(t, seconds) });
  const body = thinkingText(items);

  return (
    <SegmentCard
      icon={<Brain className="size-3" />}
      iconClassName="bg-purple-500/15 text-purple-500/90 dark:text-purple-400"
      title={label}
      contentClassName="py-2.5"
    >
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap font-mono text-code-sm leading-relaxed text-muted-foreground/90 select-text">
        {body}
      </pre>
    </SegmentCard>
  );
}
