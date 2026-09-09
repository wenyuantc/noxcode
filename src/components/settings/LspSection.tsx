import { Check, Code2, Download, Loader2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { installLspServer, listLspServers, updateNativeSettings } from "@/lib/backend";
import type { LspServerStatus } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";
import { SettingCard, SettingRow } from "./SettingCard";

export function LspSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [lspServers, setLspServers] = useState<LspServerStatus[]>([]);
  const [lspLoading, setLspLoading] = useState(true);
  const [lspInstalling, setLspInstalling] = useState<Set<string>>(() => new Set());
  const [lspErrors, setLspErrors] = useState<Record<string, string>>({});
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);
  const lspInstallingRef = useRef(new Set<string>());

  const refreshLspServers = useCallback(async (silent = false) => {
    if (!silent) setLspLoading(true);
    try {
      setLspServers(await listLspServers());
    } catch (error) {
      setFeedback({
        variant: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      if (!silent) setLspLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshLspServers();
  }, [refreshLspServers]);

  const handleInstallLsp = useCallback(
    async (language: string) => {
      if (lspInstallingRef.current.has(language)) return;
      lspInstallingRef.current.add(language);
      setLspInstalling(new Set(lspInstallingRef.current));
      setLspErrors((prev) => {
        if (!(language in prev)) return prev;
        const next = { ...prev };
        delete next[language];
        return next;
      });
      try {
        await installLspServer(language);
        await refreshLspServers(true);
      } catch (error) {
        setLspErrors((prev) => ({
          ...prev,
          [language]: error instanceof Error ? error.message : String(error),
        }));
      } finally {
        lspInstallingRef.current.delete(language);
        setLspInstalling(new Set(lspInstallingRef.current));
      }
    },
    [refreshLspServers],
  );

  const pendingLspCount = lspServers.filter(
    (server) => !server.installed_command && server.installable && !lspInstalling.has(server.id),
  ).length;

  const handleInstallMissing = useCallback(() => {
    for (const server of lspServers) {
      if (!server.installed_command && server.installable) {
        void handleInstallLsp(server.id);
      }
    }
  }, [handleInstallLsp, lspServers]);

  const persistEnabled = useCallback(
    (checked: boolean) => {
      void updateNativeSettings({ lsp_enabled: checked })
        .then((updated) => {
          setNative(updated);
          setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
        })
        .catch((error: unknown) => {
          setFeedback({
            variant: "error",
            message: error instanceof Error ? error.message : String(error),
          });
        });
    },
    [setNative, t],
  );

  if (!native) return null;

  return (
    <div className="space-y-6">
      {feedback ? (
        <SettingFeedbackCallout
          variant={feedback.variant}
          message={feedback.message}
          onClose={() => setFeedback(null)}
        />
      ) : null}

      <SettingCard
        icon={Code2}
        title={t("settings:lsp.enabled")}
        description={t("settings:lsp.enabledHint")}
        divided
      >
        <SettingRow title={t("settings:lsp.enabled")} description={t("settings:lsp.enabledHint")}>
          <Switch
            id="native-lsp-enabled"
            checked={native.lsp_enabled}
            onCheckedChange={persistEnabled}
          />
        </SettingRow>
      </SettingCard>

      <SettingCard
        icon={Download}
        title={t("settings:lsp.installTitle")}
        description={t("settings:lsp.installHint")}
        headerAction={
          !lspLoading && pendingLspCount > 0 ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 text-xs"
              onClick={handleInstallMissing}
            >
              {lspInstalling.size > 0 ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Download className="size-3" />
              )}
              {t("settings:lsp.installMissing", { count: pendingLspCount })}
            </Button>
          ) : null
        }
        divided
      >
        {lspLoading ? (
          <div className="flex items-center gap-2 px-5 py-4 text-xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            {t("settings:lsp.loading")}
          </div>
        ) : (
          lspServers.map((server) => {
            const installing = lspInstalling.has(server.id);
            const error = lspErrors[server.id];
            return (
              <SettingRow
                key={server.id}
                title={
                  <span className="flex items-center gap-2">
                    {server.label}
                    {server.installed_command ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-600 dark:text-emerald-400">
                        <Check className="size-3" />
                        {t("settings:lsp.installed")}
                      </span>
                    ) : null}
                  </span>
                }
                description={
                  error ? (
                    <span className="text-destructive">{error}</span>
                  ) : server.installed_command ? (
                    `${t("settings:lsp.command")}: ${server.installed_command}`
                  ) : (
                    `${t("settings:lsp.installCommand")}: ${server.install_command ?? server.commands.join(" / ")}`
                  )
                }
              >
                {server.installed_command ? (
                  <span className="text-[11px] text-muted-foreground">
                    {t("settings:lsp.ready")}
                  </span>
                ) : server.installable ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs"
                    disabled={installing}
                    onClick={() => void handleInstallLsp(server.id)}
                  >
                    {installing ? (
                      <Loader2 className="mr-1.5 size-3 animate-spin" />
                    ) : (
                      <Download className="mr-1.5 size-3" />
                    )}
                    {t("settings:lsp.install")}
                  </Button>
                ) : (
                  <span className="text-[11px] text-muted-foreground">
                    {t("settings:lsp.manual")}
                  </span>
                )}
              </SettingRow>
            );
          })
        )}
      </SettingCard>
    </div>
  );
}
