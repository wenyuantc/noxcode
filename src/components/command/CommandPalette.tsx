import {
  Activity,
  FileText,
  FolderOpen,
  MessageSquare,
  PanelLeft,
  Search,
  Settings,
  SquarePen,
  SunMoon,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { Dialog, DialogContent } from "@/components/ui/dialog";
import { listGitFiles } from "@/lib/backend";
import { getCurrentAppLocale, getDateLocale } from "@/lib/i18n/locale";
import { displaySessionTitle } from "@/lib/sessionLines";
import { GLOBAL_SHORTCUTS, shortcutDisplay } from "@/lib/shortcuts";
import { cn, formatRelativeTime } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

type Filter = "all" | "actions" | "sessions" | "files";

type Row = {
  id: string;
  label: string;
  sublabel?: string;
  hint?: string;
  icon: React.ComponentType<{ className?: string }>;
  iconColor?: string;
  run: () => void;
  group: string;
};

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
  const locale = getDateLocale(getCurrentAppLocale());

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
        icon: SquarePen,
        iconColor: "text-sky-500 dark:text-sky-400",
        run: onNewSession,
        group: "suggested" as const,
      },
      {
        id: "open",
        label: t("nav:shortcuts.openWorkspace"),
        hint: shortcutDisplay(GLOBAL_SHORTCUTS[2]!),
        icon: FolderOpen,
        iconColor: "text-amber-500 dark:text-amber-400",
        run: onOpenWorkspace,
        group: "suggested" as const,
      },
      {
        id: "settings",
        label: t("nav:settings"),
        hint: "",
        icon: Settings,
        iconColor: "text-slate-500 dark:text-slate-400",
        run: () => void navigate("/settings"),
        group: "suggested" as const,
      },
      {
        id: "sidebar",
        label: t("nav:shortcuts.toggleSidebar"),
        hint: shortcutDisplay(GLOBAL_SHORTCUTS[3]!),
        icon: PanelLeft,
        iconColor: "text-indigo-500 dark:text-indigo-400",
        run: toggleSidebar,
        group: "panels" as const,
      },
      {
        id: "theme",
        label: t("layout:command.theme"),
        hint: "",
        icon: SunMoon,
        iconColor: "text-purple-500 dark:text-purple-400",
        run: cycleTheme,
        group: "panels" as const,
      },
      {
        id: "logs",
        label: t("nav:apiLogs"),
        hint: "",
        icon: Activity,
        iconColor: "text-emerald-500 dark:text-emerald-400",
        run: () => void navigate("/api-logs"),
        group: "panels" as const,
      },
    ],
    [cycleTheme, navigate, onNewSession, onOpenWorkspace, t, toggleSidebar],
  );

  const filteredActions: Row[] = useMemo(
    () =>
      actions
        .filter((item) => item.label.toLowerCase().includes(query.toLowerCase()))
        .map((item) => ({ ...item })),
    [actions, query],
  );

  const filteredSessions: Row[] = useMemo(
    () =>
      sessions
        .filter((item) => {
          const title = displaySessionTitle(item.title).toLowerCase();
          const q = query.toLowerCase();
          return (
            item.id.includes(query) || title.includes(q) || (item.working_dir ?? "").includes(query)
          );
        })
        .slice(0, 10)
        .map((session) => ({
          id: `s-${session.id}`,
          label: displaySessionTitle(session.title) || session.id.slice(0, 8),
          sublabel: session.working_dir
            ? session.working_dir.split("/").slice(-2).join("/")
            : undefined,
          hint: formatRelativeTime(session.started_at, locale),
          icon: MessageSquare,
          iconColor: "text-violet-500 dark:text-violet-400",
          group: "sessions",
          run: () => void loadHistory(session.id),
        })),
    [sessions, query, locale, loadHistory],
  );

  const filteredFiles: Row[] = useMemo(
    () =>
      files.map((file) => {
        const lastSlash = file.lastIndexOf("/");
        const name = lastSlash >= 0 ? file.slice(lastSlash + 1) : file;
        const dir = lastSlash >= 0 ? file.slice(0, lastSlash) : undefined;
        return {
          id: `f-${file}`,
          label: name,
          sublabel: dir,
          icon: FileText,
          iconColor: "text-cyan-500 dark:text-cyan-400",
          group: "files",
          run: () => useUiStore.getState().setComposerDraft(`@${file} `),
        };
      }),
    [files],
  );

  const rows: Row[] = useMemo(() => {
    const list: Row[] = [];
    if (filter === "all" || filter === "actions") {
      list.push(...filteredActions);
    }
    if (filter === "all" || filter === "sessions") {
      list.push(...filteredSessions);
    }
    if (filter === "all" || filter === "files") {
      list.push(...filteredFiles);
    }
    return list;
  }, [filter, filteredActions, filteredSessions, filteredFiles]);

  const filterTabs: { id: Filter; label: string }[] = [
    { id: "all", label: t("layout:command.all") },
    { id: "actions", label: t("layout:command.actions") },
    { id: "sessions", label: t("layout:command.sessions") },
    { id: "files", label: t("layout:command.files") },
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        setQuery("");
        setActive(0);
      }}
    >
      <DialogContent
        showCloseButton={false}
        className="top-[18%] max-w-xl translate-y-0 overflow-hidden rounded-2xl border border-border/80 bg-popover/95 p-0 shadow-2xl backdrop-blur-md"
      >
        <div className="flex items-center gap-2.5 border-b border-border/60 px-3.5 py-2.5">
          <Search className="size-4 shrink-0 text-muted-foreground/70" />
          <input
            autoFocus
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            placeholder={t("layout:command.placeholder")}
            className="h-7 w-full bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground/60"
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
          <kbd className="inline-flex shrink-0 items-center rounded border border-border/50 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground/70 shadow-2xs">
            ESC
          </kbd>
        </div>

        <div className="flex items-center gap-1.5 border-b border-border/50 bg-muted/15 px-3 py-1.5 text-xs">
          {filterTabs.map((item) => (
            <button
              key={item.id}
              type="button"
              className={cn(
                "cursor-pointer rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-150",
                filter === item.id
                  ? "bg-accent text-accent-foreground font-semibold shadow-2xs"
                  : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
              )}
              onClick={() => {
                setFilter(item.id);
                setActive(0);
              }}
            >
              {item.label}
            </button>
          ))}
        </div>

        <div className="max-h-80 overflow-y-auto p-1.5">
          {rows.length === 0 ? (
            <div className="py-8 text-center text-xs text-muted-foreground">
              <p>{t("layout:command.noResults", { defaultValue: "无匹配结果" })}</p>
            </div>
          ) : (
            rows.map((row, index) => {
              const Icon = row.icon;
              const isSelected = index === active;
              return (
                <button
                  key={row.id}
                  type="button"
                  className={cn(
                    "flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs transition-colors duration-100",
                    isSelected
                      ? "bg-accent text-accent-foreground font-medium shadow-2xs"
                      : "text-foreground/80 hover:bg-muted/40 hover:text-foreground",
                  )}
                  onMouseEnter={() => setActive(index)}
                  onClick={() => {
                    row.run();
                    setOpen(false);
                  }}
                >
                  <div
                    className={cn(
                      "flex size-6 shrink-0 items-center justify-center rounded-md border border-border/40 bg-background/80 shadow-2xs",
                      row.iconColor,
                    )}
                  >
                    <Icon className="size-3.5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs leading-tight">{row.label}</div>
                    {row.sublabel ? (
                      <div className="truncate font-mono text-[10.5px] text-muted-foreground/70">
                        {row.sublabel}
                      </div>
                    ) : null}
                  </div>
                  {row.hint ? (
                    row.hint.startsWith("⌘") || row.hint.startsWith("Ctrl") ? (
                      <kbd className="inline-flex shrink-0 items-center rounded border border-border/50 bg-background/60 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground shadow-2xs">
                        {row.hint}
                      </kbd>
                    ) : (
                      <span className="shrink-0 font-mono text-[10px] text-muted-foreground/75 tabular-nums">
                        {row.hint}
                      </span>
                    )
                  ) : null}
                </button>
              );
            })
          )}
        </div>

        <div className="flex items-center justify-between border-t border-border/40 bg-muted/15 px-3 py-1.5 text-[11px] text-muted-foreground/60">
          <div className="flex items-center gap-3">
            <span className="flex items-center gap-1">
              <kbd className="rounded border border-border/40 bg-muted/30 px-1 py-0.2 font-mono text-[10px]">
                ↑↓
              </kbd>
              <span>选择</span>
            </span>
            <span className="flex items-center gap-1">
              <kbd className="rounded border border-border/40 bg-muted/30 px-1 py-0.2 font-mono text-[10px]">
                ↵
              </kbd>
              <span>确认</span>
            </span>
          </div>
          <span className="font-mono text-[10px]">{rows.length} 项</span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
