import { BarChart3 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import { UsageTooltip, UsageTooltipProvider } from "./UsageTooltip";
import type { NativeUsageDailyBucket } from "@/lib/types";
import {
  formatUsageDateWithWeekday,
  formatUsageDayLabel,
  formatUsageTokenCompact,
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
  const { t, i18n } = useTranslation("settings");
  const maxTokens = Math.max(1, ...days.map((item) => item.input_tokens + item.output_tokens));
  const labelIndexes = new Set(usageTrendLabelIndexes(days.length));

  return (
    <SettingCard icon={BarChart3} title={t("usage.trendTitle")} description={t("usage.trendHint")}>
      {empty ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
      ) : (
        <UsageTooltipProvider>
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
                <span>{formatUsageTokenCompact(maxTokens, i18n.language)}</span>
                <span>{formatUsageTokenCompact(maxTokens / 2, i18n.language)}</span>
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
                      >
                        <UsageTooltip
                          content={
                            <div className="min-w-40 space-y-1.5 py-0.5">
                              <p className="font-medium text-neutral-200">
                                {formatUsageDateWithWeekday(item.date, i18n.language)}
                              </p>
                              <p className="font-mono text-[11px] text-neutral-400">
                                <span className="font-semibold text-neutral-100">
                                  {formatUsageTokenCompact(total, i18n.language)}
                                </span>{" "}
                                tokens · {t("usage.callsCount", { count: item.calls })}
                              </p>
                              <div className="flex items-center justify-between gap-3 border-t border-neutral-800 pt-1 text-[10px] font-mono">
                                <span className="inline-flex items-center gap-1 text-sky-400">
                                  <span className="size-1.5 rounded-full bg-sky-400" />
                                  {t("usage.inputTokens")}:{" "}
                                  {formatUsageTokenCompact(item.input_tokens, i18n.language)}
                                </span>
                                <span className="inline-flex items-center gap-1 text-violet-400">
                                  <span className="size-1.5 rounded-full bg-violet-400" />
                                  {t("usage.outputTokens")}:{" "}
                                  {formatUsageTokenCompact(item.output_tokens, i18n.language)}
                                </span>
                              </div>
                            </div>
                          }
                        >
                          <div className="group flex h-full w-full cursor-pointer flex-col justify-end">
                            <div className="flex h-full w-full flex-col justify-end overflow-hidden rounded-t-sm transition-all group-hover:brightness-125">
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
                        </UsageTooltip>
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
                {formatUsageTokenCompact(
                  days.reduce((sum, item) => sum + item.total_tokens, 0),
                  i18n.language,
                )}
              </span>
            </p>
          </div>
        </UsageTooltipProvider>
      )}
    </SettingCard>
  );
}
