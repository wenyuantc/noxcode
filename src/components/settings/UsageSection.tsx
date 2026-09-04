import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  AlertCircle,
  ArrowDownLeft,
  ArrowRight,
  ArrowUpRight,
  Ban,
  BarChart3,
  CheckCircle2,
  Hash,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatApiCallLogCacheRate } from "@/lib/apiLogs";
import { getNativeUsageAnalytics } from "@/lib/backend";
import {
  emptyUsageAnalytics,
  fillUsageDailyBuckets,
  formatUsageTokenCount,
  formatUsageTokenExact,
  resolveUsageDateRange,
  usageAnalyticsLoadError,
  usageCacheHitRatio,
  usageTotalTokens,
  type UsageRangePreset,
} from "@/lib/usageAnalytics";
import { cn } from "@/lib/utils";

import { SettingCard } from "./SettingCard";
import { UsageHeatmapCard } from "./UsageHeatmapCard";
import { UsageModelCard } from "./UsageModelCard";
import { UsageTrendCard } from "./UsageTrendCard";

const RANGE_PRESETS: UsageRangePreset[] = ["7d", "30d", "custom"];

export function UsageSection() {
  const { t } = useTranslation("settings");
  const [preset, setPreset] = useState<UsageRangePreset>("7d");
  const [customStart, setCustomStart] = useState("");
  const [customEnd, setCustomEnd] = useState("");
  const [analytics, setAnalytics] = useState(emptyUsageAnalytics);
  const [loading, setLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const range = useMemo(
    () => resolveUsageDateRange(preset, customStart, customEnd),
    [customEnd, customStart, preset],
  );

  useEffect(() => {
    if (preset !== "custom") {
      return;
    }
    const fallback = resolveUsageDateRange("7d", "", "");
    if (!fallback.ok) {
      return;
    }
    setCustomStart((current) => current || fallback.start);
    setCustomEnd((current) => current || fallback.end);
  }, [preset]);

  useEffect(() => {
    if (!range.ok) {
      setAnalytics(emptyUsageAnalytics());
      setLoading(false);
      if (range.reason === "order") {
        setErrorMessage(t("usage.invalidDateOrder"));
      } else if (range.reason === "span") {
        setErrorMessage(t("usage.invalidDateSpan"));
      } else {
        setErrorMessage(t("usage.incompleteCustomRange"));
      }
      return;
    }

    let cancelled = false;
    setLoading(true);
    setErrorMessage(null);
    void getNativeUsageAnalytics({
      start_date: range.start,
      end_date: range.end,
    })
      .then((result) => {
        if (!cancelled) {
          setAnalytics(result);
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setAnalytics(emptyUsageAnalytics());
        setErrorMessage(usageAnalyticsLoadError(error, t("usage.loadFailed")));
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [range, t]);

  const stats = analytics.stats;
  const total = stats.total;
  const success = stats.success;
  const failed = stats.failed;
  const successRate = total > 0 ? Math.round((success / total) * 100) : 100;
  const totalTokens = usageTotalTokens(stats);
  const cacheRatio = usageCacheHitRatio(stats.input_tokens, stats.cached_tokens_sum);
  const cacheRateLabel = formatApiCallLogCacheRate(stats.input_tokens, stats.cached_tokens_sum, {
    unknown: "—",
    empty: "—",
  });
  const filledDays = range.ok ? fillUsageDailyBuckets(range.start, range.end, analytics.daily) : [];
  const showEmptyCharts = !loading && (!range.ok || total === 0);

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex flex-wrap items-center gap-2">
          <div className="inline-flex rounded-lg border border-border/70 bg-muted/30 p-0.5">
            {RANGE_PRESETS.map((item) => (
              <Button
                key={item}
                type="button"
                size="sm"
                variant={preset === item ? "secondary" : "ghost"}
                className="h-7 px-2.5 text-xs"
                onClick={() => setPreset(item)}
              >
                {t(`usage.range${item === "7d" ? "7d" : item === "30d" ? "30d" : "Custom"}`)}
              </Button>
            ))}
          </div>
          {preset === "custom" ? (
            <div className="flex flex-wrap items-center gap-2">
              <Input
                type="date"
                aria-label={t("usage.startDate")}
                className="h-7 w-[9.5rem] bg-background text-xs"
                value={customStart}
                onChange={(event) => setCustomStart(event.target.value)}
              />
              <span className="text-xs text-muted-foreground">–</span>
              <Input
                type="date"
                aria-label={t("usage.endDate")}
                className="h-7 w-[9.5rem] bg-background text-xs"
                value={customEnd}
                onChange={(event) => setCustomEnd(event.target.value)}
              />
            </div>
          ) : range.ok ? (
            <p className="text-xs text-muted-foreground">
              {range.start} – {range.end}
            </p>
          ) : null}
        </div>
        <Link
          to="/api-logs"
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "h-7 gap-1 text-xs")}
        >
          {t("usage.openLogs")}
          <ArrowRight className="size-3" />
        </Link>
      </div>

      {errorMessage ? (
        <div className="flex items-center gap-2.5 rounded-xl border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive">
          <AlertCircle className="size-4 shrink-0" />
          <span>{errorMessage}</span>
        </div>
      ) : null}

      <SettingCard
        icon={BarChart3}
        title={t("usage.title")}
        description={t("usage.hint")}
        badge={
          loading
            ? t("usage.loading")
            : total > 0
              ? t("usage.healthValue", { rate: successRate })
              : undefined
        }
      >
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <StatCard
              icon={Activity}
              label={t("usage.total")}
              value={stats.total}
              color="text-foreground"
            />
            <StatCard
              icon={CheckCircle2}
              label={t("usage.success")}
              value={stats.success}
              color="text-emerald-500"
              bg="bg-emerald-500/10 border-emerald-500/20"
            />
            <StatCard
              icon={AlertCircle}
              label={t("usage.failed")}
              value={stats.failed}
              color="text-destructive"
              bg="bg-destructive/10 border-destructive/20"
            />
            <StatCard
              icon={Ban}
              label={t("usage.cancelled")}
              value={stats.cancelled}
              color="text-muted-foreground"
            />
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <TokenStat icon={Hash} label={t("usage.totalTokens")} value={totalTokens} />
            <TokenStat
              icon={ArrowDownLeft}
              label={t("usage.inputTokens")}
              value={stats.input_tokens}
            />
            <TokenStat
              icon={ArrowUpRight}
              label={t("usage.outputTokens")}
              value={stats.output_tokens}
            />
          </div>

          {total > 0 ? (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <RateBar
                label={t("usage.healthLabel")}
                valueLabel={t("usage.healthValue", { rate: successRate })}
                percent={successRate}
                tone="health"
                failed={failed > 0}
              />
              <RateBar
                label={t("usage.cacheLabel")}
                valueLabel={t("usage.cacheValue", { rate: cacheRateLabel })}
                percent={(cacheRatio ?? 0) * 100}
                tone="cache"
              />
            </div>
          ) : !loading && range.ok && !errorMessage ? (
            <p className="text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
          ) : null}
        </div>
      </SettingCard>

      <UsageHeatmapCard
        days={filledDays}
        rangeStart={range.ok ? range.start : ""}
        rangeEnd={range.ok ? range.end : ""}
        empty={showEmptyCharts}
      />
      <UsageTrendCard days={filledDays} empty={showEmptyCharts} />
      <UsageModelCard models={analytics.models} empty={showEmptyCharts} />
    </div>
  );
}

function StatCard({
  icon: Icon,
  label,
  value,
  color,
  bg,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string | number;
  color?: string;
  bg?: string;
}) {
  return (
    <div
      className={`flex flex-col justify-between rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-border ${
        bg ?? ""
      }`}
    >
      <div className="flex items-center justify-between text-muted-foreground">
        <span className="text-[11px] font-medium">{label}</span>
        <Icon className={`size-3.5 ${color ?? ""}`} />
      </div>
      <p
        className={`mt-2 text-xl font-bold font-mono tracking-tight ${color ?? "text-foreground"}`}
      >
        {value}
      </p>
    </div>
  );
}

function TokenStat({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: number;
}) {
  return (
    <div className="flex items-center gap-3.5 rounded-xl border border-border/70 bg-card p-4 shadow-2xs">
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
        <Icon className="size-4" />
      </div>
      <div className="space-y-0.5">
        <p className="text-[11px] font-medium text-muted-foreground">{label}</p>
        <p
          className="font-mono text-xl font-bold tracking-tight text-foreground"
          title={formatUsageTokenExact(value)}
        >
          {formatUsageTokenCount(value)}
        </p>
      </div>
    </div>
  );
}

function RateBar({
  label,
  valueLabel,
  percent,
  tone,
  failed = false,
}: {
  label: string;
  valueLabel: string;
  percent: number;
  tone: "health" | "cache";
  failed?: boolean;
}) {
  const width = Math.max(0, Math.min(100, percent));
  return (
    <div className="space-y-2 rounded-xl border border-border/60 bg-muted/20 p-3.5">
      <div className="flex items-center justify-between text-xs">
        <span className="font-medium text-muted-foreground">{label}</span>
        <span
          className={cn(
            "font-mono font-semibold",
            tone === "health"
              ? "text-emerald-600 dark:text-emerald-400"
              : "text-amber-600 dark:text-amber-400",
          )}
        >
          {valueLabel}
        </span>
      </div>
      <div className="flex h-2 w-full overflow-hidden rounded-full bg-muted/60">
        <div
          className={cn(
            "h-full transition-all",
            tone === "health" ? "bg-emerald-500" : "bg-amber-500",
          )}
          style={{ width: `${width}%` }}
        />
        {tone === "health" && failed ? (
          <div
            className="h-full bg-destructive transition-all"
            style={{ width: `${100 - width}%` }}
          />
        ) : null}
      </div>
    </div>
  );
}
