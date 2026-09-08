import { Loader2, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { cn } from "@/lib/utils";
import {
  type AppUpdateProgress,
  type AppUpdateStatus,
  sidebarUpdateLabelKey,
  useUpdateStore,
} from "@/stores/updateStore";

export function SidebarUpdateButton({
  status,
  version,
  progress,
  onDownload,
  onRelaunch,
}: {
  status: AppUpdateStatus;
  version?: string;
  progress?: AppUpdateProgress | null;
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

  const hasPercent = status === "downloading" && progress?.percent != null;
  const busy = status === "downloading" || status === "restarting";
  const label = hasPercent ? `${progress.percent}%` : t(labelKey);
  const title =
    status === "downloading"
      ? hasPercent
        ? `${t("downloading")} (${progress.percent}%)`
        : t("downloading")
      : version && status === "available"
        ? t("updateAvailableTitle", { version })
        : t(labelKey);

  return (
    <button
      type="button"
      disabled={busy}
      onClick={handleClick}
      title={title}
      aria-label={title}
      className={cn(
        "inline-flex shrink-0 items-center justify-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium text-white shadow-xs select-none transition-all duration-150",
        busy
          ? "cursor-default bg-blue-600/90 dark:bg-blue-500/90"
          : "cursor-pointer bg-blue-600 hover:bg-blue-500 active:bg-blue-700 dark:bg-blue-500 dark:hover:bg-blue-400 dark:active:bg-blue-600",
      )}
    >
      {busy && !hasPercent ? <Loader2 className="size-3 animate-spin" /> : null}
      {label}
    </button>
  );
}

export function SidebarFooter() {
  const { t } = useTranslation("nav");
  const navigate = useNavigate();
  const status = useUpdateStore((state) => state.status);
  const update = useUpdateStore((state) => state.update);
  const progress = useUpdateStore((state) => state.progress);
  const startDownload = useUpdateStore((state) => state.startDownload);
  const relaunch = useUpdateStore((state) => state.relaunch);

  return (
    <div className="flex items-center justify-between border-t border-sidebar-border/70 px-2.5 py-2">
      <button
        type="button"
        className="group flex min-w-0 items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-sidebar-foreground/90 transition-all duration-150 hover:bg-sidebar-accent/80 hover:text-sidebar-foreground active:scale-[0.99]"
        onClick={() => void navigate("/settings")}
        title={t("settings")}
        aria-label={t("settings")}
      >
        <Settings className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-sidebar-foreground" />
        <span className="truncate tracking-tight">noxcode</span>
      </button>
      <SidebarUpdateButton
        status={status}
        version={update?.version}
        progress={progress}
        onDownload={() => void startDownload()}
        onRelaunch={() => void relaunch()}
      />
    </div>
  );
}
