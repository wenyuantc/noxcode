import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { mergeSessionWorktree, resolveSessionWorktreeMerge } from "@/lib/backend";
import type { MergeWorktreeResult } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";

export function MergeWorktreeDialog() {
  const { t } = useTranslation(["git", "common"]);
  const prompt = useSessionStore((state) => state.worktreeMergePrompt);
  const close = useSessionStore((state) => state.closeWorktreeMergePrompt);
  const openPrompt = useSessionStore((state) => state.openWorktreeMergePrompt);
  const markMerged = useSessionStore((state) => state.markWorktreeMerged);
  const setGitOpen = useUiStore((state) => state.setGitOpen);
  const [branchName, setBranchName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const applyResult = (result: MergeWorktreeResult) => {
    if (!prompt) return;
    if (result.status === "conflicted" || result.status === "partial") {
      openPrompt({
        ...prompt,
        phase: "conflict",
        conflicts: result.conflicts,
        branch: result.branch,
        message: result.message,
      });
      return;
    }
    if (
      result.status === "merged" ||
      result.status === "resolved" ||
      result.status === "branched"
    ) {
      markMerged(prompt.sessionId);
      return;
    }
    close();
  };

  const run = async (operation: () => Promise<MergeWorktreeResult>) => {
    if (!prompt || busy) return;
    setBusy(true);
    setError(null);
    try {
      applyResult(await operation());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  if (!prompt) return null;

  const conflicted = prompt.phase === "conflict";

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) close();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {conflicted ? t("git:mergeWorktreeConflictsTitle") : t("git:mergeWorktreeTitle")}
          </DialogTitle>
          <DialogDescription>
            {prompt.message ||
              (conflicted ? t("git:mergeWorktreeConflictsHint") : t("git:mergeWorktreeHint"))}
          </DialogDescription>
        </DialogHeader>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        {conflicted ? (
          <>
            <ul className="max-h-40 overflow-auto font-mono text-[11px] text-muted-foreground">
              {prompt.conflicts.map((path) => (
                <li key={path}>{path}</li>
              ))}
            </ul>
            <DialogFooter className="flex-col gap-2 sm:flex-col">
              <Button
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    resolveSessionWorktreeMerge(prompt.workspaceId, prompt.sessionId, "ai"),
                  )
                }
              >
                {t("git:mergeWorktreeAi")}
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => {
                  close();
                  setGitOpen(true);
                }}
              >
                {t("git:mergeWorktreeManual")}
              </Button>
              <Button
                variant="destructive"
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    resolveSessionWorktreeMerge(prompt.workspaceId, prompt.sessionId, "abort"),
                  )
                }
              >
                {t("git:mergeWorktreeAbort")}
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <Input
              value={branchName}
              onChange={(event) => setBranchName(event.target.value)}
              placeholder={t("git:mergeWorktreeBranchName")}
              className="h-8"
            />
            <DialogFooter className="flex-col gap-2 sm:flex-col">
              <Button
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    mergeSessionWorktree(
                      prompt.workspaceId,
                      prompt.sessionId,
                      "merge_current",
                      branchName.trim() || null,
                    ),
                  )
                }
              >
                {t("git:mergeWorktreeCurrent")}
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    mergeSessionWorktree(
                      prompt.workspaceId,
                      prompt.sessionId,
                      "create_branch",
                      branchName.trim() || null,
                    ),
                  )
                }
              >
                {t("git:mergeWorktreeBranch")}
              </Button>
              <Button
                variant="ghost"
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    mergeSessionWorktree(prompt.workspaceId, prompt.sessionId, "keep", null),
                  )
                }
              >
                {t("git:mergeWorktreeKeep")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
