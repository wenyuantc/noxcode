import { GitBranch, PanelLeft } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { displaySessionTitle } from "@/lib/sessionLines";
import { GLOBAL_SHORTCUTS, shortcutDisplay } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { WorkspacePicker } from "./WorkspacePicker";

export function SessionHeader() {
  const { t } = useTranslation("nav");
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

  return (
    <div className="flex h-11 items-center justify-between gap-2 border-b border-border/60 bg-background/60 px-4 py-1.5 backdrop-blur-xs">
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
      </div>
      <span className="min-w-0 flex-1 truncate px-3 text-center text-xs font-medium tracking-tight text-muted-foreground/75">
        {title}
      </span>
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
