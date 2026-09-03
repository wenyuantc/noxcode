import { open, save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { backupDatabase, healthCheck, openDatabaseFolder, restoreDatabase } from "@/lib/backend";
import type { AppHealthCheck } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { SettingCard } from "./SettingCard";

export function DatabaseSection() {
  const { t } = useTranslation(["settings", "common"]);
  const [health, setHealth] = useState<AppHealthCheck | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  return (
    <SettingCard title={t("settings:database.title")} description={t("settings:database.hint")}>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() =>
            void healthCheck()
              .then(setHealth)
              .catch((err: unknown) => setMessage(String(err)))
          }
        >
          {t("settings:database.health")}
        </Button>
        <Button
          onClick={() => {
            void save({ defaultPath: "noxcode.db.sql" }).then((path) => {
              if (typeof path === "string") {
                return backupDatabase(path).then((result) => setMessage(result.message));
              }
            });
          }}
        >
          {t("settings:database.backup")}
        </Button>
        <Button
          variant="outline"
          onClick={() => {
            void open({ multiple: false }).then((path) => {
              if (typeof path === "string") {
                return restoreDatabase(path).then((result) => setMessage(result.message));
              }
            });
          }}
        >
          {t("settings:database.restore")}
        </Button>
        <Button variant="ghost" onClick={() => void openDatabaseFolder()}>
          {t("settings:database.openFolder")}
        </Button>
      </div>
      {health ? (
        <p className="mt-3 text-xs text-muted-foreground">
          git {health.git_version ?? "—"} · db v{health.database_current_version ?? "—"}
        </p>
      ) : null}
      {message ? <p className="mt-2 text-sm">{message}</p> : null}
    </SettingCard>
  );
}
