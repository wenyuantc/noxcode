import { Layers } from "lucide-react";
import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";
import type { NativeUsageModelBucket } from "@/lib/types";
import {
  displayUsageModelName,
  formatUsageTokenCount,
  formatUsageTokenExact,
  mergeUsageModels,
} from "@/lib/usageAnalytics";

export function UsageModelCard({
  models,
  empty,
}: {
  models: NativeUsageModelBucket[];
  empty: boolean;
}) {
  const { t } = useTranslation("settings");
  const rows = mergeUsageModels(models);
  const maxTokens = Math.max(1, ...rows.map((item) => item.total_tokens));
  const allTokens = rows.reduce((sum, item) => sum + item.total_tokens, 0);

  return (
    <SettingCard icon={Layers} title={t("usage.modelsTitle")} description={t("usage.modelsHint")}>
      {empty ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("usage.emptyRange")}</p>
      ) : (
        <div className="space-y-3">
          {rows.map((item) => {
            const name = displayUsageModelName(
              item.model,
              t("usage.unknownModel"),
              t("usage.otherModels"),
            );
            const share = allTokens > 0 ? (item.total_tokens / allTokens) * 100 : 0;
            const width = (item.total_tokens / maxTokens) * 100;
            return (
              <div
                key={item.model || name}
                className="space-y-1.5"
                title={`${name} · ${formatUsageTokenExact(item.total_tokens)} tokens`}
              >
                <div className="flex items-center justify-between gap-3 text-xs">
                  <span className="truncate font-medium text-foreground">{name}</span>
                  <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                    {formatUsageTokenCount(item.total_tokens)} ·{" "}
                    {t("usage.callsCount", { count: item.calls })} · {share.toFixed(1)}%
                  </span>
                </div>
                <div className="h-2 overflow-hidden rounded-full bg-muted/60">
                  <div
                    className="h-full rounded-full bg-primary transition-all"
                    style={{ width: `${Math.max(width, item.total_tokens > 0 ? 2 : 0)}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </SettingCard>
  );
}
