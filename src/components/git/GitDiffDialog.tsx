import { FileDiff, Loader2, RefreshCw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { getGitFileDiff } from "@/lib/backend";
import type { GitFileDiff } from "@/lib/types";
import { DiffView } from "./DiffView";

export interface GitDiffTarget {
  workspaceId: string;
  path: string;
  scope: "worktree" | "staged";
  oldPath?: string;
}

export function GitDiffDialog({
  target,
  onClose,
}: {
  target: GitDiffTarget | null;
  onClose: () => void;
}) {
  const { t } = useTranslation("git");
  return (
    <Dialog
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        showCloseButton={false}
        className="flex h-[min(85dvh,calc(100dvh-2rem))] w-[92vw] max-w-[min(1440px,calc(100vw-2rem))] min-h-0 flex-col gap-0 overflow-hidden rounded-lg p-0 sm:max-w-[min(1440px,calc(100vw-2rem))]"
      >
        <div className="max-h-36 shrink-0 overflow-auto border-b px-4 py-3 pr-12">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <FileDiff className="size-4 shrink-0 text-muted-foreground" />
            {t("diffTitle")}
            <span className="text-xs font-normal text-muted-foreground">
              {t(target?.scope === "staged" ? "staged" : "worktree")}
            </span>
          </DialogTitle>
          <DialogDescription className="mt-2 break-all font-mono text-xs">
            {target?.oldPath ? `${target.oldPath} → ` : ""}
            {target?.path}
          </DialogDescription>
        </div>
        <DialogClose
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              className="absolute right-2 top-2"
              title={t("close")}
              aria-label={t("close")}
            />
          }
        >
          <X className="size-4" />
        </DialogClose>
        {target ? <DiffContent key={JSON.stringify(target)} target={target} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function DiffContent({ target }: { target: GitDiffTarget }) {
  const { t } = useTranslation("git");
  const [diff, setDiff] = useState<GitFileDiff | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const { workspaceId, path, scope, oldPath } = target;

  useEffect(() => {
    let cancelled = false;
    setDiff(null);
    setError(null);
    void getGitFileDiff(workspaceId, path, scope, oldPath).then(
      (next) => {
        if (!cancelled) setDiff(next);
      },
      (reason) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, path, scope, oldPath, attempt]);

  if (error !== null) {
    return (
      <div
        role="alert"
        className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 overflow-auto p-6"
      >
        <p className="max-w-full whitespace-pre-wrap break-words text-sm text-destructive">
          {error}
        </p>
        <Button variant="outline" size="sm" onClick={() => setAttempt((value) => value + 1)}>
          <RefreshCw className="size-3.5" />
          {t("retry")}
        </Button>
      </div>
    );
  }
  if (!diff) {
    return (
      <div
        role="status"
        className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground"
      >
        <Loader2 className="size-4 animate-spin" />
        {t("loadingDiff")}
      </div>
    );
  }
  if (!diff.is_binary && !diff.patch.trim()) {
    return (
      <div
        role="status"
        className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground"
      >
        {t("emptyDiff")}
      </div>
    );
  }
  return <DiffView diff={diff} className="flex-1" />;
}
