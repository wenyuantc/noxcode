import { useEffect, useState } from "react";
import {
  Activity,
  AlertCircle,
  ArrowDownLeft,
  ArrowRight,
  ArrowUpRight,
  Ban,
  BarChart3,
  CheckCircle2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { listNativeApiCallLogs } from "@/lib/backend";
import type { NativeApiCallLogStats } from "@/lib/types";
import { cn, formatTokenCount } from "@/lib/utils";
import { buttonVariants } from "@/components/ui/button";
import { SettingCard } from "./SettingCard";

export function UsageSection() {
  const { t } = useTranslation("settings");
  const [stats, setStats] = useState<NativeApiCallLogStats | null>(null);

  useEffect(() => {
    void listNativeApiCallLogs({ limit: 1, include_total: true }).then((page) =>
      setStats(page.stats),
    );
  }, []);

  const total = stats?.total ?? 0;
  const success = stats?.success ?? 0;
  const failed = stats?.failed ?? 0;
  const successRate = total > 0 ? Math.round((success / total) * 100) : 100;

  return (
    <div className="space-y-6">
      <SettingCard
        icon={BarChart3}
        title={t("usage.title")}
        description={t("usage.hint")}
        badge={total > 0 ? `成功率 ${successRate}%` : undefined}
        headerAction={
          <Link
            to="/api-logs"
            className={cn(buttonVariants({ variant: "outline", size: "sm" }), "h-7 gap-1 text-xs")}
          >
            {t("usage.openLogs")}
            <ArrowRight className="size-3" />
          </Link>
        }
      >
        <div className="space-y-4">
          {/* 状态统计卡片网格 */}
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <StatCard
              icon={Activity}
              label={t("usage.total")}
              value={stats?.total ?? 0}
              color="text-foreground"
            />
            <StatCard
              icon={CheckCircle2}
              label={t("usage.success")}
              value={stats?.success ?? 0}
              color="text-emerald-500"
              bg="bg-emerald-500/10 border-emerald-500/20"
            />
            <StatCard
              icon={AlertCircle}
              label={t("usage.failed")}
              value={stats?.failed ?? 0}
              color="text-destructive"
              bg="bg-destructive/10 border-destructive/20"
            />
            <StatCard
              icon={Ban}
              label={t("usage.cancelled")}
              value={stats?.cancelled ?? 0}
              color="text-muted-foreground"
            />
          </div>

          {/* Token 消耗网格 */}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="flex items-center gap-3.5 rounded-xl border border-border/70 bg-card p-4 shadow-2xs">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                <ArrowDownLeft className="size-4" />
              </div>
              <div className="space-y-0.5">
                <p className="text-[11px] text-muted-foreground font-medium">
                  {t("usage.inputTokens")}
                </p>
                <p className="text-xl font-bold font-mono tracking-tight text-foreground">
                  {stats ? formatTokenCount(stats.input_tokens) : "—"}
                </p>
              </div>
            </div>

            <div className="flex items-center gap-3.5 rounded-xl border border-border/70 bg-card p-4 shadow-2xs">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                <ArrowUpRight className="size-4" />
              </div>
              <div className="space-y-0.5">
                <p className="text-[11px] text-muted-foreground font-medium">
                  {t("usage.outputTokens")}
                </p>
                <p className="text-xl font-bold font-mono tracking-tight text-foreground">
                  {stats ? formatTokenCount(stats.output_tokens) : "—"}
                </p>
              </div>
            </div>
          </div>

          {/* 进度率指示 */}
          {total > 0 ? (
            <div className="rounded-xl border border-border/60 bg-muted/20 p-3.5 space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="text-muted-foreground font-medium">请求调用健康度</span>
                <span className="font-mono text-emerald-600 dark:text-emerald-400 font-semibold">
                  {successRate}% 成功率
                </span>
              </div>
              <div className="h-2 w-full overflow-hidden rounded-full bg-muted/60 flex">
                <div
                  className="h-full bg-emerald-500 transition-all"
                  style={{ width: `${successRate}%` }}
                />
                {failed > 0 ? (
                  <div
                    className="h-full bg-destructive transition-all"
                    style={{ width: `${100 - successRate}%` }}
                  />
                ) : null}
              </div>
            </div>
          ) : null}
        </div>
      </SettingCard>
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
