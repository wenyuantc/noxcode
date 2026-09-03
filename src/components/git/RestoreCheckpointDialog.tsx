import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { GitCheckpoint, GitRestorePreview } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const FOLD_AT = 20;

export function RestoreCheckpointDialog({
  open,
  checkpoint,
  preview,
  busy,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  checkpoint: GitCheckpoint | null;
  preview: GitRestorePreview | null;
  busy?: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (deleteNewPaths: string[]) => void;
}) {
  const { t } = useTranslation(["git", "common"]);
  const [deleteNew, setDeleteNew] = useState(false);

  useEffect(() => {
    if (open) setDeleteNew(false);
  }, [open, checkpoint?.id]);

  const deleteCandidates = preview?.wont_be_touched ?? [];
  const blocked = Boolean(preview?.blocked_reason);

  const title = useMemo(() => {
    if (!checkpoint) return t("git:restore");
    return t("git:restoreTitle", {
      seq: checkpoint.seq,
      label: checkpoint.label || checkpoint.kind,
    });
  }, [checkpoint, t]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{t("git:preRestore")}</DialogDescription>
        </DialogHeader>
        {preview?.blocked_reason ? (
          <p className="text-sm text-destructive">
            {t("git:blocked", { reason: preview.blocked_reason })}
          </p>
        ) : null}
        {preview?.warnings.map((warning) => (
          <p key={warning} className="text-xs text-amber-600">
            {warning}
          </p>
        ))}
        <ImpactList
          title={t("git:willOverwrite", { count: preview?.will_overwrite.length ?? 0 })}
          paths={preview?.will_overwrite ?? []}
        />
        <ImpactList
          title={t("git:willRecreate", { count: preview?.will_recreate.length ?? 0 })}
          paths={preview?.will_recreate ?? []}
        />
        <ImpactList title={t("git:wontTouch")} paths={deleteCandidates} />
        {deleteCandidates.length > 0 ? (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={deleteNew}
              onChange={(event) => setDeleteNew(event.target.checked)}
            />
            {t("git:deleteNew")}
          </label>
        ) : null}
        <DialogFooter>
          <Button variant="outline" autoFocus onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            variant="destructive"
            disabled={busy || blocked || !preview}
            onClick={() => onConfirm(deleteNew ? deleteCandidates : [])}
          >
            {t("git:confirmRestore")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ImpactList({ title, paths }: { title: string; paths: string[] }) {
  const [expanded, setExpanded] = useState(false);
  const visible = expanded || paths.length <= FOLD_AT ? paths : paths.slice(0, FOLD_AT);
  return (
    <div>
      <p className="text-sm font-medium">{title}</p>
      {visible.length > 0 ? (
        <ul className="mt-1 max-h-32 overflow-auto font-mono text-[11px] text-muted-foreground">
          {visible.map((path) => (
            <li key={path}>{path}</li>
          ))}
        </ul>
      ) : null}
      {paths.length > FOLD_AT && !expanded ? (
        <button type="button" className="mt-1 text-xs underline" onClick={() => setExpanded(true)}>
          +{paths.length - FOLD_AT}
        </button>
      ) : null}
    </div>
  );
}
