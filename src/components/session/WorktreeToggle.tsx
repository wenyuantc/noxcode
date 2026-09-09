import { FolderGit2, GitFork } from "lucide-react";
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
      className="inline-flex h-8 items-stretch overflow-hidden rounded-lg border-2 border-border bg-background/90 text-xs font-medium shadow-2xs"
      title={t("isolateWorktreeHint")}
      role="group"
      aria-label={t("isolateWorktreeHint")}
    >
      <button
        type="button"
        className={cn(
          "inline-flex cursor-pointer items-center gap-1.5 px-2.5 outline-none transition-colors",
          !enabled
            ? "bg-accent font-semibold text-accent-foreground"
            : "text-foreground/80 hover:bg-muted/60",
        )}
        aria-pressed={!enabled}
        onClick={() => setEnabled(false)}
      >
        <FolderGit2 className="size-3.5 shrink-0" />
        <span className="whitespace-nowrap">{t("worktreeModeCurrent")}</span>
      </button>
      <button
        type="button"
        className={cn(
          "inline-flex cursor-pointer items-center gap-1.5 border-l-2 border-border px-2.5 outline-none transition-colors",
          enabled
            ? "bg-accent font-semibold text-accent-foreground"
            : "text-foreground/80 hover:bg-muted/60",
        )}
        aria-pressed={enabled}
        onClick={() => setEnabled(true)}
      >
        <GitFork className="size-3.5 shrink-0" />
        <span className="whitespace-nowrap">{t("worktreeModeIsolate")}</span>
      </button>
    </div>
  );
}
