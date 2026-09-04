import { BarChart3 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import type { NativeUsageDailyBucket } from "@/lib/types";
import {
  formatUsageDayLabel,
  formatUsageTokenCount,
  formatUsageTokenExact,
  usageTrendLabelIndexes,
} from "@/lib/usageAnalytics";
import { cn } from "@/lib/utils";

export function UsageTrendCard({
  days,
  empty,
}: {
  days: NativeUsageDailyBucket[];
  empty: boolean;
}) {
  const { t } = useTranslation("settings");
  const maxTokens = Math.max(1, ...days.map((item) => item.input_tokens + item.output_tokens));
  const labelIndexes = new Set(usageTrendLabelIndexes(days.length));

  return (
    <SettingCard icon={BarChart3} title={t("usage.trendTitle")} description={t("usage.trendHint")}>
      {empty ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
      ) : (
        <div className="space-y-3">
          <div className="flex items-center justify-end gap-3 text-[10px] text-muted-foreground">
            <span className="inline-flex items-center gap-1">
              <span className="size-2 rounded-sm bg-sky-500/80" />
              {t("usage.inputTokens")}
            </span>
            <span className="inline-flex items-center gap-1">
              <span className="size-2 rounded-sm bg-violet-500/80" />
              {t("usage.outputTokens")}
            </span>
          </div>
          <div className="flex h-44 gap-2">
            <div className="flex h-[calc(100%-1.25rem)] flex-col justify-between py-0.5 text-[10px] font-mono text-muted-foreground">
              <span>{formatUsageTokenCount(maxTokens)}</span>
              <span>{formatUsageTokenCount(maxTokens / 2)}</span>
              <span>0</span>
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex h-[calc(100%-1.25rem)] items-end gap-px sm:gap-1">
                {days.map((item) => {
                  const total = item.input_tokens + item.output_tokens;
                  const inputHeight = (item.input_tokens / maxTokens) * 100;
                  const outputHeight = (item.output_tokens / maxTokens) * 100;
                  return (
                    <div
                      key={item.date}
                      className="flex h-full min-w-0 flex-1 flex-col justify-end"
                      title={`${item.date} · in ${formatUsageTokenExact(item.input_tokens)} · out ${formatUsageTokenExact(item.output_tokens)} · ${formatUsageTokenExact(total)}`}
                    >
                      <div className="flex h-full w-full flex-col justify-end overflow-hidden rounded-t-sm">
                        <div
                          className="w-full bg-violet-500/80"
                          style={{ height: `${outputHeight}%` }}
                        />
                        <div
                          className="w-full bg-sky-500/80"
                          style={{ height: `${inputHeight}%` }}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
              <div className="mt-1.5 flex gap-px sm:gap-1">
                {days.map((item, index) => (
                  <div
                    key={`${item.date}-label`}
                    className="min-w-0 flex-1 text-center text-[9px] text-muted-foreground"
                  >
                    {labelIndexes.has(index) ? formatUsageDayLabel(item.date) : ""}
                  </div>
                ))}
              </div>
            </div>
          </div>
          <p className={cn("text-[11px] text-muted-foreground")}>
            {t("usage.totalTokens")}{" "}
            <span className="font-mono text-foreground">
              {formatUsageTokenCount(days.reduce((sum, item) => sum + item.total_tokens, 0))}
            </span>
          </p>
        </div>
      )}
    </SettingCard>
  );
}
