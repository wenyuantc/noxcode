import { useTranslation } from "react-i18next";

import { formatRelativeTime } from "@/lib/utils";
import type { GitCheckpoint } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function CheckpointTimeline({
  checkpoints,
  onRestore,
}: {
  checkpoints: GitCheckpoint[];
  onRestore: (checkpoint: GitCheckpoint) => void;
}) {
  const { t, i18n } = useTranslation("git");
  if (checkpoints.length === 0) {
    return <p className="px-3 py-2 text-xs text-muted-foreground">{t("empty")}</p>;
  }
  return (
    <ol className="space-y-2">
      {checkpoints.map((checkpoint) => (
        <li key={checkpoint.id} className="rounded-md border px-3 py-2">
          <div className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">
                #{checkpoint.seq} {checkpoint.label || checkpoint.kind}
              </p>
              <p className="truncate font-mono text-[11px] text-muted-foreground">
                {checkpoint.commit_oid.slice(0, 12)}
              </p>
              <p className="text-[11px] text-muted-foreground">
                {formatRelativeTime(checkpoint.created_at, i18n.language)}
              </p>
            </div>
            {checkpoint.ref_valid ? (
              <Button size="sm" variant="outline" onClick={() => onRestore(checkpoint)}>
                {t("restore")}
              </Button>
            ) : (
              <Badge variant="destructive">{t("invalid")}</Badge>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
