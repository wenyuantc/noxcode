import { useEffect, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
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
    <SettingCard
      title={t("about.title")}
      description={t("about.description")}
      badge={displayVersion}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="text-sm">
          <span className="text-muted-foreground">{t("about.currentVersion")}: </span>
          <span className="font-medium">{displayVersion}</span>
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => void handleCheck()}
        >
          {checking ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
          {checking ? t("about.checking") : t("about.check")}
        </Button>
      </div>

      {upToDate && !availableUpdate ? (
        <p className="mt-3 text-sm text-muted-foreground">{t("about.upToDate")}</p>
      ) : null}

      {availableUpdate ? (
        <div className="mt-3 space-y-3 rounded-md border border-border bg-background/60 p-3">
          <div>
            <p className="text-sm font-medium">
              {t("about.available", { version: availableUpdate.version })}
            </p>
            {availableUpdate.pubDate ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("about.publishedAt", { date: formatDate(availableUpdate.pubDate) })}
              </p>
            ) : null}
          </div>
          {availableUpdate.notes ? (
            <div>
              <p className="mb-1 text-xs font-medium text-muted-foreground">{t("about.notes")}</p>
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap text-xs text-foreground">
                {availableUpdate.notes}
              </pre>
            </div>
          ) : null}
          {installing && progress ? (
            <div className="space-y-1">
              <div
                className="h-1 w-full overflow-hidden rounded-full bg-muted"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={progress.percent ?? undefined}
                aria-label={t("about.downloading")}
              >
                {progress.percent != null ? (
                  <div
                    className="h-full bg-primary transition-all"
                    style={{ width: `${progress.percent}%` }}
                  />
                ) : (
                  <div className="h-full w-full animate-pulse bg-primary/80" />
                )}
              </div>
              <p className="text-xs text-muted-foreground">
                {progress.percent != null
                  ? t("about.progressPercent", { percent: progress.percent })
                  : t("about.downloading")}
              </p>
            </div>
          ) : null}
          {installed ? (
            <div className="flex flex-wrap items-center gap-2">
              <p className="text-sm">{t("about.installed")}</p>
              <Button type="button" size="sm" onClick={() => void handleRestart()}>
                {t("about.restart")}
              </Button>
            </div>
          ) : (
            <Button type="button" size="sm" disabled={busy} onClick={() => void handleInstall()}>
              {installing ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {installing ? t("about.downloading") : t("about.update")}
            </Button>
          )}
        </div>
      ) : null}

      {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}
    </SettingCard>
  );
}
