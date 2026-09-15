import { Check, Code2, Download, Loader2, Zap } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  installLspServer,
  listLspServers,
  testLspServer,
  updateNativeSettings,
} from "@/lib/backend";
import { errorMessage, runToastAction, showToast, type ToastVariant } from "@/lib/toast";
import type { LspServerStatus, LspTestResult } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard, SettingRow } from "./SettingCard";

/** 测试结果摘要：优先展示 language server 自报的名称与版本。 */
export function lspTestSummary(result: LspTestResult): string {
  const identity = result.server_name
    ? result.server_version
      ? `${result.server_name} ${result.server_version}`
      : result.server_name
    : result.command;
  return `${identity} · ${result.elapsed_ms} ms`;
}

/** 已安装的服务器提供「测试」，其余按是否可自动安装区分为「安装」或「手动安装」。 */
export function lspRowAction(server: LspServerStatus): "test" | "install" | "manual" {
  if (server.installed_command) return "test";
  return server.installable ? "install" : "manual";
}

/** 测试成功但 language server 自报异常时用 warning 变体（保留更久），否则 success。 */
export function lspTestVariant(result: LspTestResult): ToastVariant {
  return result.warning ? "warning" : "success";
}

export function LspSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [lspServers, setLspServers] = useState<LspServerStatus[]>([]);
  const [lspLoading, setLspLoading] = useState(true);
  const [lspInstalling, setLspInstalling] = useState<Set<string>>(() => new Set());
  const [lspTesting, setLspTesting] = useState<Set<string>>(() => new Set());
  const [lspErrors, setLspErrors] = useState<Record<string, string>>({});
  const [lspListError, setLspListError] = useState<string | null>(null);
  const lspInstallingRef = useRef(new Set<string>());
  const lspTestingRef = useRef(new Set<string>());

  const refreshLspServers = useCallback(async (silent = false) => {
    if (!silent) setLspLoading(true);
    try {
      setLspServers(await listLspServers());
      setLspListError(null);
    } catch (error) {
      // 列表加载失败是持久问题：内联展示，直到重试成功，不用会自动消失的 toast。
      setLspListError(errorMessage(error));
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
          [language]: errorMessage(error),
        }));
      } finally {
        lspInstallingRef.current.delete(language);
        setLspInstalling(new Set(lspInstallingRef.current));
      }
    },
    [refreshLspServers],
  );

  const handleTestLsp = useCallback(
    async (server: LspServerStatus) => {
      if (lspTestingRef.current.has(server.id)) return;
      lspTestingRef.current.add(server.id);
      setLspTesting(new Set(lspTestingRef.current));
      setLspErrors((prev) => {
        if (!(server.id in prev)) return prev;
        const next = { ...prev };
        delete next[server.id];
        return next;
      });
      try {
        const result = await testLspServer(server.id);
        // 测试结果走 toast：正常为 success（约 3s），language server 自报异常时用
        // warning 变体保留更久，并完整展示 warning 内容。
        showToast({
          id: `lsp-test-${server.id}`,
          variant: lspTestVariant(result),
          description: (
            <>
              <span>
                {t("settings:lsp.testSuccess", {
                  label: result.label,
                  detail: lspTestSummary(result),
                })}
              </span>
              {result.warning ? (
                <span className="mt-0.5 block opacity-80">
                  {t("settings:lsp.testWarning", { detail: result.warning })}
                </span>
              ) : null}
            </>
          ),
        });
      } catch (error) {
        setLspErrors((prev) => ({
          ...prev,
          [server.id]: errorMessage(error),
        }));
      } finally {
        lspTestingRef.current.delete(server.id);
        setLspTesting(new Set(lspTestingRef.current));
      }
    },
    [t],
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
      void runToastAction(
        () => updateNativeSettings({ lsp_enabled: checked }).then((updated) => setNative(updated)),
        {
          id: "lsp-enabled-save",
          successMessage: t("common:saved"),
        },
      );
    },
    [setNative, t],
  );

  if (!native) return null;

  return (
    <div className="space-y-6">
      {lspListError ? (
        <div
          role="alert"
          className="flex flex-wrap items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3.5 py-2.5 text-xs text-destructive"
        >
          <span className="min-w-0 flex-1 break-words">
            {t("settings:lsp.loadFailed", { detail: lspListError })}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 shrink-0 text-xs"
            onClick={() => void refreshLspServers()}
          >
            {t("common:retry")}
          </Button>
        </div>
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
            const testing = lspTesting.has(server.id);
            const error = lspErrors[server.id];
            const action = lspRowAction(server);
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
                {action === "test" ? (
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-muted-foreground">
                      {t("settings:lsp.ready")}
                    </span>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-7 gap-1.5 text-xs"
                      disabled={testing}
                      title={t("settings:lsp.testHint")}
                      onClick={() => void handleTestLsp(server)}
                    >
                      {testing ? (
                        <Loader2 className="size-3 animate-spin" />
                      ) : (
                        <Zap className="size-3" />
                      )}
                      {testing ? t("settings:lsp.testing") : t("settings:lsp.test")}
                    </Button>
                  </div>
                ) : action === "install" ? (
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
