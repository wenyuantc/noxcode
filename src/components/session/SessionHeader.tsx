import { GitBranch } from "lucide-react";

import { displaySessionTitle } from "@/lib/sessionLines";
import { Button } from "@/components/ui/button";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { BranchPicker } from "./BranchPicker";
import { WorkspacePicker } from "./WorkspacePicker";

export function SessionHeader() {
  const selected = useSessionStore((state) => state.selectedSessionId);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const session = sessions.find((item) => item.id === selected);
  const title = displaySessionTitle(session?.title);
  const toggleGit = useUiStore((state) => state.toggleGit);

  return (
    <div className="flex items-center gap-2 border-b px-4 py-2">
      <WorkspacePicker />
      <BranchPicker />
      <span className="min-w-0 flex-1 truncate px-2 text-center text-sm text-muted-foreground">
        {title}
      </span>
      <Button size="sm" variant="ghost" onClick={toggleGit}>
        <GitBranch className="size-4" />
        Git
      </Button>
    </div>
  );
}
