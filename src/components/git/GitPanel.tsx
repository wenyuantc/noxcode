import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  commitGitChanges,
  getGitFileDiff,
  getGitStatus,
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
  GitCheckpoint,
  GitFileDiff,
  GitRestorePreview,
  GitStatus,
  GitStatusEntry,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { CheckpointTimeline } from "./CheckpointTimeline";
import { DiffView } from "./DiffView";
import { RestoreCheckpointDialog } from "./RestoreCheckpointDialog";

export function GitPanel() {
  const { t } = useTranslation("git");
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const gitFocusPath = useUiStore((state) => state.gitFocusPath);
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [checkpoints, setCheckpoints] = useState<GitCheckpoint[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [diff, setDiff] = useState<GitFileDiff | null>(null);
  const [message, setMessage] = useState("");
  const [restoreTarget, setRestoreTarget] = useState<GitCheckpoint | null>(null);
  const [preview, setPreview] = useState<GitRestorePreview | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    if (!workspaceId) return;
    const next = await getGitStatus(workspaceId);
    setStatus(next);
    if (sessionId) {
      setCheckpoints(await listGitCheckpoints(workspaceId, sessionId));
    }
  }, [sessionId, workspaceId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    if (!workspaceId || !gitFocusPath) return;
    void getGitFileDiff(workspaceId, gitFocusPath, "worktree").then(setDiff);
  }, [gitFocusPath, workspaceId]);

  const groups = groupGitStatus(status);
  const toggle = (path: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const showDiff = (entry: GitStatusEntry, scope: "worktree" | "staged") => {
    if (!workspaceId) return;
    void getGitFileDiff(workspaceId, entry.path, scope, entry.orig_path ?? undefined).then(setDiff);
  };

  const selectedOf = (entries: GitStatusEntry[]) =>
    entries.filter((entry) => selected.has(entry.path)).map((entry) => entry.path);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Tabs defaultValue="changes" className="flex min-h-0 flex-1 flex-col">
        <TabsList className="mx-3 mt-3">
          <TabsTrigger value="changes">{t("changes")}</TabsTrigger>
          <TabsTrigger value="checkpoints">{t("checkpoints")}</TabsTrigger>
        </TabsList>
        <TabsContent value="changes" className="min-h-0 overflow-auto px-3 pb-3">
          <FileGroup
            title={t("staged")}
            entries={groups.staged}
            selected={selected}
            onToggle={toggle}
            onOpen={(entry) => showDiff(entry, "staged")}
          />
          <FileGroup
            title={t("unstaged")}
            entries={groups.unstaged}
            selected={selected}
            onToggle={toggle}
            onOpen={(entry) => showDiff(entry, "worktree")}
          />
          <FileGroup
            title={t("untracked")}
            entries={groups.untracked}
            selected={selected}
            onToggle={toggle}
            onOpen={(entry) => showDiff(entry, "worktree")}
          />
          {groups.staged.length + groups.unstaged.length + groups.untracked.length === 0 ? (
            <p className="py-4 text-xs text-muted-foreground">{t("empty")}</p>
          ) : null}
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={!workspaceId}
              onClick={() => {
                const paths = selectedOf([...groups.unstaged, ...groups.untracked]);
                if (!workspaceId || paths.length === 0) return;
                void stageGitPaths(workspaceId, paths).then(reload);
              }}
            >
              {t("stage")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={!workspaceId}
              onClick={() => {
                const paths = selectedOf(groups.staged);
                if (!workspaceId || paths.length === 0) return;
                void unstageGitPaths(workspaceId, paths).then(reload);
              }}
            >
              {t("unstage")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={!workspaceId}
              onClick={() => {
                const paths = selectedOf([...groups.unstaged, ...groups.untracked]);
                if (!workspaceId || paths.length === 0) return;
                void restoreGitPaths(workspaceId, paths).then(reload);
              }}
            >
              {t("discard")}
            </Button>
          </div>
          <div className="mt-3 space-y-2">
            <Input
              value={message}
              placeholder={t("commitMessage")}
              onChange={(event) => setMessage(event.target.value)}
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                disabled={!workspaceId || !message.trim()}
                onClick={() => {
                  if (!workspaceId) return;
                  void commitGitChanges(workspaceId, message).then(() => {
                    setMessage("");
                    void reload();
                  });
                }}
              >
                {t("commit")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!workspaceId}
                onClick={() => {
                  if (!workspaceId) return;
                  void pushGitBranch(workspaceId);
                }}
              >
                {t("push")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!workspaceId}
                onClick={() => {
                  if (!workspaceId) return;
                  void pushGitBranch(workspaceId, undefined, undefined, true);
                }}
              >
                {t("setUpstream")}
              </Button>
            </div>
          </div>
          <div className="mt-3 rounded-md border">
            <DiffView diff={diff} />
          </div>
        </TabsContent>
        <TabsContent value="checkpoints" className="min-h-0 overflow-auto px-3 pb-3">
          <CheckpointTimeline
            checkpoints={checkpoints}
            onRestore={(checkpoint) => {
              if (!workspaceId) return;
              setRestoreTarget(checkpoint);
              void previewGitCheckpointRestore(workspaceId, checkpoint.id).then(setPreview);
            }}
          />
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

function FileGroup({
  title,
  entries,
  selected,
  onToggle,
  onOpen,
}: {
  title: string;
  entries: GitStatusEntry[];
  selected: Set<string>;
  onToggle: (path: string) => void;
  onOpen: (entry: GitStatusEntry) => void;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="mt-3">
      <p className="mb-1 text-[11px] font-medium text-muted-foreground">
        {title} ({entries.length})
      </p>
      <ul className="space-y-1">
        {entries.map((entry) => (
          <li key={entry.path} className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={selected.has(entry.path)}
              onChange={() => onToggle(entry.path)}
            />
            <button
              type="button"
              className="min-w-0 flex-1 truncate text-left hover:underline"
              onClick={() => onOpen(entry)}
            >
              {entry.path}
            </button>
            <span className="font-mono text-muted-foreground">{entry.xy}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
