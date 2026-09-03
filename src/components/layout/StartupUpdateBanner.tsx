import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  checkForAppUpdateOnStartup,
  dismissStartupUpdate,
  type AppUpdateInfo,
} from "@/lib/appUpdate";

export function StartupUpdateBanner() {
  const { t } = useTranslation("settings");
  const navigate = useNavigate();
  const [update, setUpdate] = useState<AppUpdateInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    void checkForAppUpdateOnStartup().then((info) => {
      if (!cancelled && info) {
        setUpdate(info);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!update) {
    return null;
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-b border-sky-500/30 bg-sky-500/10 px-4 py-2 text-sm">
      <p>
        {t("about.startupAvailable", {
          version: update.version,
          current: update.currentVersion,
        })}
      </p>
      <div className="flex items-center gap-2">
        <Button type="button" size="sm" onClick={() => navigate("/settings/about")}>
          {t("about.startupOpenSettings")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-label={t("about.startupDismiss")}
          onClick={() => {
            dismissStartupUpdate(update.version);
            setUpdate(null);
          }}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
