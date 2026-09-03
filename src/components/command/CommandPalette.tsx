import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { listGitFiles } from "@/lib/backend";
import { displaySessionTitle } from "@/lib/sessionLines";
import { shortcutDisplay, GLOBAL_SHORTCUTS } from "@/lib/shortcuts";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

type Filter = "all" | "actions" | "sessions" | "files";

export function CommandPalette({
  onNewSession,
  onOpenWorkspace,
}: {
  onNewSession: () => void;
  onOpenWorkspace: () => void;
}) {
  const { t } = useTranslation(["layout", "nav"]);
  const open = useUiStore((state) => state.commandOpen);
  const setOpen = useUiStore((state) => state.setCommandOpen);
  const cycleTheme = useUiStore((state) => state.cycleTheme);
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);
  const sessions = useWorkspaceStore((state) => state.sessions);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const loadHistory = useSessionStore((state) => state.loadHistory);
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [files, setFiles] = useState<string[]>([]);
  const [active, setActive] = useState(0);

  useEffect(() => {
    if (!open || !workspaceId) {
      setFiles([]);
      return;
    }
    void listGitFiles(workspaceId, query, 20)
      .then(setFiles)
      .catch(() => setFiles([]));
  }, [open, workspaceId, query]);

  const actions = useMemo(
    () => [
      {
        id: "new",
        label: t("nav:newSession"),
        hint: shortcutDisplay(GLOBAL_SHORTCUTS[0]!),
        run: onNewSession,
        group: "suggested" as const,
      },
      {
        id: "open",
        label: t("nav:shortcuts.openWorkspace"),
        hint: shortcutDisplay(GLOBAL_SHORTCUTS[2]!),
        run: onOpenWorkspace,
        group: "suggested" as const,
      },
      {
        id: "settings",
        label: t("nav:settings"),
        hint: "",
        run: () => void navigate("/settings"),
        group: "suggested" as const,
      },
      {
        id: "sidebar",
        label: t("nav:shortcuts.toggleSidebar"),
        hint: shortcutDisplay(GLOBAL_SHORTCUTS[3]!),
        run: toggleSidebar,
        group: "panels" as const,
      },
      {
        id: "theme",
        label: t("layout:command.theme"),
        hint: "",
        run: cycleTheme,
        group: "panels" as const,
      },
      {
        id: "logs",
        label: t("nav:apiLogs"),
        hint: "",
        run: () => void navigate("/api-logs"),
        group: "panels" as const,
      },
    ],
    [cycleTheme, navigate, onNewSession, onOpenWorkspace, t, toggleSidebar],
  );

  const filteredActions = actions.filter((item) =>
    item.label.toLowerCase().includes(query.toLowerCase()),
  );
  const filteredSessions = sessions
    .filter((item) => {
      const title = displaySessionTitle(item.title).toLowerCase();
      const q = query.toLowerCase();
      return (
        item.id.includes(query) || title.includes(q) || (item.working_dir ?? "").includes(query)
      );
    })
    .slice(0, 8);

  type Row = { id: string; label: string; hint?: string; run: () => void; group: string };
  const rows: Row[] = [];
  if (filter === "all" || filter === "actions") {
    rows.push(...filteredActions);
  }
  if (filter === "all" || filter === "sessions") {
    rows.push(
      ...filteredSessions.map((session) => ({
        id: `s-${session.id}`,
        label: displaySessionTitle(session.title) || session.id.slice(0, 8),
        hint: session.started_at,
        group: "sessions",
        run: () => void loadHistory(session.id),
      })),
    );
  }
  if (filter === "all" || filter === "files") {
    rows.push(
      ...files.map((file) => ({
        id: `f-${file}`,
        label: file,
        group: "files",
        run: () => useUiStore.getState().setComposerDraft(`@${file} `),
      })),
    );
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        setQuery("");
        setActive(0);
      }}
    >
      <DialogContent className="top-[20%] max-w-xl translate-y-0 p-0">
        <Input
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("layout:command.placeholder")}
          className="h-11 rounded-none border-0 border-b"
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((value) => Math.min(rows.length - 1, value + 1));
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((value) => Math.max(0, value - 1));
            }
            if (event.key === "Enter") {
              event.preventDefault();
              rows[active]?.run();
              setOpen(false);
            }
          }}
        />
        <div className="flex gap-1 px-3 py-2 text-xs">
          {(["all", "actions", "sessions", "files"] as Filter[]).map((item) => (
            <button
              key={item}
              type="button"
              className={`rounded-full px-2 py-1 ${filter === item ? "bg-accent" : ""}`}
              onClick={() => setFilter(item)}
            >
              {t(`layout:command.${item === "all" ? "all" : item}`)}
            </button>
          ))}
        </div>
        <div className="max-h-80 overflow-y-auto pb-2">
          {rows.map((row, index) => (
            <button
              key={row.id}
              type="button"
              className={`flex w-full items-center gap-2 px-4 py-2 text-left text-sm ${
                index === active ? "bg-accent" : ""
              }`}
              onMouseEnter={() => setActive(index)}
              onClick={() => {
                row.run();
                setOpen(false);
              }}
            >
              <span className="flex-1 truncate">{row.label}</span>
              {row.hint ? <span className="text-xs text-muted-foreground">{row.hint}</span> : null}
            </button>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
