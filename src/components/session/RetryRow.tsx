import { Loader2, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseRetryLine, summarizeRetry } from "@/lib/sessionLines";
import { SegmentCard } from "./SegmentCard";

export function RetryRow({ items, live }: { items: GroupedSessionItem[]; live?: boolean }) {
  const { t } = useTranslation("sessions");
  const summary = summarizeRetry(items);
  const failed = summary.failed;
  const parts = [
    failed ? t("retryFailedLabel") : t("retry"),
    summary.status != null ? `HTTP ${summary.status}` : null,
    live && !failed && summary.attempt != null && summary.maxRetries != null
      ? t("retryProgress", { attempt: summary.attempt, max: summary.maxRetries })
      : summary.count > 0
        ? t("retryCount", { count: summary.count })
        : null,
  ].filter(Boolean);

  return (
    <SegmentCard
      icon={
        live && !failed ? (
          <Loader2 className="size-3 animate-spin" />
        ) : (
          <RotateCcw className="size-3" />
        )
      }
      iconClassName="bg-muted/60 text-muted-foreground"
      title={parts.join(" · ")}
      tone={failed ? "danger" : "default"}
      contentClassName="space-y-2"
    >
      {items.map((item) => (
        <RetryAttempt key={item.id} text={item.text} />
      ))}
    </SegmentCard>
  );
}

function RetryAttempt({ text }: { text: string }) {
  const { t } = useTranslation("sessions");
  const parsed = parseRetryLine(text);
  if (!parsed) {
    return (
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md border px-3 py-2 text-xs leading-5 text-muted-foreground">
        {text}
      </pre>
    );
  }
  const headline = [
    parsed.failed
      ? t("retryFailedShort")
      : parsed.attempt != null && parsed.maxRetries != null
        ? t("retryProgress", { attempt: parsed.attempt, max: parsed.maxRetries })
        : null,
    parsed.status != null ? `HTTP ${parsed.status}` : parsed.title || null,
  ].filter(Boolean);
  return (
    <div>
      {headline.length > 0 ? (
        <p className="text-xs text-muted-foreground">{headline.join(" · ")}</p>
      ) : null}
      {parsed.message ? <p className="text-xs text-muted-foreground">{parsed.message}</p> : null}
      {parsed.json ? (
        <pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap rounded-md border px-3 py-2 text-xs leading-5 text-muted-foreground">
          {parsed.json}
        </pre>
      ) : null}
    </div>
  );
}
