import { confirm, message } from "@tauri-apps/plugin-dialog";
import {
  ChevronDown,
  Folder,
  MoreHorizontal,
  Pencil,
  Pin,
  Plus,
  Sparkle,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { deleteAgentSession, setAgentSessionPinned } from "@/lib/backend";
import { displaySessionTitle } from "@/lib/sessionLines";
import { formatRelativeTime } from "@/lib/utils";
import { getCurrentAppLocale, getDateLocale } from "@/lib/i18n/locale";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { cn } from "@/lib/utils";
import type { AgentSession, Workspace } from "@/lib/types";

function SessionRow({
  session,
  selected,
  locale,
  indent,
}: {
  session: AgentSession;
  selected: boolean;
  locale: string;
  indent?: boolean;
}) {
  const { t } = useTranslation("layout");
  const loadHistory = useSessionStore((state) => state.loadHistory);
  const working = useSessionStore((state) => state.turnState[session.id] === "working");
  const pinned = session.pinned !== 0;

  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        "group flex cursor-pointer items-center rounded-md p-[10px] select-none hover:bg-sidebar-accent",
        indent && "ml-[10px]",
        selected && "bg-sidebar-accent",
      )}
      onClick={() => void loadHistory(session.id)}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        void loadHistory(session.id);
      }}
    >
      <button
        type="button"
        className={cn(
          "cursor-pointer rounded p-1 text-muted-foreground hover:text-foreground",
          pinned
            ? "visible"
            : "invisible pointer-events-none group-hover:visible group-hover:pointer-events-auto",
        )}
        aria-label={pinned ? t("unpinSession") : t("pinSession")}
        onClick={(event) => {
          event.stopPropagation();
          void setAgentSessionPinned(session.id, !pinned).then(() =>
            useWorkspaceStore.getState().refreshSessions(),
          );
        }}
      >
        <Pin className={cn("size-3.5", pinned && "fill-current")} />
      </button>
      <span className="flex min-w-0 flex-1 items-center gap-2 text-sm">
        {working ? (
          <Sparkle
            className="size-3.5 shrink-0 animate-spin text-muted-foreground"
            aria-label={t("sessionWorking")}
          />
        ) : null}
        <span className="min-w-0 flex-1 truncate text-left">
          {displaySessionTitle(session.title) ||
            (session.session_kind === "plan" ? "Plan" : t("sessions"))}
        </span>
        <span className="shrink-0 text-[10px] text-muted-foreground">
          {formatRelativeTime(session.started_at, locale)}
        </span>
      </span>
      <button
        type="button"
        className="invisible pointer-events-none cursor-pointer rounded p-1 text-muted-foreground group-hover:visible group-hover:pointer-events-auto hover:text-destructive"
        onClick={(event) => {
          event.stopPropagation();
          void deleteAgentSession(session.id).then(() =>
            useWorkspaceStore.getState().refreshSessions(),
          );
        }}
      >
        <Trash2 className="size-3.5" />
      </button>
    </div>
  );
}

