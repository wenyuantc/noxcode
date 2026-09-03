import { ChevronDown, Folder, Pin, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { deleteAgentSession, setAgentSessionPinned } from "@/lib/backend";
import { displaySessionTitle } from "@/lib/sessionLines";
import { formatRelativeTime } from "@/lib/utils";
import { getCurrentAppLocale, getDateLocale } from "@/lib/i18n/locale";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { cn } from "@/lib/utils";
import type { AgentSession } from "@/lib/types";

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
  const pinned = session.pinned !== 0;

  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        "group flex items-center rounded-md p-[10px] hover:bg-sidebar-accent",
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
          "rounded p-1 text-muted-foreground hover:text-foreground",
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
        className="invisible pointer-events-none rounded p-1 text-muted-foreground group-hover:visible group-hover:pointer-events-auto hover:text-destructive"
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
  const { t } = useTranslation("layout");
  const workspaces = useWorkspaceStore((state) => state.workspaces);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const expanded = useWorkspaceStore((state) => state.expanded);
  const shownCount = useWorkspaceStore((state) => state.shownCount);
  const toggleExpand = useWorkspaceStore((state) => state.toggleExpand);
  const showMore = useWorkspaceStore((state) => state.showMore);
  const setActive = useWorkspaceStore((state) => state.setActive);
  const activeWorkspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const locale = getDateLocale(getCurrentAppLocale());
  const pinnedSessions = sessions
    .filter((session) => session.pinned !== 0)
    .sort((a, b) => b.started_at.localeCompare(a.started_at));

  return (
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
            <button
              type="button"
              className={cn(
                "flex w-full items-center gap-1 rounded-md px-2 py-1.5 text-sm hover:bg-sidebar-accent",
                activeWorkspaceId === workspace.id && "bg-sidebar-accent",
              )}
              onClick={() => {
                void setActive(workspace.id);
                toggleExpand(workspace.id);
              }}
            >
              <ChevronDown className={cn("size-3.5 transition", !open && "-rotate-90")} />
              <Folder className="size-3.5 text-muted-foreground" />
              <span className="flex-1 truncate text-left">{workspace.name}</span>
            </button>
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
  );
}
