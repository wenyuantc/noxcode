import { Activity } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import type { NativeUsageDailyBucket } from "@/lib/types";
import {
  buildHeatmapWeeks,
  formatUsageTokenCount,
  formatUsageTokenExact,
  type UsageHeatmapLevel,
} from "@/lib/usageAnalytics";
import { cn } from "@/lib/utils";

const LEVEL_CLASS: Record<UsageHeatmapLevel, string> = {
  0: "bg-muted/70",
  1: "bg-emerald-500/20",
  2: "bg-emerald-500/40",
  3: "bg-emerald-500/70",
  4: "bg-emerald-500",
};

export function UsageHeatmapCard({
  days,
  rangeStart,
  rangeEnd,
  empty,
}: {
  days: NativeUsageDailyBucket[];
  rangeStart: string;
  rangeEnd: string;
  empty: boolean;
}) {
  const { t } = useTranslation("settings");
  const weeks = buildHeatmapWeeks(days, rangeStart, rangeEnd);
  const weekdays = [
    t("usage.weekdayMon"),
    t("usage.weekdayTue"),
    t("usage.weekdayWed"),
    t("usage.weekdayThu"),
    t("usage.weekdayFri"),
    t("usage.weekdaySat"),
    t("usage.weekdaySun"),
  ];

  return (
    <SettingCard
      icon={Activity}
      title={t("usage.heatmapTitle")}
      description={t("usage.heatmapHint")}
    >
      {empty ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
      ) : (
        <div className="space-y-3">
          <div className="grid grid-cols-7 gap-1.5">
            {weekdays.map((label) => (
              <div
                key={label}
                className="text-center text-[10px] font-medium text-muted-foreground"
              >
                {label}
              </div>
            ))}
          </div>
          <div className="space-y-1.5">
            {weeks.map((week) => (
              <div key={week.cells[0]?.date} className="grid grid-cols-7 gap-1.5">
                {week.cells.map((cell) => (
                  <div
                    key={cell.date}
                    title={
                      cell.inRange
                        ? t("usage.tooltipDay", {
                            date: cell.date,
                            calls: cell.calls,
                            tokens: formatUsageTokenExact(cell.totalTokens),
                          })
                        : undefined
                    }
                    className={cn(
                      "aspect-square rounded-md border border-transparent transition-colors",
                      cell.inRange
                        ? cn("border-border/40", LEVEL_CLASS[cell.level])
                        : "bg-transparent opacity-30",
                    )}
                  />
                ))}
              </div>
            ))}
          </div>
          <div className="flex items-center justify-end gap-1.5 text-[10px] text-muted-foreground">
            <span>{t("usage.legendLess")}</span>
            {([0, 1, 2, 3, 4] as UsageHeatmapLevel[]).map((level) => (
              <span
                key={level}
                className={cn("size-2.5 rounded-[3px] border border-border/40", LEVEL_CLASS[level])}
              />
            ))}
            <span>{t("usage.legendMore")}</span>
            <span className="ml-1 font-mono">
              {formatUsageTokenCount(Math.max(0, ...days.map((item) => item.total_tokens)))}
            </span>
          </div>
        </div>
      )}
    </SettingCard>
  );
}
