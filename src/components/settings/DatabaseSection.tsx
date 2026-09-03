import { confirm, message, open, save } from "@tauri-apps/plugin-dialog";
import { Download, FolderOpen, Loader2, Upload } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { backupDatabase, healthCheck, openDatabaseFolder, restoreDatabase } from "@/lib/backend";
import type { AppHealthCheck } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

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

  async function handleBackup() {
    setActionLoading("backup");
    setActionError(null);
    setActionMessage(null);

    try {
      const destination = await save({
        title: t("database.dialogs.exportTitle"),
        defaultPath: buildBackupDefaultPath(health),
        filters: databaseFileFilters,
      });

      if (!destination) {
        return;
      }

      const result = await backupDatabase(destination);
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

      const selected = await open({
        title: t("database.dialogs.selectBackupTitle"),
        directory: false,
        multiple: false,
        filters: databaseFileFilters,
      });

      if (typeof selected !== "string") {
        return;
      }

      const result = await restoreDatabase(selected);
      setActionMessage(result.message);
      await refreshHealth();
      await Promise.all([
        useChannelStore.getState().load(),
        useSettingsStore.getState().load(),
        useWorkspaceStore.getState().load(),
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
    <div className="space-y-4 rounded-lg border border-border bg-card p-4">
      <div>
        <h3 className="text-sm font-medium">{t("database.maintenance.title")}</h3>
        <p className="text-xs text-muted-foreground">{t("database.maintenance.description")}</p>
      </div>

      <div className="grid gap-2 rounded-md border border-border px-3 py-3 text-xs text-muted-foreground">
        <p className="break-all">
          {t("database.maintenance.pathLabel")}:
          {health?.database_path ?? t("database.maintenance.detecting")}
        </p>
        <p>
          {t("database.maintenance.currentVersionLabel")}:
          {health?.database_current_version ?? t("database.maintenance.unknown")}
        </p>
        <p>
          {t("database.maintenance.latestVersionLabel")}:
          {health?.database_latest_version ?? t("database.maintenance.unknown")}
        </p>
        {health?.database_current_description ? <p>{health.database_current_description}</p> : null}
      </div>

      <div className="space-y-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-xs text-amber-950 dark:text-amber-100">
        <p className="font-medium">{t("database.backupScope.title")}</p>
        <div className="grid gap-2 sm:grid-cols-2">
          <div>
            <p className="mb-1 font-medium text-green-800 dark:text-green-200">
              {t("database.backupScope.included")}
            </p>
            <ul className="list-disc space-y-1 pl-4">
              {includeItems.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
          <div>
            <p className="mb-1 font-medium text-amber-900 dark:text-amber-50">
              {t("database.backupScope.excluded")}
            </p>
            <ul className="list-disc space-y-1 pl-4">
              {excludeItems.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        </div>
        <p className="text-[11px] leading-5 opacity-90">
          {t("database.backupScope.restoreWarning")}
        </p>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() => void handleBackup()}
          disabled={actionLoading !== null}
        >
          {actionLoading === "backup" ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Download className="h-4 w-4" />
          )}
          {t("database.actions.exportSql")}
        </Button>
        <Button
          variant="outline"
          onClick={() => void handleRestore()}
          disabled={actionLoading !== null}
        >
          {actionLoading === "restore" ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Upload className="h-4 w-4" />
          )}
          {t("database.actions.importSql")}
        </Button>
        <Button
          variant="ghost"
          onClick={() => void handleOpenFolder()}
          disabled={actionLoading !== null || !health?.database_path}
          title={openDatabaseFolderTitle}
        >
          {actionLoading === "open-folder" ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <FolderOpen className="h-4 w-4" />
          )}
          {t("database.actions.openDirectory")}
        </Button>
      </div>

      {actionMessage ? <p className="text-xs text-green-700">{actionMessage}</p> : null}
      {actionError ? <p className="text-xs text-destructive">{actionError}</p> : null}
    </div>
  );
}
