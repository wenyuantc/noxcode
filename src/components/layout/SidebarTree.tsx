import { confirm, message } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  ArrowDown,
  ArrowUp,
  ChevronDown,
  Folder,
  MoreHorizontal,
  Pencil,
  Pin,
  Plus,
  Sparkle,
  Trash2,
} from "lucide-react";
import { useRef, useState } from "react";
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
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { SessionMenu } from "@/components/session/SessionMenu";
import { useWorkspaceDrag } from "@/hooks/useWorkspaceDrag";
import { displaySessionTitle } from "@/lib/sessionLines";
import { workspaceMoveTarget } from "@/lib/workspaceOrder";
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
  const { t } = useTranslation(["layout", "common"]);
  const loadHistory = useSessionStore((state) => state.loadHistory);
  const working = useSessionStore((state) => state.turnState[session.id] === "working");
  const pinned = session.pinned !== 0;

  return (
    <SessionMenu session={session}>
      {(menuButton) => (
        <div
          role="button"
          data-session-row={session.id}
          tabIndex={0}
          title={session.title ?? undefined}
          className={cn(
            "group relative flex cursor-pointer items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 select-none transition-all duration-150",
            indent && "ml-2.5",
            selected
              ? "bg-sidebar-accent text-sidebar-accent-foreground font-medium shadow-2xs before:absolute before:top-1.5 before:bottom-1.5 before:left-0 before:w-0.5 before:rounded-r before:bg-primary"
              : "text-sidebar-foreground/80 hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
          )}
          onClick={() => void loadHistory(session.id)}
          onKeyDown={(event) => {
            if (event.target !== event.currentTarget) return;
            if (event.key !== "Enter" && event.key !== " ") return;
            event.preventDefault();
            void loadHistory(session.id);
          }}
        >
          <div className="flex min-w-0 flex-1 items-center gap-2">
            {working ? (
              <Sparkle
                className="size-3.5 shrink-0 animate-spin text-amber-500 dark:text-amber-400"
                aria-label={t("sessionWorking")}
              />
            ) : null}
            <span className="min-w-0 flex-1 truncate text-left text-xs leading-tight">
              {displaySessionTitle(session.title) ||
                (session.session_kind === "plan" ? "Plan" : t("sessions"))}
            </span>
          </div>

          <div className="flex shrink-0 items-center gap-1">
            <span
              className={cn(
                "text-[10px] tabular-nums text-muted-foreground/75 transition-opacity",
                pinned && "hidden",
              )}
            >
              {formatRelativeTime(session.started_at, locale)}
            </span>

            {pinned ? (
              <Pin className="size-3 shrink-0 fill-amber-500/90 text-amber-500/90" />
            ) : null}
            {menuButton}
          </div>
        </div>
      )}
    </SessionMenu>
  );
}

