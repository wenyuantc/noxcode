import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { listNativeApiCallLogs } from "@/lib/backend";
import type { NativeApiCallLogStats } from "@/lib/types";
import { formatTokenCount } from "@/lib/utils";
import { SettingCard } from "./SettingCard";

export function UsageSection() {
  const { t } = useTranslation("settings");
  const [stats, setStats] = useState<NativeApiCallLogStats | null>(null);

  useEffect(() => {
    void listNativeApiCallLogs({ limit: 1, include_total: true }).then((page) =>
      setStats(page.stats),
    );
  }, []);

  return (
    <SettingCard title={t("usage.title")} description={t("usage.hint")}>
      <div className="grid grid-cols-2 gap-3 text-sm">
        <Stat label={t("usage.total")} value={stats?.total} />
        <Stat label={t("usage.success")} value={stats?.success} />
        <Stat label={t("usage.failed")} value={stats?.failed} />
        <Stat label={t("usage.cancelled")} value={stats?.cancelled} />
        <Stat
          label={t("usage.inputTokens")}
          value={stats ? formatTokenCount(stats.input_tokens) : "—"}
        />
        <Stat
          label={t("usage.outputTokens")}
          value={stats ? formatTokenCount(stats.output_tokens) : "—"}
        />
      </div>
      <Link to="/api-logs" className="mt-3 inline-block text-sm underline">
        {t("usage.openLogs")}
      </Link>
    </SettingCard>
  );
}

function Stat({ label, value }: { label: string; value?: string | number | null }) {
  return (
    <div className="rounded-md border px-3 py-2">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-lg font-medium">{value ?? "—"}</p>
    </div>
  );
}
