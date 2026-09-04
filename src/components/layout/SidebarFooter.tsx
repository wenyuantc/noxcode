import { Loader2, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { cn } from "@/lib/utils";
import { type AppUpdateStatus, sidebarUpdateLabelKey, useUpdateStore } from "@/stores/updateStore";

export function SidebarUpdateButton({
  status,
  version,
  onDownload,
  onRelaunch,
}: {
  status: AppUpdateStatus;
  version?: string;
  onDownload: () => void;
  onRelaunch: () => void;
}) {
  const { t } = useTranslation("nav");
  const labelKey = sidebarUpdateLabelKey(status);
  if (!labelKey) {
    return null;
  }

  const handleClick = () => {
    if (status === "available") {
      onDownload();
      return;
    }
    if (status === "ready") {
      onRelaunch();
    }
  };

  return (
    <button
      type="button"
      disabled={status === "downloading"}
      onClick={handleClick}
      title={
        version && status === "available" ? t("updateAvailableTitle", { version }) : t(labelKey)
      }
      aria-label={t(labelKey)}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium transition-all duration-150",
        status === "downloading"
          ? "cursor-wait text-muted-foreground"
          : "text-primary hover:bg-sidebar-accent/80 hover:text-primary",
      )}
    >
      {status === "downloading" ? <Loader2 className="size-3 animate-spin" /> : null}
      {t(labelKey)}
    </button>
  );
}

export function SidebarFooter() {
  const { t } = useTranslation("nav");
  const navigate = useNavigate();
  const status = useUpdateStore((state) => state.status);
  const update = useUpdateStore((state) => state.update);
  const startDownload = useUpdateStore((state) => state.startDownload);
  const relaunch = useUpdateStore((state) => state.relaunch);

  return (
    <div className="flex items-center justify-between border-t border-sidebar-border/70 px-3 py-2">
      <span className="text-[11px] font-medium tracking-tight text-muted-foreground/60 select-none">
        noxcode
      </span>
      <SidebarUpdateButton
        status={status}
        version={update?.version}
        onDownload={() => void startDownload()}
        onRelaunch={() => void relaunch()}
      />
      <button
        type="button"
        className="flex items-center gap-1.5 rounded-lg p-1.5 text-muted-foreground transition-all duration-150 hover:bg-sidebar-accent/80 hover:text-sidebar-foreground"
        onClick={() => void navigate("/settings")}
        title={t("settings")}
        aria-label={t("settings")}
      >
        <Settings className="size-4" />
      </button>
    </div>
  );
}
