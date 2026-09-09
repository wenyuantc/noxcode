import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Loader2, Sparkles } from "lucide-react";

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
import { Textarea } from "@/components/ui/textarea";
import { useDismissible } from "@/hooks/useDismissible";
import {
  generateGitCommitMessage,
  listGitBranches,
  mergeSessionWorktree,
  resolveSessionWorktreeMerge,
} from "@/lib/backend";
import type { MergeWorktreeResult } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";

function BranchNameField({
  workspaceId,
  value,
  onChange,
  placeholder,
  disabled,
}: {
  workspaceId: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation("git");
  const [open, setOpen] = useState(false);
  const [names, setNames] = useState<string[]>([]);
  const rootRef = useRef<HTMLDivElement>(null);
  useDismissible(open, () => setOpen(false), rootRef);

  useEffect(() => {
    let cancelled = false;
    void listGitBranches(workspaceId)
      .then((branches) => {
        if (!cancelled) setNames(branches.map((item) => item.name));
      })
      .catch(() => {
        if (!cancelled) setNames([]);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  const filtered = useMemo(() => {
    const query = value.trim().toLowerCase();
    return names.filter((name) => !query || name.toLowerCase().includes(query));
  }, [names, value]);

  return (
    <div ref={rootRef} className="relative">
      <div className="flex items-center gap-1">
        <Input
          value={value}
          disabled={disabled}
          onChange={(event) => {
            onChange(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          placeholder={placeholder}
          className="h-8"
          role="combobox"
          aria-expanded={open}
          aria-autocomplete="list"
        />
        <Button
          type="button"
          size="icon-sm"
          variant="outline"
          disabled={disabled}
          className="h-8 w-8 shrink-0"
          aria-label={t("searchBranch")}
          onClick={() => setOpen((current) => !current)}
        >
          <ChevronDown className="size-3.5" />
        </Button>
      </div>
      {open && filtered.length > 0 ? (
        <ul
          role="listbox"
          className="absolute z-20 mt-1 max-h-40 w-full overflow-auto rounded-md border bg-popover p-1 text-xs shadow-md"
        >
          {filtered.map((name) => (
            <li key={name}>
              <button
                type="button"
                role="option"
                aria-selected={name === value}
                className={cn(
                  "flex w-full cursor-pointer rounded-sm px-2 py-1.5 text-left hover:bg-accent",
                  name === value && "bg-accent",
                )}
                onClick={() => {
                  onChange(name);
                  setOpen(false);
                }}
              >
                {name}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

export function MergeWorktreeDialog() {
  const { t } = useTranslation(["git", "common"]);
  const prompt = useSessionStore((state) => state.worktreeMergePrompt);
  const close = useSessionStore((state) => state.closeWorktreeMergePrompt);
  const openPrompt = useSessionStore((state) => state.openWorktreeMergePrompt);
  const markMerged = useSessionStore((state) => state.markWorktreeMerged);
  const setGitOpen = useUiStore((state) => state.setGitOpen);
  const [branchName, setBranchName] = useState("");
  const [commitMessage, setCommitMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [generatingCommit, setGeneratingCommit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [generateError, setGenerateError] = useState<string | null>(null);
  const commitAiEnabled = useSettingsStore((state) => state.ai?.commit_message.enabled) === true;

  useEffect(() => {
    if (useSettingsStore.getState().ai) return;
    void useSettingsStore.getState().load();
  }, []);

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
        {generateError ? (
          <p className="text-sm text-destructive">
            {t("git:generateCommitFailed")} {generateError}
          </p>
        ) : null}
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
            <div className="space-y-1.5">
              <div className="relative">
                <Textarea
                  value={commitMessage}
                  placeholder={t("git:commitMessage")}
                  className="h-20 min-h-20 max-h-20 w-full resize-none pr-10 text-xs"
                  disabled={busy || generatingCommit}
                  onChange={(event) => setCommitMessage(event.target.value)}
                />
                {commitAiEnabled ? (
                  <Button
                    type="button"
                    size="icon-sm"
                    variant="ghost"
                    className="absolute top-1.5 right-1.5 size-7 rounded-md bg-foreground text-background hover:bg-foreground/90 hover:text-background"
                    title={t(generatingCommit ? "git:generatingCommit" : "git:generateCommit")}
                    aria-label={t(generatingCommit ? "git:generatingCommit" : "git:generateCommit")}
                    disabled={busy || generatingCommit}
                    onClick={() => {
                      if (busy || generatingCommit) return;
                      setGeneratingCommit(true);
                      setGenerateError(null);
                      void generateGitCommitMessage(prompt.workspaceId, prompt.sessionId)
                        .then((next) => setCommitMessage(next))
                        .catch((reason: unknown) => setGenerateError(String(reason)))
                        .finally(() => setGeneratingCommit(false));
                    }}
                  >
                    {generatingCommit ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : (
                      <Sparkles className="size-3.5" />
                    )}
                  </Button>
                ) : null}
              </div>
              <p className="text-[11px] text-muted-foreground">
                {t("git:mergeWorktreeCommitHint")}
              </p>
            </div>
            <div className="space-y-1.5">
              <BranchNameField
                workspaceId={prompt.workspaceId}
                value={branchName}
                onChange={setBranchName}
                placeholder={t("git:mergeWorktreeBranchName")}
                disabled={busy}
              />
              <p className="text-[11px] text-muted-foreground">
                {t("git:mergeWorktreeBranchNameHint")}
              </p>
            </div>
            <DialogFooter className="flex-col gap-2 sm:flex-col">
              <Button
                disabled={busy || generatingCommit || !commitMessage.trim()}
                onClick={() =>
                  void run(() =>
                    mergeSessionWorktree(
                      prompt.workspaceId,
                      prompt.sessionId,
                      "merge_current",
                      null,
                      commitMessage.trim(),
                    ),
                  )
                }
              >
                {t("git:mergeWorktreeCurrent")}
              </Button>
              <Button
                variant="outline"
                disabled={busy || generatingCommit || !commitMessage.trim()}
                onClick={() =>
                  void run(() =>
                    mergeSessionWorktree(
                      prompt.workspaceId,
                      prompt.sessionId,
                      "create_branch",
                      branchName.trim() || null,
                      commitMessage.trim(),
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
