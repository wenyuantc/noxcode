import { useState, type MouseEvent } from "react";
import { Layers } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import { UsageTooltip, UsageTooltipProvider } from "./UsageTooltip";
import type { NativeUsageModelBucket } from "@/lib/types";
import {
  buildUsageDonutSlices,
  displayUsageModelName,
  formatUsageTokenCompact,
  getUsageModelColor,
  mergeUsageModels,
  USAGE_OTHER_MODEL_ID,
} from "@/lib/usageAnalytics";
import { cn } from "@/lib/utils";

export function UsageModelCard({
  models,
  empty,
}: {
  models: NativeUsageModelBucket[];
  empty: boolean;
}) {
  const { t, i18n } = useTranslation("settings");
  const [hoveredModel, setHoveredModel] = useState<string | null>(null);
  const [chartMousePos, setChartMousePos] = useState<{ x: number; y: number } | null>(null);

  const unknownLabel = t("usage.unknownModel");
  const otherLabel = t("usage.otherModels");
  const rows = mergeUsageModels(models);
  const { slices, allTokens } = buildUsageDonutSlices(models, unknownLabel, otherLabel);
  const maxTokens = Math.max(1, ...rows.map((item) => item.total_tokens));

  const hoveredSlice = slices.find((s) => s.model === hoveredModel);

  const handleChartMouseMove = (e: MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    setChartMousePos({
      x: e.clientX - rect.left,
      y: e.clientY - rect.top,
    });
  };

  const handleChartMouseLeave = () => {
    setHoveredModel(null);
    setChartMousePos(null);
  };

  return (
    <SettingCard icon={Layers} title={t("usage.modelsTitle")} description={t("usage.modelsHint")}>
      {empty ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
      ) : (
        <UsageTooltipProvider>
          <div className="grid grid-cols-1 items-center gap-6 md:grid-cols-[180px_1fr] lg:grid-cols-[200px_1fr]">
            {/* Left: Donut Chart */}
            <div
              className="relative mx-auto flex size-44 items-center justify-center shrink-0 sm:size-48"
              onMouseMove={handleChartMouseMove}
              onMouseLeave={handleChartMouseLeave}
            >
              <svg viewBox="0 0 160 160" className="size-full">
                {slices.length === 0 ? (
                  <circle
                    cx="80"
                    cy="80"
                    r="57"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="22"
                    className="text-muted/20"
                  />
                ) : (
                  slices.map((slice) => {
                    const isHovered = hoveredModel === slice.model;
                    const isOtherHovered = hoveredModel !== null && !isHovered;
                    return (
                      <path
                        key={slice.model}
                        d={slice.path}
                        fill={slice.color}
                        stroke="var(--color-card, #18181b)"
                        strokeWidth={2.5}
                        className={cn(
                          "cursor-pointer transition-all duration-150",
                          isHovered
                            ? "opacity-100 filter brightness-110 drop-shadow-md"
                            : isOtherHovered
                              ? "opacity-35"
                              : "opacity-90 hover:opacity-100",
                        )}
                        onMouseEnter={() => setHoveredModel(slice.model)}
                      />
                    );
                  })
                )}
              </svg>

              {/* Center Token Count */}
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center text-center">
                <span className="font-mono text-base font-bold tracking-tight text-foreground leading-tight sm:text-lg">
                  {formatUsageTokenCompact(allTokens, i18n.language)}
                </span>
                <span className="text-[11px] font-medium text-muted-foreground">tokens</span>
              </div>

              {/* Chart Hover Tooltip */}
              {hoveredSlice && chartMousePos && (
                <div
                  className="pointer-events-none absolute z-50 min-w-36 max-w-xs -translate-x-1/2 -translate-y-full rounded-lg border border-neutral-800 bg-neutral-900/95 px-3 py-2 text-xs text-neutral-100 shadow-xl backdrop-blur-xs transition-opacity duration-150"
                  style={{
                    left: chartMousePos.x,
                    top: chartMousePos.y - 8,
                  }}
                >
                  <div className="space-y-1 py-0.5">
                    <div className="flex items-center gap-1.5">
                      <span
                        className="size-2 rounded-full"
                        style={{ backgroundColor: hoveredSlice.color }}
                      />
                      <span className="font-semibold text-neutral-100">{hoveredSlice.name}</span>
                    </div>
                    <p className="font-mono text-[11px] text-neutral-400">
                      <span className="font-semibold text-neutral-100">
                        {formatUsageTokenCompact(hoveredSlice.tokens, i18n.language)}
                      </span>{" "}
                      tokens · {hoveredSlice.percentage.toFixed(1)}% ·{" "}
                      {t("usage.callsCount", { count: hoveredSlice.calls })}
                    </p>
                  </div>
                </div>
              )}
            </div>

            {/* Right: Models List */}
            <div className="min-w-0 space-y-2">
              {rows.map((item, index) => {
                const name = displayUsageModelName(item.model, unknownLabel, otherLabel);
                const isOther = item.model === USAGE_OTHER_MODEL_ID;
                const color = getUsageModelColor(index, isOther);
                const isHovered = hoveredModel === item.model;
                const isDimmed = hoveredModel !== null && !isHovered;
                const share = allTokens > 0 ? (item.total_tokens / allTokens) * 100 : 0;
                const width = (item.total_tokens / maxTokens) * 100;

                return (
                  <UsageTooltip
                    key={item.model || name}
                    content={
                      <div className="space-y-1 py-0.5">
                        <div className="flex items-center gap-1.5">
                          <span
                            className="size-2 rounded-full"
                            style={{ backgroundColor: color }}
                          />
                          <span className="font-semibold text-neutral-100">{name}</span>
                        </div>
                        <p className="font-mono text-[11px] text-neutral-400">
                          <span className="font-semibold text-neutral-100">
                            {formatUsageTokenCompact(item.total_tokens, i18n.language)}
                          </span>{" "}
                          tokens · {share.toFixed(1)}% ·{" "}
                          {t("usage.callsCount", { count: item.calls })}
                        </p>
                      </div>
                    }
                  >
                    <div
                      className={cn(
                        "space-y-1.5 rounded-lg p-1.5 transition-all cursor-pointer",
                        isHovered ? "bg-muted/50" : isDimmed ? "opacity-45" : "opacity-100",
                      )}
                      onMouseEnter={() => setHoveredModel(item.model)}
                      onMouseLeave={() => setHoveredModel(null)}
                    >
                      <div className="flex items-center justify-between gap-3 text-xs">
                        <span className="flex items-center gap-1.5 truncate font-medium text-foreground">
                          <span
                            className="size-2 shrink-0 rounded-full"
                            style={{ backgroundColor: color }}
                          />
                          <span className="truncate">{name}</span>
                        </span>
                        <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                          {formatUsageTokenCompact(item.total_tokens, i18n.language)} ·{" "}
                          {t("usage.callsCount", { count: item.calls })} · {share.toFixed(1)}%
                        </span>
                      </div>
                      <div className="h-2 overflow-hidden rounded-full bg-muted/60">
                        <div
                          className="h-full rounded-full transition-all"
                          style={{
                            width: `${Math.max(width, item.total_tokens > 0 ? 2 : 0)}%`,
                            backgroundColor: color,
                          }}
                        />
                      </div>
                    </div>
                  </UsageTooltip>
                );
              })}
            </div>
          </div>
        </UsageTooltipProvider>
      )}
    </SettingCard>
  );
}
