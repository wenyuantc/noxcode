import { FileDiff, FileText, Loader2, RefreshCw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/code/CodeBlock";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { getGitFileDiff, getGitFilePreview } from "@/lib/backend";
import { languageFromPath } from "@/lib/codeLanguage";
import type { GitFilePreview } from "@/lib/types";
import { DiffView } from "./DiffView";

export interface GitDiffTarget {
  workspaceId: string;
  path: string;
  scope: "auto" | "worktree" | "staged";
  oldPath?: string;
}

export function GitDiffDialog({
  target,
  onClose,
}: {
  target: GitDiffTarget | null;
  onClose: () => void;
}) {
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
        {target ? <FilePreviewPanel key={JSON.stringify(target)} target={target} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function FilePreviewPanel({ target }: { target: GitDiffTarget }) {
  const { t } = useTranslation("git");
  const [preview, setPreview] = useState<GitFilePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const { workspaceId, path, scope, oldPath } = target;

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setError(null);
    const request: Promise<GitFilePreview> =
      scope === "auto"
        ? getGitFilePreview(workspaceId, path)
        : getGitFileDiff(workspaceId, path, scope, oldPath).then((diff) => ({
            kind: "diff",
            scope,
            diff,
          }));
    void request.then(
      (next) => {
        if (!cancelled) setPreview(next);
      },
      (reason) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, path, scope, oldPath, attempt]);

  const showDiff = preview?.kind === "diff" || (!preview && scope !== "auto");
  const resolvedScope = preview?.kind === "diff" ? preview.scope : scope;
  return (
    <>
      <div className="max-h-36 shrink-0 overflow-auto border-b px-4 py-3 pr-12">
        <DialogTitle className="flex flex-wrap items-center gap-2 text-sm">
          {showDiff ? (
            <FileDiff className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <FileText className="size-4 shrink-0 text-muted-foreground" />
          )}
          {t(showDiff ? "diffTitle" : "filePreviewTitle")}
          {showDiff || preview?.kind === "content" ? (
            <span className="text-xs font-normal text-muted-foreground">
              {t(
                showDiff ? (resolvedScope === "staged" ? "staged" : "worktree") : "currentContent",
              )}
            </span>
          ) : null}
        </DialogTitle>
        <DialogDescription className="mt-2 break-all font-mono text-xs">
          {oldPath ? `${oldPath} → ` : ""}
          {path}
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
      {error !== null ? (
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
      ) : !preview ? (
        <div
          role="status"
          className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground"
        >
          <Loader2 className="size-4 animate-spin" />
          {t(scope === "auto" ? "loadingPreview" : "loadingDiff")}
        </div>
      ) : (
        <GitFilePreviewBody preview={preview} />
      )}
    </>
  );
}

export function GitFilePreviewBody({ preview }: { preview: GitFilePreview }) {
  const { t } = useTranslation("git");
  if (preview.kind === "diff") {
    if (preview.diff.is_binary || preview.diff.patch.trim()) {
      return <DiffView diff={preview.diff} className="flex-1" />;
    }
  }
  if (preview.kind !== "content") {
    return (
      <div
        role="status"
        className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground"
      >
        {t(preview.kind === "missing" ? "fileMissing" : "emptyDiff")}
      </div>
    );
  }
  return (
    <>
      <div
        role="status"
        className="max-h-32 shrink-0 overflow-auto border-b bg-muted/30 px-4 py-2 text-xs text-muted-foreground"
      >
        {t(`previewReason.${preview.reason}`)}
        {preview.truncated ? (
          <p className="mt-1 text-amber-700 dark:text-amber-300">{t("contentTruncated")}</p>
        ) : null}
      </div>
      {preview.is_binary || preview.content.length === 0 ? (
        <div
          role="status"
          className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground"
        >
          {t(preview.is_binary ? "binary" : "fileEmpty")}
        </div>
      ) : (
        <CodeBlock
          code={preview.content}
          language={languageFromPath(preview.path)}
          className="min-h-0 min-w-0 flex-1 rounded-none border-0"
        />
      )}
    </>
  );
}
