import { useEffect, useState } from "react";
import { CheckCircle2, Download, Info, Loader2, RefreshCw, RotateCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  type AppUpdateInfo,
  type AppUpdateProgress,
  checkForAppUpdate,
  downloadAndInstallUpdate,
  getAppVersion,
  mapUpdaterError,
  relaunchApp,
  updaterErrorI18nKey,
} from "@/lib/appUpdate";
import { formatDate } from "@/lib/utils";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

export function AboutSection() {
  const { t } = useTranslation("settings");
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [availableUpdate, setAvailableUpdate] = useState<AppUpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [upToDate, setUpToDate] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<AppUpdateProgress | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getAppVersion()
      .then((version) => {
        if (!cancelled) {
          setCurrentVersion(version);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCurrentVersion(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = checking || installing;
  const displayVersion = currentVersion ?? t("about.unknownVersion");

  const setMappedError = (cause: unknown) => {
    const code = mapUpdaterError(cause);
    const detail = cause instanceof Error ? cause.message : String(cause ?? "");
    setError(t(updaterErrorI18nKey(code), { detail }));
  };

  const handleCheck = async () => {
    setChecking(true);
    setError(null);
    setUpToDate(false);
    setInstalled(false);
    setProgress(null);
    try {
      const update = await checkForAppUpdate();
      if (!update) {
        setAvailableUpdate(null);
        setUpToDate(true);
        return;
      }
      setAvailableUpdate(update);
    } catch (cause) {
      setAvailableUpdate(null);
      setMappedError(cause);
    } finally {
      setChecking(false);
    }
  };

  const handleInstall = async () => {
    if (!availableUpdate) {
      return;
    }
    setInstalling(true);
    setError(null);
    setProgress({ downloaded: 0, total: null, percent: null });
    try {
      await downloadAndInstallUpdate(availableUpdate, setProgress);
      setInstalled(true);
    } catch (cause) {
      setMappedError(cause);
    } finally {
      setInstalling(false);
    }
  };

  const handleRestart = async () => {
    setError(null);
    try {
      await relaunchApp();
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : String(cause ?? "");
      setError(t("about.restartFailed", { detail }));
    }
  };

  return (
    <div className="space-y-6">
      {error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      {/* 品牌与版本横幅卡片 */}
      <SettingCard
        icon={Info}
        title={t("about.title")}
        description={t("about.description")}
        badge={`v${displayVersion}`}
        headerAction={
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void handleCheck()}
            className="h-7 text-xs gap-1"
          >
            {checking ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            {checking ? t("about.checking") : t("about.check")}
          </Button>
        }
      >
        <div className="space-y-6">
          {/* 品牌视觉展示 */}
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 rounded-xl border border-border/70 bg-gradient-to-br from-card to-muted/30 p-5 shadow-2xs">
            <div className="space-y-1">
              <div className="flex items-center gap-2.5">
                <span className="text-xl font-bold tracking-tight text-foreground font-mono">
                  noxcode
                </span>
                <span className="rounded-full border border-primary/30 bg-primary/10 px-2.5 py-0.5 font-mono text-[11px] font-semibold text-primary">
                  v{displayVersion}
                </span>
              </div>
              <p className="text-xs text-muted-foreground leading-relaxed">
                本地优先、极速响应、深度内嵌 Git 与 SSH 的智能编程工作空间。
              </p>
            </div>
          </div>

          {/* 已是最新版本提示 */}
          {upToDate && !availableUpdate ? (
            <div className="flex items-center gap-3 rounded-xl border border-emerald-500/25 bg-emerald-500/10 p-3.5 text-xs text-emerald-950 dark:text-emerald-100">
              <CheckCircle2 className="size-4 shrink-0 text-emerald-500" />
              <span>{t("about.upToDate")}</span>
            </div>
          ) : null}

          {/* 新版本可用卡片 */}
          {availableUpdate ? (
            <div className="space-y-4 rounded-xl border border-primary/30 bg-primary/5 p-4 shadow-xs">
              <div className="flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-semibold text-foreground">
                      {t("about.available", { version: availableUpdate.version })}
                    </span>
                    <span className="rounded-full bg-primary/20 px-2 py-0.2 text-[10px] font-mono text-primary font-semibold">
                      New
                    </span>
                  </div>
                  {availableUpdate.pubDate ? (
                    <p className="mt-0.5 text-[11px] text-muted-foreground font-mono">
                      {t("about.publishedAt", { date: formatDate(availableUpdate.pubDate) })}
                    </p>
                  ) : null}
                </div>

                {!installed ? (
                  <Button
                    type="button"
                    size="sm"
                    disabled={busy}
                    onClick={() => void handleInstall()}
                    className="h-8 text-xs gap-1.5"
                  >
                    {installing ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : (
                      <Download className="size-3.5" />
                    )}
                    {installing ? t("about.downloading") : t("about.update")}
                  </Button>
                ) : (
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => void handleRestart()}
                    className="h-8 text-xs gap-1.5"
                  >
                    <RotateCw className="size-3.5" />
                    {t("about.restart")}
                  </Button>
                )}
              </div>

              {availableUpdate.notes ? (
                <div className="rounded-lg border border-border/60 bg-background/80 p-3">
                  <p className="mb-1.5 text-[11px] font-semibold text-muted-foreground">
                    {t("about.notes")}
                  </p>
                  <pre className="max-h-40 overflow-auto whitespace-pre-wrap font-sans text-xs leading-relaxed text-foreground">
                    {availableUpdate.notes}
                  </pre>
                </div>
              ) : null}

              {installing && progress ? (
                <div className="space-y-1.5 pt-1">
                  <div
                    className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
                    role="progressbar"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={progress.percent ?? undefined}
                    aria-label={t("about.downloading")}
                  >
                    {progress.percent != null ? (
                      <div
                        className="h-full bg-primary transition-all duration-200"
                        style={{ width: `${progress.percent}%` }}
                      />
                    ) : (
                      <div className="h-full w-full animate-pulse bg-primary/80" />
                    )}
                  </div>
                  <p className="text-[11px] text-muted-foreground font-mono text-right">
                    {progress.percent != null
                      ? t("about.progressPercent", { percent: progress.percent })
                      : t("about.downloading")}
                  </p>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      </SettingCard>
    </div>
  );
}
