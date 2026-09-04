import { confirm } from "@tauri-apps/plugin-dialog";
import {
  CheckCheck,
  CheckCircle2,
  ChevronRight,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitCommit,
  GitFork,
  History,
  Minus,
  Plus,
  RefreshCw,
  RotateCcw,
  Trash2,
  UploadCloud,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import {
  clearGitCheckpoints,
  commitGitChanges,
  getGitFileDiff,
  getGitStatus,
  listActivityLogs,
  listGitCheckpoints,
  previewGitCheckpointRestore,
  pushGitBranch,
  restoreGitCheckpoint,
  restoreGitPaths,
  stageGitPaths,
  unstageGitPaths,
} from "@/lib/backend";
import { groupGitStatus } from "@/lib/gitHelpers";
import type {
  ActivityLog,
  GitCheckpoint,
  GitFileDiff,
  GitRestorePreview,
  GitStatus,
  GitStatusEntry,
} from "@/lib/types";
import { cn, formatRelativeTime } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { CheckpointTimeline } from "./CheckpointTimeline";
import { DiffView } from "./DiffView";
import { RestoreCheckpointDialog } from "./RestoreCheckpointDialog";

export function GitPanel() {
  const { t, i18n } = useTranslation("git");
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const gitFocusPath = useUiStore((state) => state.gitFocusPath);
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [checkpoints, setCheckpoints] = useState<GitCheckpoint[]>([]);
  const [activityLogs, setActivityLogs] = useState<ActivityLog[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [diff, setDiff] = useState<GitFileDiff | null>(null);
  const [message, setMessage] = useState("");
  const [restoreTarget, setRestoreTarget] = useState<GitCheckpoint | null>(null);
  const [preview, setPreview] = useState<GitRestorePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [checkpointNotice, setCheckpointNotice] = useState<string | null>(null);
  const [checkpointError, setCheckpointError] = useState<string | null>(null);

  const reloadStatus = useCallback(async () => {
    if (!workspaceId) return;
    setStatus(await getGitStatus(workspaceId));
  }, [workspaceId]);

  const reloadCheckpoints = useCallback(async () => {
    if (!workspaceId || !sessionId) {
      setCheckpoints([]);
      return;
    }
    setCheckpoints(await listGitCheckpoints(workspaceId, sessionId));
  }, [sessionId, workspaceId]);

  const reloadActivityLogs = useCallback(async () => {
    if (!workspaceId) {
      setActivityLogs([]);
      return;
    }
    setActivityLogs(await listActivityLogs(workspaceId, 20));
  }, [workspaceId]);

  const reload = useCallback(async () => {
    await Promise.all([reloadStatus(), reloadCheckpoints(), reloadActivityLogs()]);
  }, [reloadActivityLogs, reloadCheckpoints, reloadStatus]);

  useEffect(() => {
    void reloadStatus();
  }, [reloadStatus]);

  useEffect(() => {
    void reloadCheckpoints();
  }, [reloadCheckpoints]);

  useEffect(() => {
    void reloadActivityLogs();
  }, [reloadActivityLogs]);

  useEffect(() => {
    if (!workspaceId || !gitFocusPath) return;
    void getGitFileDiff(workspaceId, gitFocusPath, "worktree").then(setDiff);
  }, [gitFocusPath, workspaceId]);

  const groups = groupGitStatus(status);
  const totalChanges = groups.staged.length + groups.unstaged.length + groups.untracked.length;
  const allUnstaged = [...groups.unstaged, ...groups.untracked];
  const allUnstagedPaths = allUnstaged.map((entry) => entry.path);

  const selectedOf = (entries: GitStatusEntry[]) =>
    entries.filter((entry) => selected.has(entry.path)).map((entry) => entry.path);

  const selectedUnstagedPaths = selectedOf(allUnstaged);
  const selectedStagedPaths = selectedOf(groups.staged);

  const toggle = (path: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleGroup = (entries: GitStatusEntry[]) => {
    setSelected((current) => {
      const next = new Set(current);
      const allSelected = entries.every((e) => next.has(e.path));
      if (allSelected) {
        entries.forEach((e) => next.delete(e.path));
      } else {
        entries.forEach((e) => next.add(e.path));
      }
      return next;
    });
  };

  const showDiff = (entry: GitStatusEntry, scope: "worktree" | "staged") => {
    if (!workspaceId) return;
    if (diff?.path === entry.path) {
      setDiff(null);
      return;
    }
    void getGitFileDiff(workspaceId, entry.path, scope, entry.orig_path ?? undefined).then(setDiff);
  };

  const handleStageAll = async () => {
    if (!workspaceId || allUnstagedPaths.length === 0) return;
    setBusy(true);
    try {
      await stageGitPaths(workspaceId, allUnstagedPaths);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleStageSelected = async () => {
    if (!workspaceId || selectedUnstagedPaths.length === 0) return;
    setBusy(true);
    try {
      await stageGitPaths(workspaceId, selectedUnstagedPaths);
      setSelected((curr) => {
        const next = new Set(curr);
        selectedUnstagedPaths.forEach((p) => next.delete(p));
        return next;
      });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleUnstageSelected = async () => {
    if (!workspaceId || selectedStagedPaths.length === 0) return;
    setBusy(true);
    try {
      await unstageGitPaths(workspaceId, selectedStagedPaths);
      setSelected((curr) => {
        const next = new Set(curr);
        selectedStagedPaths.forEach((p) => next.delete(p));
        return next;
      });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleDiscardSelected = async () => {
    if (!workspaceId || selectedUnstagedPaths.length === 0) return;
    const accepted = await confirm(t("discardConfirm"), {
      title: t("discard"),
      kind: "warning",
    });
    if (!accepted) return;
    setBusy(true);
    try {
      await restoreGitPaths(workspaceId, selectedUnstagedPaths);
      setSelected((curr) => {
        const next = new Set(curr);
        selectedUnstagedPaths.forEach((p) => next.delete(p));
        return next;
      });
      if (diff && selectedUnstagedPaths.includes(diff.path)) {
        setDiff(null);
      }
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleStageSingle = async (path: string) => {
    if (!workspaceId || busy) return;
    setBusy(true);
    try {
      await stageGitPaths(workspaceId, [path]);
      setSelected((curr) => {
        const next = new Set(curr);
        next.delete(path);
        return next;
      });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleUnstageSingle = async (path: string) => {
    if (!workspaceId || busy) return;
    setBusy(true);
    try {
      await unstageGitPaths(workspaceId, [path]);
      setSelected((curr) => {
        const next = new Set(curr);
        next.delete(path);
        return next;
      });
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleDiscardSingle = async (path: string) => {
    if (!workspaceId || busy) return;
    const accepted = await confirm(t("discardFileConfirm"), {
      title: t("discard"),
      kind: "warning",
    });
    if (!accepted) return;
    setBusy(true);
    try {
      await restoreGitPaths(workspaceId, [path]);
      setSelected((curr) => {
        const next = new Set(curr);
        next.delete(path);
        return next;
      });
      if (diff?.path === path) {
        setDiff(null);
      }
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleCommit = async () => {
    if (!workspaceId || !message.trim() || busy) return;
    setBusy(true);
    try {
      await commitGitChanges(workspaceId, message.trim());
      setMessage("");
      if (diff) setDiff(null);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handlePush = async (setUpstream = false) => {
    if (!workspaceId || busy) return;
    setBusy(true);
    try {
      await pushGitBranch(workspaceId, undefined, undefined, setUpstream);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const clearAllCheckpoints = async () => {
    if (!workspaceId) return;
    const accepted = await confirm(t("clearAllConfirm"), {
      title: t("clearAllTitle"),
      kind: "warning",
    });
    if (!accepted) return;
    setBusy(true);
    setCheckpointNotice(null);
    setCheckpointError(null);
    try {
      const count = await clearGitCheckpoints(workspaceId);
      setCheckpointNotice(t("clearAllDone", { count }));
      await reload();
    } catch (error) {
      setCheckpointError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const restoreLogs = activityLogs.filter((log) => log.kind.startsWith("git_checkpoint_restore"));

  return (
    <div className="flex h-full min-h-0 flex-col bg-background/50">
      {/* Panel Header */}
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-border/70 px-3">
        <div className="flex min-w-0 items-center gap-2">
          <FolderGit2 className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="text-xs font-semibold tracking-tight text-foreground">{t("panel")}</span>
          {status?.branch?.head ? (
            <div className="flex max-w-[140px] items-center gap-1 truncate rounded-md bg-accent/60 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">
              <GitBranch className="size-3 shrink-0" />
              <span className="truncate">{status.branch.head}</span>
              {Boolean(status.branch.ahead) && (
                <span className="font-mono text-[10px] text-emerald-600 dark:text-emerald-400">
                  ↑{status.branch.ahead}
                </span>
              )}
              {Boolean(status.branch.behind) && (
                <span className="font-mono text-[10px] text-amber-600 dark:text-amber-400">
                  ↓{status.branch.behind}
                </span>
              )}
            </div>
          ) : null}
        </div>
        <div className="flex items-center gap-0.5">
          <Button
            size="icon-xs"
            variant="ghost"
            className="text-muted-foreground hover:text-foreground"
            title={t("refresh")}
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void reload().finally(() => setBusy(false));
            }}
          >
            <RefreshCw className={cn("size-3", busy && "animate-spin")} />
          </Button>
          <Button
            size="icon-xs"
            variant="ghost"
            className="text-muted-foreground hover:text-foreground"
            title={t("close")}
            onClick={() => useUiStore.getState().toggleGit()}
          >
            <X className="size-3" />
          </Button>
        </div>
      </div>

      <Tabs defaultValue="changes" className="flex min-h-0 flex-1 flex-col">
        <div className="px-3 pt-2">
          <TabsList className="grid h-7 w-full grid-cols-2 rounded-lg bg-muted/60 p-0.5">
            <TabsTrigger value="changes" className="h-6 text-xs data-[state=active]:shadow-2xs">
              {t("changes")}
              {totalChanges > 0 ? (
                <span className="ml-1.5 rounded-full bg-foreground/10 px-1.5 py-0.2 text-[10px] font-medium text-foreground">
                  {totalChanges}
                </span>
              ) : null}
            </TabsTrigger>
            <TabsTrigger value="checkpoints" className="h-6 text-xs data-[state=active]:shadow-2xs">
              {t("checkpoints")}
              {checkpoints.length > 0 ? (
                <span className="ml-1.5 rounded-full bg-foreground/10 px-1.5 py-0.2 text-[10px] font-medium text-foreground">
                  {checkpoints.length}
                </span>
              ) : null}
            </TabsTrigger>
          </TabsList>
        </div>

        {/* Changes Tab */}
        <TabsContent value="changes" className="min-h-0 flex-1 overflow-y-auto px-3 py-2.5">
          <div className="space-y-3">
            {/* Commit Message Box */}
            <div className="space-y-2 rounded-xl border border-border/70 bg-card/40 p-2.5 shadow-2xs backdrop-blur-xs">
              <div className="relative">
                <Textarea
                  value={message}
                  placeholder={t("commitMessage")}
                  className="h-20 min-h-20 max-h-20 w-full resize-none rounded-lg border border-input/60 bg-background/80 p-2 font-sans text-xs leading-relaxed shadow-2xs transition-all placeholder:text-muted-foreground/50 focus-visible:border-ring/60 focus-visible:ring-1"
                  disabled={!workspaceId || busy}
                  onChange={(event) => setMessage(event.target.value)}
                  onKeyDown={(event) => {
                    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                      event.preventDefault();
                      void handleCommit();
                    }
                  }}
                />
                <span className="pointer-events-none absolute bottom-1.5 right-2 text-[10px] font-sans text-muted-foreground/40">
                  ⌘↵
                </span>
              </div>
              <div className="flex items-center gap-1.5">
                <Button
                  size="sm"
                  className="flex-1 text-xs font-medium shadow-2xs"
                  disabled={!workspaceId || !message.trim() || busy}
                  onClick={() => void handleCommit()}
                >
                  <GitCommit className="mr-1.5 size-3.5" />
                  {t("commit")}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="text-xs"
                  title={t("push")}
                  disabled={!workspaceId || busy}
                  onClick={() => void handlePush(false)}
                >
                  <UploadCloud className="mr-1 size-3.5" />
                  {t("push")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  className="px-2 text-xs text-muted-foreground hover:text-foreground"
                  title={t("setUpstream")}
                  disabled={!workspaceId || busy}
                  onClick={() => void handlePush(true)}
                >
                  <GitFork className="size-3.5" />
                </Button>
              </div>
            </div>

            {/* Quick Action Toolbar */}
            <div className="flex items-center justify-between gap-1 border-y border-border/50 py-1.5">
              <Button
                size="xs"
                variant="secondary"
                className="h-6.5 bg-secondary/80 text-xs font-medium hover:bg-secondary"
                disabled={!workspaceId || allUnstagedPaths.length === 0 || busy}
                onClick={() => void handleStageAll()}
              >
                <CheckCheck className="mr-1 size-3" />
                {t("stageAll")}
                {allUnstagedPaths.length > 0 ? (
                  <span className="ml-1 rounded-full bg-foreground/10 px-1 text-[10px]">
                    {allUnstagedPaths.length}
                  </span>
                ) : null}
              </Button>

              <div className="flex items-center gap-1">
                <Button
                  size="xs"
                  variant="outline"
                  className="h-6.5 text-[11px]"
                  disabled={!workspaceId || selectedUnstagedPaths.length === 0 || busy}
                  onClick={() => void handleStageSelected()}
                >
                  <Plus className="mr-0.5 size-2.5" />
                  {t("stage")}
                </Button>
                <Button
                  size="xs"
                  variant="outline"
                  className="h-6.5 text-[11px]"
                  disabled={!workspaceId || selectedStagedPaths.length === 0 || busy}
                  onClick={() => void handleUnstageSelected()}
                >
                  <Minus className="mr-0.5 size-2.5" />
                  {t("unstage")}
                </Button>
                <Button
                  size="xs"
                  variant="ghost"
                  className="h-6.5 text-[11px] text-muted-foreground hover:text-destructive"
                  disabled={!workspaceId || selectedUnstagedPaths.length === 0 || busy}
                  onClick={() => void handleDiscardSelected()}
                >
                  <Trash2 className="mr-0.5 size-2.5" />
                  {t("discard")}
                </Button>
              </div>
            </div>

            {/* File Changes Groups */}
            <div className="space-y-2">
              <FileGroup
                title={t("staged")}
                entries={groups.staged}
                selected={selected}
                isStagedGroup
                activeDiffPath={diff?.path}
                onToggle={toggle}
                onToggleAll={() => toggleGroup(groups.staged)}
                onOpen={(entry) => showDiff(entry, "staged")}
                onUnstageSingle={handleUnstageSingle}
                onActionAll={() => {
                  if (!workspaceId) return;
                  setBusy(true);
                  void unstageGitPaths(
                    workspaceId,
                    groups.staged.map((e) => e.path),
                  )
                    .then(reload)
                    .finally(() => setBusy(false));
                }}
                actionAllTooltip={t("unstage")}
              />

              <FileGroup
                title={t("unstaged")}
                entries={groups.unstaged}
                selected={selected}
                activeDiffPath={diff?.path}
                onToggle={toggle}
                onToggleAll={() => toggleGroup(groups.unstaged)}
                onOpen={(entry) => showDiff(entry, "worktree")}
                onStageSingle={handleStageSingle}
                onDiscardSingle={handleDiscardSingle}
                onActionAll={() => {
                  if (!workspaceId) return;
                  setBusy(true);
                  void stageGitPaths(
                    workspaceId,
                    groups.unstaged.map((e) => e.path),
                  )
                    .then(reload)
                    .finally(() => setBusy(false));
                }}
                actionAllTooltip={t("stage")}
              />

              <FileGroup
                title={t("untracked")}
                entries={groups.untracked}
                selected={selected}
                activeDiffPath={diff?.path}
                onToggle={toggle}
                onToggleAll={() => toggleGroup(groups.untracked)}
                onOpen={(entry) => showDiff(entry, "worktree")}
                onStageSingle={handleStageSingle}
                onDiscardSingle={handleDiscardSingle}
                onActionAll={() => {
                  if (!workspaceId) return;
                  setBusy(true);
                  void stageGitPaths(
                    workspaceId,
                    groups.untracked.map((e) => e.path),
                  )
                    .then(reload)
                    .finally(() => setBusy(false));
                }}
                actionAllTooltip={t("stage")}
              />

              {totalChanges === 0 ? (
                <div className="my-6 flex flex-col items-center justify-center rounded-xl border border-dashed border-border/70 p-8 text-center">
                  <CheckCircle2 className="mb-2.5 size-8 stroke-[1.5] text-emerald-500/80" />
                  <p className="text-xs font-medium text-foreground">{t("empty")}</p>
                  <p className="mt-1 text-[11px] text-muted-foreground">{t("noChanges")}</p>
                </div>
              ) : null}
            </div>

            {/* Diff Preview Drawer */}
            {diff ? (
              <div className="mt-3 overflow-hidden rounded-xl border border-border/80 bg-card/60 shadow-2xs">
                <div className="flex items-center justify-between border-b border-border/60 bg-muted/40 px-3 py-1.5">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <FileDiff className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate text-xs font-medium text-foreground">
                      {diff.path}
                    </span>
                  </div>
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    className="size-5 text-muted-foreground hover:text-foreground"
                    onClick={() => setDiff(null)}
                    title={t("close")}
                  >
                    <X className="size-3" />
                  </Button>
                </div>
                <div className="max-h-[300px] overflow-auto">
                  <DiffView diff={diff} />
                </div>
              </div>
            ) : null}
          </div>
        </TabsContent>

        {/* Checkpoints Tab */}
        <TabsContent value="checkpoints" className="min-h-0 flex-1 overflow-y-auto px-3 py-2.5">
          <div className="mb-3 flex justify-end">
            <Button
              size="xs"
              variant="outline"
              className="h-6.5 text-[11px] text-destructive hover:bg-destructive/10 hover:text-destructive"
              disabled={!workspaceId || busy}
              onClick={() => void clearAllCheckpoints()}
            >
              <Trash2 className="mr-1 size-3" />
              {t("clearAll")}
            </Button>
          </div>
          {checkpointNotice ? (
            <div className="mb-3 rounded-lg border border-border/60 bg-muted/40 p-2 text-xs text-muted-foreground">
              {checkpointNotice}
            </div>
          ) : null}
          {checkpointError ? (
            <div className="mb-3 rounded-lg border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive">
              {checkpointError}
            </div>
          ) : null}
          <CheckpointTimeline
            checkpoints={checkpoints}
            onRestore={(checkpoint) => {
              if (!workspaceId) return;
              setRestoreTarget(checkpoint);
              void previewGitCheckpointRestore(workspaceId, checkpoint.id).then(setPreview);
            }}
          />
          <Collapsible className="mt-4 rounded-xl border border-border/70 bg-card/40">
            <CollapsibleTrigger className="flex w-full items-center justify-between px-3 py-2 text-left text-xs font-medium text-muted-foreground hover:text-foreground">
              <span className="flex items-center gap-1.5">
                <History className="size-3.5" />
                {t("restoreHistory")}
              </span>
              <span className="rounded-full bg-muted px-1.5 py-0.2 text-[10px]">
                {restoreLogs.length}
              </span>
            </CollapsibleTrigger>
            <CollapsibleContent className="border-t border-border/60 px-3 py-2">
              {restoreLogs.length === 0 ? (
                <p className="text-xs text-muted-foreground">{t("restoreHistoryEmpty")}</p>
              ) : (
                <ol className="space-y-2">
                  {restoreLogs.map((log) => (
                    <li key={log.id} className="rounded-md bg-muted/30 p-2">
                      <p className="text-xs font-medium">{log.summary}</p>
                      <p className="mt-0.5 text-[11px] text-muted-foreground">
                        {formatRelativeTime(log.created_at, i18n.language)}
                      </p>
                    </li>
                  ))}
                </ol>
              )}
            </CollapsibleContent>
          </Collapsible>
        </TabsContent>
      </Tabs>

      <RestoreCheckpointDialog
        open={Boolean(restoreTarget)}
        checkpoint={restoreTarget}
        preview={preview}
        busy={busy}
        onOpenChange={(open) => {
          if (!open) {
            setRestoreTarget(null);
            setPreview(null);
          }
        }}
        onConfirm={(deleteNewPaths) => {
          if (!workspaceId || !restoreTarget) return;
          setBusy(true);
          void restoreGitCheckpoint(workspaceId, restoreTarget.id, deleteNewPaths)
            .then(() => {
              setRestoreTarget(null);
              setPreview(null);
              void reload();
            })
            .finally(() => setBusy(false));
        }}
      />
    </div>
  );
}

function StatusBadge({ xy }: { xy: string }) {
  const code = xy.trim() || "?";
  const char = code.charAt(0);
  let colorClass = "bg-muted text-muted-foreground border-border";
  if (char === "M") {
    colorClass = "bg-amber-500/15 text-amber-600 dark:text-amber-400 border-amber-500/20";
  } else if (char === "A") {
    colorClass = "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/20";
  } else if (char === "D") {
    colorClass = "bg-rose-500/15 text-rose-600 dark:text-rose-400 border-rose-500/20";
  } else if (char === "?" || char === "U") {
    colorClass = "bg-sky-500/15 text-sky-600 dark:text-sky-400 border-sky-500/20";
  } else if (char === "R") {
    colorClass = "bg-purple-500/15 text-purple-600 dark:text-purple-400 border-purple-500/20";
  }
  return (
    <span
      className={cn(
        "inline-flex size-4 shrink-0 items-center justify-center rounded text-[10px] font-mono font-semibold border",
        colorClass,
      )}
    >
      {char}
    </span>
  );
}

function FileGroup({
  title,
  entries,
  selected,
  isStagedGroup,
  activeDiffPath,
  onToggle,
  onToggleAll,
  onOpen,
  onStageSingle,
  onUnstageSingle,
  onDiscardSingle,
  onActionAll,
  actionAllTooltip,
}: {
  title: string;
  entries: GitStatusEntry[];
  selected: Set<string>;
  isStagedGroup?: boolean;
  activeDiffPath?: string;
  onToggle: (path: string) => void;
  onToggleAll: () => void;
  onOpen: (entry: GitStatusEntry) => void;
  onStageSingle?: (path: string) => void;
  onUnstageSingle?: (path: string) => void;
  onDiscardSingle?: (path: string) => void;
  onActionAll?: () => void;
  actionAllTooltip?: string;
}) {
  const [open, setOpen] = useState(true);
  if (entries.length === 0) return null;

  const allSelected = entries.every((entry) => selected.has(entry.path));

  return (
    <div className="rounded-xl border border-border/70 bg-card/30 overflow-hidden shadow-2xs">
      {/* Group Header */}
      <div className="flex items-center justify-between bg-muted/40 px-2 py-1.5 text-xs">
        <div className="flex items-center gap-1.5 min-w-0">
          <button
            type="button"
            onClick={() => setOpen((prev) => !prev)}
            className="flex items-center gap-1 text-muted-foreground hover:text-foreground transition-colors"
          >
            <ChevronRight
              className={cn("size-3.5 transition-transform duration-150", open && "rotate-90")}
            />
          </button>
          <input
            type="checkbox"
            checked={allSelected}
            onChange={onToggleAll}
            className="size-3.5 cursor-pointer rounded border-muted-foreground/30 accent-primary"
            title={title}
          />
          <button
            type="button"
            onClick={() => setOpen((prev) => !prev)}
            className="flex items-center gap-1.5 font-medium text-foreground text-left"
          >
            <span>{title}</span>
            <span className="rounded-full bg-muted px-1.5 py-0.2 text-[10px] font-medium text-muted-foreground">
              {entries.length}
            </span>
          </button>
        </div>

        {onActionAll ? (
          <Button
            size="icon-xs"
            variant="ghost"
            className="size-5 text-muted-foreground hover:text-foreground"
            onClick={onActionAll}
            title={actionAllTooltip}
          >
            {isStagedGroup ? <Minus className="size-3" /> : <Plus className="size-3" />}
          </Button>
        ) : null}
      </div>

      {/* Group Content */}
      {open ? (
        <ul className="divide-y divide-border/40 p-0.5">
          {entries.map((entry) => {
            const isChecked = selected.has(entry.path);
            const isActive = activeDiffPath === entry.path;
            const lastSlash = entry.path.lastIndexOf("/");
            const fileName = lastSlash >= 0 ? entry.path.slice(lastSlash + 1) : entry.path;
            const dirPath = lastSlash >= 0 ? entry.path.slice(0, lastSlash) : "";

            return (
              <li
                key={entry.path}
                className={cn(
                  "group flex items-center gap-2 rounded-lg px-2 py-1 transition-colors text-xs hover:bg-accent/50",
                  isActive && "bg-accent text-accent-foreground font-medium",
                )}
              >
                <input
                  type="checkbox"
                  checked={isChecked}
                  onChange={() => onToggle(entry.path)}
                  className="size-3.5 cursor-pointer rounded border-muted-foreground/30 accent-primary shrink-0"
                />
                <StatusBadge xy={entry.xy} />
                <button
                  type="button"
                  className="flex min-w-0 flex-1 items-baseline gap-1 text-left truncate"
                  onClick={() => onOpen(entry)}
                >
                  <span className="truncate text-xs text-foreground">{fileName}</span>
                  {dirPath ? (
                    <span className="truncate font-mono text-[10px] text-muted-foreground/60">
                      {dirPath}
                    </span>
                  ) : null}
                </button>

                {/* Quick Row Actions on hover */}
                <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                  {isStagedGroup ? (
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      className="size-5 text-muted-foreground hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        onUnstageSingle?.(entry.path);
                      }}
                      title="Unstage"
                    >
                      <Minus className="size-3" />
                    </Button>
                  ) : (
                    <>
                      <Button
                        size="icon-xs"
                        variant="ghost"
                        className="size-5 text-muted-foreground hover:text-foreground"
                        onClick={(e) => {
                          e.stopPropagation();
                          onStageSingle?.(entry.path);
                        }}
                        title="Stage"
                      >
                        <Plus className="size-3" />
                      </Button>
                      <Button
                        size="icon-xs"
                        variant="ghost"
                        className="size-5 text-muted-foreground hover:text-destructive"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDiscardSingle?.(entry.path);
                        }}
                        title="Discard"
                      >
                        <RotateCcw className="size-3" />
                      </Button>
                    </>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