export function SidebarTree() {
  const { t } = useTranslation(["layout", "common"]);
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const archivedSessionIds = useWorkspaceStore((state) => state.archivedSessionIds);
  const archivedLoading = useWorkspaceStore((state) => state.archivedLoading);
  const archivedHasMore = useWorkspaceStore((state) => state.archivedHasMore);
  const archivedError = useWorkspaceStore((state) => state.archivedError);
  const loadArchivedSessions = useWorkspaceStore((state) => state.loadArchivedSessions);
  const [archivesOpen, setArchivesOpen] = useState(false);
  const expanded = useWorkspaceStore((state) => state.expanded);
  const shownCount = useWorkspaceStore((state) => state.shownCount);
  const toggleExpand = useWorkspaceStore((state) => state.toggleExpand);
  const showMore = useWorkspaceStore((state) => state.showMore);
  const setActive = useWorkspaceStore((state) => state.setActive);
  const rename = useWorkspaceStore((state) => state.rename);
  const remove = useWorkspaceStore((state) => state.remove);
  const moveWorkspace = useWorkspaceStore((state) => state.moveWorkspace);
  const activeWorkspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const [renameTarget, setRenameTarget] = useState<Workspace | null>(null);
  const [renameName, setRenameName] = useState("");
  const [renaming, setRenaming] = useState(false);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const listRef = useRef<HTMLDivElement>(null);
  const { draggingId, indicatorY, rowProps } = useWorkspaceDrag({
    containerRef: listRef,
    onMove: moveWorkspace,
  });
  const locale = getDateLocale(getCurrentAppLocale());
  const pinnedSessions = sessions
    .filter((session) => session.pinned !== 0 && !session.archived)
    .sort((a, b) => b.started_at.localeCompare(a.started_at));

  const openRename = (workspace: Workspace) => {
    setRenameTarget(workspace);
    setRenameName(workspace.name);
  };

  // Keyboard-reachable equivalent of dragging a row in the workspace list.
  const moveWorkspaceBy = (id: string, direction: "up" | "down") => {
    const target = workspaceMoveTarget(workspaces, id, direction);
    if (target === null) return;
    moveWorkspace(id, target);
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
      <div ref={listRef} className="relative min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {pinnedSessions.length > 0 ? (
          <div className="mb-2.5">
            <div className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
              <Pin className="size-3 fill-amber-500/90 text-amber-500/90" />
              <span>{t("pinnedSection")}</span>
              <span className="ml-auto rounded-full bg-muted/60 px-1.5 font-mono text-[10px] text-muted-foreground">
                {pinnedSessions.length}
              </span>
            </div>
            <div className="mt-1 space-y-0.5">
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
        {workspaces.map((workspace, workspaceIndex) => {
          const items = sessions
            .filter(
              (session) =>
                session.workspace_id === workspace.id && session.pinned === 0 && !session.archived,
            )
            .sort((a, b) => b.started_at.localeCompare(a.started_at));
          const limit = shownCount[workspace.id] ?? 5;
          const visible = items.slice(0, limit);
          const open = expanded[workspace.id] !== false;
          const canMoveUp = workspaceIndex > 0;
          const canMoveDown = workspaceIndex < workspaces.length - 1;
          const dragging = draggingId === workspace.id;
          return (
            <div key={workspace.id} data-workspace-block className="mb-2">
              <div
                {...rowProps(workspace.id)}
                className={cn(
                  "group flex items-center justify-between rounded-lg px-2 py-1 select-none transition-colors",
                  dragging ? "cursor-grabbing opacity-60" : "cursor-grab",
                  activeWorkspaceId === workspace.id
                    ? "bg-sidebar-accent/50 text-sidebar-foreground"
                    : "text-sidebar-foreground/90 hover:bg-sidebar-accent/40",
                )}
              >
                <button
                  type="button"
                  className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md text-left text-xs font-semibold"
                  onClick={() => {
                    void setActive(workspace.id);
                    toggleExpand(workspace.id);
                  }}
                >
                  <ChevronDown
                    className={cn(
                      "size-3.5 shrink-0 text-muted-foreground transition-transform duration-150",
                      !open && "-rotate-90",
                    )}
                  />
                  <Folder className="size-3.5 shrink-0 text-muted-foreground" />
                  <span className="flex-1 truncate tracking-tight">{workspace.name}</span>
                  {items.length > 0 ? (
                    <span className="mr-1 inline-flex items-center rounded-full bg-muted/50 px-1.5 font-mono text-[10px] font-normal text-muted-foreground">
                      {items.length}
                    </span>
                  ) : null}
                </button>
                <div
                  data-no-drag
                  className="flex cursor-default items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100"
                >
                  <button
                    type="button"
                    className="rounded p-1 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground"
                    title={t("newSession")}
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
                      className="rounded p-1 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground"
                      aria-label={t("workspaceActions")}
                    >
                      <MoreHorizontal className="size-3.5" />
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        disabled={!canMoveUp}
                        onClick={() => moveWorkspaceBy(workspace.id, "up")}
                      >
                        <ArrowUp />
                        {t("moveWorkspaceUp")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={!canMoveDown}
                        onClick={() => moveWorkspaceBy(workspace.id, "down")}
                      >
                        <ArrowDown />
                        {t("moveWorkspaceDown")}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
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
              </div>
              {open ? (
                <div className="mt-0.5 space-y-0.5">
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
                      className="ml-5 mt-1 flex items-center gap-1 rounded px-2 py-1 text-[11px] font-medium text-muted-foreground/80 transition-colors hover:bg-sidebar-accent/50 hover:text-foreground"
                      onClick={() => showMore(workspace.id)}
                    >
                      <span>{t("moreSessions")}</span>
                      <span className="font-mono text-[10px]">({items.length - limit})</span>
                    </button>
                  ) : null}
                </div>
              ) : null}
            </div>
          );
        })}
        {indicatorY !== null ? (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-2 h-0.5 rounded-full bg-primary"
            style={{ top: indicatorY }}
          />
        ) : null}
        <div className="mt-3 border-t border-sidebar-border/60 pt-2">
          <button
            type="button"
            className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs font-semibold text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground"
            aria-expanded={archivesOpen}
            onClick={() => {
              setArchivesOpen(!archivesOpen);
              if (!archivesOpen) void loadArchivedSessions();
            }}
          >
            <ChevronDown
              className={cn(
                "size-3.5 shrink-0 transition-transform",
                !archivesOpen && "-rotate-90",
              )}
            />
            <Archive className="size-3.5 shrink-0" />
            {t("sessionMenu.archivedSection")}
          </button>
          {archivesOpen ? (
            <div className="mt-1 space-y-0.5">
              {archivedSessionIds.map((id) => {
                const session = sessions.find((item) => item.id === id && item.archived);
                return session ? (
                  <SessionRow
                    key={id}
                    session={session}
                    selected={selectedSessionId === id}
                    locale={locale}
                    indent
                  />
                ) : null;
              })}
              {archivedError ? (
                <div role="alert" className="px-3 py-2 text-xs break-words text-destructive">
                  <p>{archivedError}</p>
                  <Button variant="ghost" size="sm" onClick={() => void loadArchivedSessions()}>
                    {t("common:retry")}
                  </Button>
                </div>
              ) : archivedLoading ? (
                <p role="status" className="px-5 py-2 text-xs text-muted-foreground">
                  {t("common:loading")}
                </p>
              ) : archivedSessionIds.length === 0 ? (
                <p className="px-5 py-2 text-xs text-muted-foreground">
                  {t("sessionMenu.archivedEmpty")}
                </p>
              ) : archivedHasMore ? (
                <button
                  type="button"
                  className="ml-5 rounded px-2 py-1 text-[11px] text-muted-foreground hover:bg-sidebar-accent/50"
                  onClick={() => void loadArchivedSessions(true)}
                >
                  {t("moreSessions")}
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
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
