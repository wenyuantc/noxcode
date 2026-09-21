import {
  AlertTriangle,
  Layers,
  Loader2,
  Monitor,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  deleteNativePermissionRule,
  getComputerPermissionStatus,
  getNativePermissionRules,
  openComputerPrivacySettings,
  updateNativeSettings,
} from "@/lib/backend";
import { errorMessage, runToastAction } from "@/lib/toast";
import type { ComputerPermissionFlag, ComputerPermissionStatus, PermissionRule } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard, SettingRow } from "./SettingCard";

export function computerFlagLabel(flag: ComputerPermissionFlag): string {
  return flag.label;
}

export async function requestComputerPermissionStatus(
  loader: () => Promise<ComputerPermissionStatus> = getComputerPermissionStatus,
): Promise<ComputerPermissionStatus> {
  return loader();
}

export function computerStatusShouldRefreshOnFocus(options: { visibilityState?: string }): boolean {
  return options.visibilityState !== "hidden";
}

export function computerProcessIdentityValues(status: ComputerPermissionStatus): {
  bundle: string | null;
  path: string | null;
  fallback: string;
} {
  return {
    bundle: status.bundle_id,
    path: status.executable_path,
    fallback: status.process_identity,
  };
}

function computerAllowRules(
  rules: { allow: PermissionRule[] } | null | undefined,
): PermissionRule[] {
  return (rules?.allow ?? []).filter((rule) => rule.capability === "computer");
}

export function ComputerSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [status, setStatus] = useState<ComputerPermissionStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [opening, setOpening] = useState(false);
  const [approved, setApproved] = useState<PermissionRule[]>([]);
  const [approvedError, setApprovedError] = useState<string | null>(null);

  const refreshStatus = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setStatusLoading(true);
    }
    try {
      setStatus(await requestComputerPermissionStatus());
      setStatusError(null);
    } catch (error) {
      setStatusError(errorMessage(error));
    } finally {
      if (!options?.silent) {
        setStatusLoading(false);
      }
    }
  }, []);

  const refreshApproved = useCallback(async () => {
    try {
      const view = await getNativePermissionRules(workspaceId);
      setApproved([...computerAllowRules(view.global), ...computerAllowRules(view.workspace)]);
      setApprovedError(null);
    } catch (error) {
      setApprovedError(errorMessage(error));
    }
  }, [workspaceId]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  useEffect(() => {
    const onFocus = () => {
      const visibilityState =
        typeof document === "undefined" ? "visible" : document.visibilityState;
      if (computerStatusShouldRefreshOnFocus({ visibilityState })) {
        void refreshStatus({ silent: true });
      }
    };
    const onVisibility = () => {
      if (typeof document !== "undefined" && document.visibilityState === "visible") {
        void refreshStatus({ silent: true });
      }
    };
    if (typeof window !== "undefined") {
      window.addEventListener("focus", onFocus);
    }
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisibility);
    }
    return () => {
      if (typeof window !== "undefined") {
        window.removeEventListener("focus", onFocus);
      }
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", onVisibility);
      }
    };
  }, [refreshStatus]);

  useEffect(() => {
    void refreshApproved();
  }, [refreshApproved]);

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

  const handleRevoke = useCallback(
    (rule: PermissionRule) => {
      void runToastAction(
        () =>
          deleteNativePermissionRule(rule.id, rule.scope === "workspace" ? workspaceId : null).then(
            () => refreshApproved(),
          ),
        {
          id: `computer-revoke-${rule.id}`,
          successMessage: t("common:saved"),
        },
      );
    },
    [refreshApproved, t, workspaceId],
  );

  const platformNotes = useMemo(
    () => [
      t("settings:computer.platformMac"),
      t("settings:computer.platformWindows"),
      t("settings:computer.platformLinux"),
    ],
    [t],
  );

  const processIdentityNote = useMemo(() => {
    if (!status) return null;
    const identity = computerProcessIdentityValues(status);
    if (identity.bundle || identity.path) {
      return t("settings:computer.processIdentity", {
        bundle: identity.bundle || t("settings:computer.unknownBundle"),
        path: identity.path || t("settings:computer.unknownPath"),
      });
    }
    return identity.fallback;
  }, [status, t]);

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
        <p className="px-5 py-3 text-xs leading-relaxed text-muted-foreground">
          {t("settings:computer.backgroundHint")}
        </p>
        <p className="px-5 pb-3 text-xs leading-relaxed text-muted-foreground">
          {t("settings:computer.foregroundHint")}
        </p>
      </SettingCard>

      <SettingCard
        icon={ShieldCheck}
        title={t("settings:computer.permissionTitle")}
        description={t("settings:computer.permissionHint")}
        headerAction={
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              disabled={statusLoading}
              onClick={() => void refreshStatus()}
            >
              {statusLoading ? (
                <Loader2 className="mr-1.5 size-3 animate-spin" />
              ) : (
                <RefreshCw className="mr-1.5 size-3" />
              )}
              {t("settings:computer.recheck")}
            </Button>
            {status?.can_open_settings ? (
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
            ) : null}
          </div>
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
                {processIdentityNote ? (
                  <p className="px-5 py-3 text-xs leading-relaxed text-muted-foreground">
                    {processIdentityNote}
                  </p>
                ) : null}
              </>
            ) : null}
          </>
        )}
        <p className="px-5 py-3 text-xs leading-relaxed text-muted-foreground">
          {t("settings:computer.recheckHint")}
        </p>
      </SettingCard>

      <SettingCard
        icon={Layers}
        title={t("settings:computer.approvedTitle")}
        description={t("settings:computer.approvedHint")}
        divided
      >
        {approvedError ? (
          <div
            role="alert"
            className="mx-5 mt-4 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
          >
            {approvedError}
          </div>
        ) : null}
        {approved.length === 0 ? (
          <p className="px-5 py-3 text-xs leading-relaxed text-muted-foreground">
            {t("settings:computer.approvedEmpty")}
          </p>
        ) : (
          approved.map((rule) => (
            <SettingRow key={rule.id} title={rule.pattern} description={rule.note || rule.source}>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={() => handleRevoke(rule)}
              >
                <Trash2 className="mr-1.5 size-3" />
                {t("settings:computer.revoke")}
              </Button>
            </SettingRow>
          ))
        )}
      </SettingCard>

      <SettingCard
        icon={AlertTriangle}
        title={t("settings:computer.platformTitle")}
        description={t("settings:computer.platformHint")}
      >
        <ul className="space-y-2 px-5 py-3 text-xs leading-relaxed text-muted-foreground">
          {platformNotes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
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
