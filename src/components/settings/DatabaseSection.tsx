import { confirm, message, open, save } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Copy,
  Database,
  Download,
  FolderOpen,
  HardDrive,
  Loader2,
  Upload,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { backupDatabase, healthCheck, openDatabaseFolder, restoreDatabase } from "@/lib/backend";
import type { AppHealthCheck } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard, SettingRow } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

function formatBackupTimestamp(date = new Date()) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${year}${month}${day}-${hours}${minutes}${seconds}`;
}

function buildBackupDefaultPath(health: AppHealthCheck | null) {
  const version = health?.database_current_version ?? health?.database_latest_version ?? 0;
  const fileName = `noxcode-backup-v${version}-${formatBackupTimestamp()}.sql`;
  const databasePath = health?.database_path;

  if (!databasePath) return fileName;

  const lastSeparatorIndex = Math.max(
    databasePath.lastIndexOf("/"),
    databasePath.lastIndexOf("\\"),
  );
  if (lastSeparatorIndex < 0) return fileName;

  const directory = databasePath.slice(0, lastSeparatorIndex);
  const separator = directory.includes("\\") ? "\\" : "/";
  return `${directory}${separator}${fileName}`;
}

export function DatabaseSection() {
  const { t } = useTranslation("settings");
  const [health, setHealth] = useState<AppHealthCheck | null>(null);
  const [actionLoading, setActionLoading] = useState<"backup" | "restore" | "open-folder" | null>(
    null,
  );
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const databaseFileFilters = useMemo(
    () => [{ name: t("database.dialogs.fileFilterName"), extensions: ["sql"] }],
    [t],
  );
  const includeItems = t("database.backupScope.includesItems", {
    returnObjects: true,
  }) as string[];
  const excludeItems = t("database.backupScope.excludesItems", {
    returnObjects: true,
  }) as string[];
  const openDatabaseFolderTitle = health?.database_path
    ? t("database.actions.openDirectoryAvailable")
    : t("database.actions.pathUnavailable");

  const refreshHealth = useCallback(async () => {
    const next = await healthCheck();
    setHealth(next);
    return next;
  }, []);

  useEffect(() => {
    void refreshHealth().catch((error: unknown) => {
      setActionError(error instanceof Error ? error.message : String(error));
    });
  }, [refreshHealth]);

  const copyPath = async () => {
    if (!health?.database_path) return;
    await navigator.clipboard.writeText(health.database_path);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  async function handleBackup() {
    setActionLoading("backup");
    setActionError(null);
    setActionMessage(null);

    try {
      const defaultPath = buildBackupDefaultPath(health);
      const selectedPath = await save({
        defaultPath,
        filters: databaseFileFilters,
        title: t("database.dialogs.exportTitle"),
      });

      if (!selectedPath) {
        return;
      }

      const result = await backupDatabase(selectedPath);
      await refreshHealth();
      setActionMessage(result.message);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : t("database.messages.exportFailed"));
    } finally {
      setActionLoading(null);
    }
  }

  async function handleRestore() {
    setActionLoading("restore");
    setActionError(null);
    setActionMessage(null);

    try {
      const confirmed = await confirm(t("database.dialogs.importConfirmMessage"), {
        title: t("database.dialogs.importTitle"),
        kind: "warning",
      });

      if (!confirmed) {
        return;
      }

      const selectedPath = await open({
        directory: false,
        multiple: false,
        filters: databaseFileFilters,
        title: t("database.dialogs.selectBackupTitle"),
      });

      if (!selectedPath || typeof selectedPath !== "string") {
        return;
      }

      const result = await restoreDatabase(selectedPath);
      setActionMessage(result.message);
      await refreshHealth();
      await Promise.all([
        useWorkspaceStore.getState().load(),
        useChannelStore.getState().load(),
        useSettingsStore.getState().load(),
      ]);

      await message(
        t("database.dialogs.importCompleteMessage", {
          message: result.message,
          backupPath: result.backup_path,
        }),
        {
          title: t("database.dialogs.importCompleteTitle"),
          kind: "info",
        },
      );
    } catch (error) {
      setActionError(error instanceof Error ? error.message : t("database.messages.importFailed"));
    } finally {
      setActionLoading(null);
    }
  }

  async function handleOpenFolder() {
    setActionLoading("open-folder");
    setActionError(null);
    setActionMessage(null);

    try {
      await openDatabaseFolder();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : t("database.messages.openFolderFailed"),
      );
    } finally {
      setActionLoading(null);
    }
  }

  return (
    <div className="space-y-6">
      {actionMessage ? (
        <SettingFeedbackCallout
          variant="success"
          message={actionMessage}
          onClose={() => setActionMessage(null)}
        />
      ) : null}
      {actionError ? (
        <SettingFeedbackCallout
          variant="error"
          message={actionError}
          onClose={() => setActionError(null)}
        />
      ) : null}

      {/* 数据库健康状态卡片 */}
      <SettingCard
        icon={Database}
        title={t("database.maintenance.title")}
        description={t("database.maintenance.description")}
        badge={health ? `迁移版本 v${health.database_current_version}` : undefined}
        headerAction={
          <div className="flex items-center gap-1.5">
            <span className="size-2 rounded-full bg-emerald-500 shadow-2xs shadow-emerald-500/50" />
            <span className="text-xs text-muted-foreground font-medium">运行正常</span>
          </div>
        }
        divided
      >
        <SettingRow
          title={t("database.maintenance.pathLabel")}
          description={
            <span className="font-mono text-[11px] break-all">
              {health?.database_path ?? t("database.maintenance.detecting")}
            </span>
          }
        >
          <div className="flex items-center gap-1.5">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!health?.database_path}
              onClick={() => void copyPath()}
              className="h-7 text-xs gap-1"
            >
              {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
              {copied ? "已复制" : "复制路径"}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={actionLoading !== null || !health?.database_path}
              onClick={() => void handleOpenFolder()}
              title={openDatabaseFolderTitle}
              className="h-7 text-xs gap-1"
            >
              {actionLoading === "open-folder" ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <FolderOpen className="size-3" />
              )}
              {t("database.actions.openDirectory")}
            </Button>
          </div>
        </SettingRow>

        <SettingRow title="迁移版本状态" description="数据库当前架构版本与最新目标版本保持一致。">
          <div className="flex items-center gap-2">
            <span className="rounded-md border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-xs text-foreground">
              当前: v{health?.database_current_version ?? "?"}
            </span>
            <span className="rounded-md border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-xs text-muted-foreground">
              目标: v{health?.database_latest_version ?? "?"}
            </span>
          </div>
        </SettingRow>
      </SettingCard>

      {/* 备份与恢复操作卡片 */}
      <SettingCard
        icon={HardDrive}
        title="备份与还原"
        description="将 SQLite 核心数据导出为 SQL 脚本，或从已有备份还原。"
      >
        <div className="space-y-4">
          <div className="flex flex-wrap gap-2.5">
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs gap-1.5"
              onClick={() => void handleBackup()}
              disabled={actionLoading !== null}
            >
              {actionLoading === "backup" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Download className="size-3.5" />
              )}
              {t("database.actions.exportSql")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs gap-1.5"
              onClick={() => void handleRestore()}
              disabled={actionLoading !== null}
            >
              {actionLoading === "restore" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Upload className="size-3.5" />
              )}
              {t("database.actions.importSql")}
            </Button>
          </div>

          {/* 范围说明 */}
          <div className="rounded-xl border border-border/70 bg-muted/20 p-4 text-xs space-y-3">
            <p className="font-semibold text-foreground tracking-tight">
              {t("database.backupScope.title")}
            </p>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <p className="font-medium text-emerald-600 dark:text-emerald-400">
                  ✓ {t("database.backupScope.included")}
                </p>
                <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground leading-relaxed">
                  {includeItems.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="space-y-1.5">
                <p className="font-medium text-amber-600 dark:text-amber-400">
                  ✗ {t("database.backupScope.excluded")}
                </p>
                <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground leading-relaxed">
                  {excludeItems.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
            </div>
            <p className="border-t border-border/40 pt-2 text-[11px] text-muted-foreground leading-relaxed">
              {t("database.backupScope.restoreWarning")}
            </p>
          </div>
        </div>
      </SettingCard>
    </div>
  );
}
