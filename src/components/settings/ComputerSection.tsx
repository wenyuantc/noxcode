import { AlertTriangle, Loader2, Monitor, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  getComputerPermissionStatus,
  openComputerPrivacySettings,
  updateNativeSettings,
} from "@/lib/backend";
import { errorMessage, runToastAction } from "@/lib/toast";
import type { ComputerPermissionFlag, ComputerPermissionStatus } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard, SettingRow } from "./SettingCard";

export function computerFlagLabel(flag: ComputerPermissionFlag): string {
  return flag.label;
}

export function ComputerSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [status, setStatus] = useState<ComputerPermissionStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [opening, setOpening] = useState(false);

  const refreshStatus = useCallback(async () => {
    setStatusLoading(true);
    try {
      setStatus(await getComputerPermissionStatus());
      setStatusError(null);
    } catch (error) {
      setStatusError(errorMessage(error));
    } finally {
      setStatusLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const persistEnabled = useCallback(
    (checked: boolean) => {
      void runToastAction(
        () =>
          updateNativeSettings({ computer_control_enabled: checked }).then((updated) =>
            setNative(updated),
          ),
        {
          id: "computer-enabled-save",
          successMessage: t("common:saved"),
        },
      );
    },
    [setNative, t],
  );

  const handleOpenSettings = useCallback(async () => {
    if (opening) return;
    setOpening(true);
    try {
      await openComputerPrivacySettings();
      await refreshStatus();
    } catch (error) {
      setStatusError(errorMessage(error));
    } finally {
      setOpening(false);
    }
  }, [opening, refreshStatus]);

  if (!native) return null;

  return (
    <div className="space-y-6">
      <SettingCard
        icon={Monitor}
        title={t("settings:computer.enabled")}
        description={t("settings:computer.enabledHint")}
        divided
      >
        <SettingRow
          title={t("settings:computer.enabled")}
          description={t("settings:computer.enabledHint")}
        >
          <Switch
            id="native-computer-enabled"
            checked={native.computer_control_enabled}
            onCheckedChange={persistEnabled}
          />
        </SettingRow>
      </SettingCard>

      <SettingCard
        icon={ShieldCheck}
        title={t("settings:computer.permissionTitle")}
        description={t("settings:computer.permissionHint")}
        headerAction={
          status?.can_open_settings ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              disabled={opening}
              onClick={() => void handleOpenSettings()}
            >
              {opening ? <Loader2 className="mr-1.5 size-3 animate-spin" /> : null}
              {t("settings:computer.openSettings")}
            </Button>
          ) : null
        }
        divided
      >
        {statusLoading ? (
          <div className="flex items-center gap-2 px-5 py-4 text-xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            {t("settings:computer.loading")}
          </div>
        ) : (
          <>
            {statusError ? (
              <div
                role="alert"
                className="mx-5 mt-4 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
              >
                {t("settings:computer.statusFailed", { detail: statusError })}
              </div>
            ) : null}
            {status ? (
              <>
                <SettingRow
                  title={t("settings:computer.screenshot")}
                  description={status.screenshot.detail}
                >
                  <span className="text-[11px] text-muted-foreground">
                    {computerFlagLabel(status.screenshot)}
                  </span>
                </SettingRow>
                <SettingRow title={t("settings:computer.input")} description={status.input.detail}>
                  <span className="text-[11px] text-muted-foreground">
                    {computerFlagLabel(status.input)}
                  </span>
                </SettingRow>
              </>
            ) : null}
          </>
        )}
      </SettingCard>

      <SettingCard
        icon={AlertTriangle}
        title={t("settings:computer.riskTitle")}
        description={t("settings:computer.riskHint")}
      >
        <p className="px-5 py-3 text-xs leading-relaxed text-muted-foreground">
          {status?.hint ?? t("settings:computer.riskHint")}
        </p>
      </SettingCard>
    </div>
  );
}