export function SidebarTree() {
  const { t } = useTranslation(["layout", "common"]);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const expanded = useWorkspaceStore((state) => state.expanded);
  const shownCount = useWorkspaceStore((state) => state.shownCount);
  const toggleExpand = useWorkspaceStore((state) => state.toggleExpand);
  const showMore = useWorkspaceStore((state) => state.showMore);
  const setActive = useWorkspaceStore((state) => state.setActive);
  const rename = useWorkspaceStore((state) => state.rename);
  const remove = useWorkspaceStore((state) => state.remove);
  const activeWorkspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [renameTarget, setRenameTarget] = useState<Workspace | null>(null);
  const [renameName, setRenameName] = useState("");
  const [renaming, setRenaming] = useState(false);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const locale = getDateLocale(getCurrentAppLocale());
  const pinnedSessions = sessions
    .filter((session) => session.pinned !== 0)
    .sort((a, b) => b.started_at.localeCompare(a.started_at));

  const openRename = (workspace: Workspace) => {
    setRenameTarget(workspace);
    setRenameName(workspace.name);
  };

  const handleRename = async () => {
    if (!renameTarget || !renameName.trim()) return;
    setRenaming(true);
    try {
      await rename(renameTarget.id, renameName.trim());
      setRenameTarget(null);
    } catch (error) {
      await message(error instanceof Error ? error.message : String(error), { kind: "error" });
    } finally {
      setRenaming(false);
    }
  };

  const handleDelete = async (workspace: Workspace) => {
    const confirmed = await confirm(t("deleteWorkspaceConfirm", { name: workspace.name }), {
      title: t("deleteWorkspaceTitle"),
      kind: "warning",
    });
    if (!confirmed) return;

    const wasActive = activeWorkspaceId === workspace.id;
    try {
      await remove(workspace.id);
      if (wasActive) {
        await setActive(null);
        useSessionStore.getState().selectSession(null);
      }
    } catch (error) {
      await message(error instanceof Error ? error.message : String(error), { kind: "error" });
    }
  };

  return (
    <>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {pinnedSessions.length > 0 ? (
          <div className="mb-2">
            <div className="px-2 py-1.5 text-sm text-muted-foreground">{t("pinnedSection")}</div>
            <div className="mt-1 space-y-1">
              {pinnedSessions.map((session) => (
                <SessionRow
                  key={session.id}
                  session={session}
                  selected={selectedSessionId === session.id}
                  locale={locale}
                />
              ))}
            </div>
          </div>
        ) : null}
        {workspaces.map((workspace) => {
          const items = sessions
            .filter((session) => session.workspace_id === workspace.id && session.pinned === 0)
            .sort((a, b) => b.started_at.localeCompare(a.started_at));
          const limit = shownCount[workspace.id] ?? 5;
          const visible = items.slice(0, limit);
          const open = expanded[workspace.id] !== false;
          return (
            <div key={workspace.id} className="mb-2">
              <div
                className={cn(
                  "group flex items-center rounded-md hover:bg-sidebar-accent",
                  activeWorkspaceId === workspace.id && "bg-sidebar-accent",
                )}
              >
                <button
                  type="button"
                  className="flex min-w-0 flex-1 items-center gap-1 rounded-md px-2 py-1.5 text-sm"
                  onClick={() => {
                    void setActive(workspace.id);
                    toggleExpand(workspace.id);
                  }}
                >
                  <ChevronDown className={cn("size-3.5 transition", !open && "-rotate-90")} />
                  <Folder className="size-3.5 text-muted-foreground" />
                  <span className="flex-1 truncate text-left">{workspace.name}</span>
                </button>
                <button
                  type="button"
                  className="mr-1 rounded p-1 text-muted-foreground hover:text-foreground"
                  aria-label={t("newSession")}
                  onClick={(event) => {
                    event.stopPropagation();
                    void setActive(workspace.id);
                    if (!open) toggleExpand(workspace.id);
                    useSessionStore.getState().selectSession(null);
                    useUiStore.getState().setComposerDraft("");
                  }}
                >
                  <Plus className="size-3.5" />
                </button>
                <DropdownMenu>
                  <DropdownMenuTrigger
                    className="invisible mr-1 rounded p-1 text-muted-foreground group-hover:visible hover:text-foreground"
                    aria-label={t("workspaceActions")}
                  >
                    <MoreHorizontal className="size-3.5" />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => openRename(workspace)}>
                      <Pencil />
                      {t("renameWorkspace")}
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      variant="destructive"
                      onClick={() => void handleDelete(workspace)}
                    >
                      <Trash2 />
                      {t("deleteWorkspace")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
              {open ? (
                <div className="mt-1 space-y-1">
                  {visible.map((session) => (
                    <SessionRow
                      key={session.id}
                      session={session}
                      selected={selectedSessionId === session.id}
                      locale={locale}
                      indent
                    />
                  ))}
                  {items.length > limit ? (
                    <button
                      type="button"
                      className="px-2 pl-8 text-left text-xs text-muted-foreground"
                      onClick={() => showMore(workspace.id)}
                    >
                      {t("moreSessions")}
                    </button>
                  ) : null}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      <Dialog
        open={Boolean(renameTarget)}
        onOpenChange={(open) => {
          if (!open && !renaming) setRenameTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-sm" showCloseButton={!renaming}>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void handleRename();
            }}
          >
            <DialogHeader>
              <DialogTitle>{t("renameWorkspace")}</DialogTitle>
            </DialogHeader>
            <Input
              className="mt-4"
              autoFocus
              value={renameName}
              disabled={renaming}
              onChange={(event) => setRenameName(event.target.value)}
            />
            <DialogFooter className="mt-4">
              <Button
                type="button"
                variant="ghost"
                disabled={renaming}
                onClick={() => setRenameTarget(null)}
              >
                {t("common:cancel")}
              </Button>
              <Button type="submit" disabled={renaming || !renameName.trim()}>
                {t("common:rename")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}
