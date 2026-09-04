import { GitCommit, History, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { GitCheckpoint } from "@/lib/types";
import { formatRelativeTime } from "@/lib/utils";

export function CheckpointTimeline({
  checkpoints,
  onRestore,
}: {
  checkpoints: GitCheckpoint[];
  onRestore: (checkpoint: GitCheckpoint) => void;
}) {
  const { t, i18n } = useTranslation("git");
  if (checkpoints.length === 0) {
    return (
      <div className="my-4 flex flex-col items-center justify-center rounded-xl border border-dashed border-border/80 p-8 text-center">
        <History className="mb-2 size-7 stroke-[1.5] text-muted-foreground/50" />
        <p className="text-xs font-medium text-muted-foreground">{t("empty")}</p>
      </div>
    );
  }
  return (
    <ol className="space-y-2">
      {checkpoints.map((checkpoint) => (
        <li
          key={checkpoint.id}
          className="group rounded-xl border border-border/70 bg-card/60 p-3 shadow-2xs transition-colors hover:border-border"
        >
          <div className="flex items-start gap-2.5">
            <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-xs font-semibold text-primary">
              #{checkpoint.seq}
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-xs font-medium text-foreground">
                {checkpoint.label || checkpoint.kind}
              </p>
              <div className="mt-1 flex items-center gap-2 text-[10px] text-muted-foreground">
                <span className="flex items-center gap-0.5 font-mono">
                  <GitCommit className="size-2.5 opacity-60" />
                  {checkpoint.commit_oid.slice(0, 8)}
                </span>
                <span>•</span>
                <span>{formatRelativeTime(checkpoint.created_at, i18n.language)}</span>
              </div>
            </div>
            {checkpoint.ref_valid ? (
              <Button
                size="xs"
                variant="outline"
                className="h-6.5 gap-1 text-[11px]"
                onClick={() => onRestore(checkpoint)}
              >
                <RotateCcw className="size-3" />
                {t("restore")}
              </Button>
            ) : (
              <Badge variant="destructive" className="text-[10px] px-1.5 py-0">
                {t("invalid")}
              </Badge>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
