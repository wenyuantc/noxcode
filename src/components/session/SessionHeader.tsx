import { GitBranch, GitMerge, PanelLeft } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { getWorktreeMergeState } from "@/lib/backend";
import { displaySessionTitle } from "@/lib/sessionLines";
import { GLOBAL_SHORTCUTS, shortcutDisplay } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { isManagedWorktreePath } from "@/lib/worktreePath";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { WorkspacePicker } from "./WorkspacePicker";
import { SessionMenu } from "./SessionMenu";

export function SessionHeader() {
  const { t } = useTranslation(["nav", "git"]);
  const selected = useSessionStore((state) => state.selectedSessionId);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const session = sessions.find((item) => item.id === selected);
  const title = displaySessionTitle(session?.title);
  const toggleGit = useUiStore((state) => state.toggleGit);
  const gitOpen = useUiStore((state) => state.gitOpen);
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);
  const sidebarCollapsed = useUiStore((state) => state.sidebarCollapsed);

  const sidebarShortcut = GLOBAL_SHORTCUTS.find((s) => s.id === "toggle-sidebar");
  const shortcutHint = sidebarShortcut ? ` (${shortcutDisplay(sidebarShortcut)})` : "";
  const runtime = useSessionStore((state) =>
    selected ? state.configurationBySession[selected] : undefined,
  );
  const openMerge = useSessionStore((state) => state.openWorktreeMergePrompt);
  const mergePrompt = useSessionStore((state) => state.worktreeMergePrompt);
  const [mergeConflicts, setMergeConflicts] = useState<string[]>([]);
  const workspaceId = session?.workspace_id ?? null;
  const worktreeRoot = useSettingsStore((state) => state.native?.worktree_root);
  const isolated = Boolean(
    session &&
    isManagedWorktreePath(runtime?.worktree_path ?? session.working_dir, session.id, worktreeRoot),
  );

  useEffect(() => {
    if (!workspaceId) {
      setMergeConflicts([]);
      return;
    }
    let cancelled = false;
    void getWorktreeMergeState(workspaceId)
      .then((state) => {
        if (!cancelled) setMergeConflicts(state.in_progress ? state.conflicts : []);
      })
      .catch(() => {
        if (!cancelled) setMergeConflicts([]);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId, selected, isolated, mergePrompt]);

  const conflicted = mergeConflicts.length > 0;
  const showMerge = Boolean(session && workspaceId && (isolated || conflicted));

  return (
    <div className="relative z-40 flex h-11 items-center justify-between gap-2 border-b border-border/60 bg-background/80 px-4 py-1.5 backdrop-blur-xs">
      <div className="flex items-center gap-1.5">
        <Button
          size="icon-sm"
          variant="ghost"
          className={cn(
            "h-7 w-7 rounded-lg text-muted-foreground transition-all hover:text-foreground",
            !sidebarCollapsed && "bg-accent/60 font-medium text-accent-foreground shadow-2xs",
          )}
          title={`${t("shortcuts.toggleSidebar")}${shortcutHint}`}
          onClick={toggleSidebar}
        >
          <PanelLeft className="size-4" />
        </Button>
        <WorkspacePicker />
        <BranchPicker />
        {session ? <SessionMenu key={session.id} session={session} /> : null}
      </div>
      <span className="min-w-0 flex-1 truncate px-3 text-center text-xs font-medium tracking-tight text-muted-foreground/75">
        {title}
      </span>
      {showMerge && session && workspaceId ? (
        <Button
          size="sm"
          className="h-7 gap-1.5 rounded-lg px-2.5 text-xs"
          onClick={() =>
            openMerge({
              sessionId: session.id,
              workspaceId,
              phase: conflicted ? "conflict" : "choose",
              conflicts: mergeConflicts,
            })
          }
        >
          <GitMerge className="size-3.5" />
          <span>{conflicted ? t("git:mergeWorktreeContinue") : t("git:mergeWorktree")}</span>
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        className={cn(
          "h-7 gap-1.5 rounded-lg px-2.5 text-xs text-muted-foreground transition-all hover:text-foreground",
          gitOpen && "bg-accent font-medium text-accent-foreground shadow-2xs",
        )}
        onClick={toggleGit}
      >
        <GitBranch className="size-3.5" />
        <span>Git</span>
      </Button>
    </div>
  );
}
