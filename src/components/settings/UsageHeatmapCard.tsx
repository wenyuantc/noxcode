import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import { UsageTooltip, UsageTooltipProvider } from "./UsageTooltip";
import { getNativeUsageAnalytics } from "@/lib/backend";
import type { NativeUsageDailyBucket } from "@/lib/types";
import {
  buildHeatmapMonthLabels,
  buildHeatmapWeeks,
  formatUsageDateWithWeekday,
  formatUsageTokenCompact,
  formatUsageTokenExact,
  resolveHeatmapYearRange,
  type UsageHeatmapLevel,
} from "@/lib/usageAnalytics";
import { cn } from "@/lib/utils";

const GREEN_LEVEL_CLASS: Record<UsageHeatmapLevel, string> = {
  0: "bg-neutral-200 dark:bg-[#25272c]",
  1: "bg-emerald-200 dark:bg-[#133e24]",
  2: "bg-emerald-400 dark:bg-[#166534]",
  3: "bg-emerald-600 dark:bg-[#15803d]",
  4: "bg-emerald-500 dark:bg-[#22c55e]",
};

export function UsageHeatmapCard({
  activeRangeStart: _activeRangeStart,
  activeRangeEnd: _activeRangeEnd,
}: {
  activeRangeStart?: string;
  activeRangeEnd?: string;
  days?: NativeUsageDailyBucket[];
  rangeStart?: string;
  rangeEnd?: string;
  empty?: boolean;
}) {
  const { t, i18n } = useTranslation("settings");
  const currentYear = new Date().getUTCFullYear();
  const [selectedYear, setSelectedYear] = useState<string>("lastYear");
  const [dailyBuckets, setDailyBuckets] = useState<NativeUsageDailyBucket[]>([]);
  const [, setLoading] = useState(true);

  const yearOptions = useMemo(
    () => [
      { key: "lastYear", label: t("usage.pastYear") },
      { key: String(currentYear), label: String(currentYear) },
      { key: String(currentYear - 1), label: String(currentYear - 1) },
    ],
    [currentYear, t],
  );

  const yearRange = useMemo(() => resolveHeatmapYearRange(selectedYear), [selectedYear]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void getNativeUsageAnalytics({
      start_date: yearRange.start,
      end_date: yearRange.end,
    })
      .then((result) => {
        if (!cancelled) {
          setDailyBuckets(result.daily);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setDailyBuckets([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [yearRange.end, yearRange.start]);

  const weeks = useMemo(
    () => buildHeatmapWeeks(dailyBuckets, yearRange.start, yearRange.end),
    [dailyBuckets, yearRange.end, yearRange.start],
  );

  const monthLabels = useMemo(
    () => buildHeatmapMonthLabels(weeks, i18n.language),
    [weeks, i18n.language],
  );

  const totalTokens = useMemo(
    () => dailyBuckets.reduce((sum, item) => sum + item.total_tokens, 0),
    [dailyBuckets],
  );

  const totalCalls = useMemo(
    () => dailyBuckets.reduce((sum, item) => sum + item.calls, 0),
    [dailyBuckets],
  );

  const headerDescription = useMemo(() => {
    if (totalTokens > 0 || totalCalls > 0) {
      return t("usage.heatmapYearSummary", {
        tokens: formatUsageTokenCompact(totalTokens, i18n.language),
        calls: totalCalls.toLocaleString(),
      });
    }
    return t("usage.heatmapHint");
  }, [totalTokens, totalCalls, t, i18n.language]);

  return (
    <SettingCard
      title={t("usage.heatmapTitle")}
      description={headerDescription}
      headerAction={
        <div className="inline-flex rounded-full border border-border/50 bg-neutral-900/90 p-0.5 shadow-xs">
          {yearOptions.map((item) => {
            const isSelected = selectedYear === item.key;
            return (
              <button
                key={item.key}
                type="button"
                className={cn(
                  "px-3 py-0.5 text-xs rounded-full font-medium transition-all",
                  isSelected
                    ? "bg-neutral-800 text-neutral-100 shadow-xs"
                    : "text-neutral-400 hover:text-neutral-200",
                )}
                onClick={() => setSelectedYear(item.key)}
              >
                {item.label}
              </button>
            );
          })}
        </div>
      }
    >
      <UsageTooltipProvider>
        <div className="space-y-2">
          <div className="overflow-x-auto pb-1 scrollbar-thin">
            <div className="inline-block min-w-fit">
              {/* Weeks grid: 52-53 columns x 7 rows of uncompressed 13px squares */}
              <div className="flex gap-[3px]">
                {weeks.map((week) => (
                  <div key={week.cells[0]?.date} className="flex flex-col gap-[3px] shrink-0">
                    {week.cells.map((cell) => (
                      <UsageTooltip
                        key={cell.date}
                        triggerClassName="size-[13px] shrink-0"
                        content={
                          <div className="space-y-1 py-0.5">
                            <p className="font-medium text-neutral-200">
                              {formatUsageDateWithWeekday(cell.date, i18n.language)}
                            </p>
                            <p className="font-mono text-neutral-400 text-[11px]">
                              {cell.totalTokens > 0 ? (
                                <>
                                  <span
                                    className="font-semibold text-neutral-100"
                                    title={`${formatUsageTokenExact(cell.totalTokens)} tokens`}
                                  >
                                    {formatUsageTokenCompact(cell.totalTokens, i18n.language)}
                                  </span>{" "}
                                  tokens · {t("usage.callsCount", { count: cell.calls })}
                                </>
                              ) : (
                                <span className="text-neutral-400">{t("usage.noActivity")}</span>
                              )}
                            </p>
                          </div>
                        }
                      >
                        <div
                          className={cn(
                            "size-full rounded-[3px] transition-all cursor-pointer hover:brightness-125",
                            GREEN_LEVEL_CLASS[cell.level],
                          )}
                        />
                      </UsageTooltip>
                    ))}
                  </div>
                ))}
              </div>

              {/* Month labels at the bottom, precisely aligned to column offset */}
              <div className="relative mt-2.5 h-4 text-[11px] text-neutral-400 select-none">
                {monthLabels.map((item) => (
                  <span
                    key={`${item.colIndex}-${item.label}`}
                    className="absolute whitespace-nowrap"
                    style={{ left: `${item.colIndex * 16}px` }}
                  >
                    {item.label}
                  </span>
                ))}
              </div>
            </div>
          </div>
        </div>
      </UsageTooltipProvider>
    </SettingCard>
  );
}
