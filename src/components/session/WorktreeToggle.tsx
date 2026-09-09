import { GitFork, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function WorktreeToggle() {
  const { t } = useTranslation("sessions");
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const enabled = useUiStore((state) => state.composerIsolateWorktree);
  const setEnabled = useUiStore((state) => state.setComposerIsolateWorktree);

  if (!workspaceId) return null;

  return (
    <div
      className={cn(
        "group/wt inline-flex h-7 items-center rounded-lg border border-border/70 px-2 text-xs font-medium shadow-2xs transition-all duration-150",
        enabled
          ? "border-accent/60 bg-accent/60 text-accent-foreground"
          : "bg-background/80 text-foreground/90 hover:bg-muted/40",
      )}
    >
      <button
        type="button"
        className={cn(
          "flex cursor-pointer items-center gap-1.5 outline-none",
          enabled && "text-accent-foreground",
        )}
        title={t("isolateWorktreeHint")}
        aria-pressed={enabled}
        onClick={() => setEnabled(!enabled)}
      >
        <GitFork className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="max-w-32 truncate">{t("isolateWorktree")}</span>
      </button>
      {enabled ? (
        <button
          type="button"
          className="ml-1.5 -mr-0.5 cursor-pointer rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-muted hover:text-foreground"
          title={t("isolateWorktreeClear")}
          aria-label={t("isolateWorktreeClear")}
          onClick={(event) => {
            event.stopPropagation();
            setEnabled(false);
          }}
        >
          <X className="size-3" />
        </button>
      ) : null}
    </div>
  );
}
