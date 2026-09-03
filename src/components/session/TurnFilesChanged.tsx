import { File } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { getGitNumstat } from "@/lib/backend";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export function TurnFilesChanged({ paths }: { paths: string[] }) {
  const { t } = useTranslation("sessions");
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const openGitPreview = useUiStore((state) => state.openGitPreview);
  const [stats, setStats] = useState<
    Record<string, { added: number | null; deleted: number | null }>
  >({});

  const pathKey = paths.join("\n");
  useEffect(() => {
    const listed = pathKey ? pathKey.split("\n") : [];
    if (!workspaceId || listed.length === 0) {
      setStats({});
      return;
    }
    let cancelled = false;
    void getGitNumstat(workspaceId, "worktree")
      .then((entries) => {
        if (cancelled) return;
        const wanted = new Set(listed);
        const next: Record<string, { added: number | null; deleted: number | null }> = {};
        for (const entry of entries) {
          if (!wanted.has(entry.path)) continue;
          next[entry.path] = { added: entry.added, deleted: entry.deleted };
        }
        setStats(next);
      })
      .catch(() => {
        if (!cancelled) setStats({});
      });
    return () => {
      cancelled = true;
    };
  }, [pathKey, workspaceId]);

  if (paths.length === 0) return null;

  return (
    <div className="rounded-lg border">
      <div className="flex items-center gap-2 border-b px-3 py-2 text-sm">
        <span className="flex-1">{t("filesChanged", { count: paths.length })}</span>
        <button
          type="button"
          className="text-xs text-muted-foreground hover:text-foreground"
          onClick={() => openGitPreview(paths[0] ?? null)}
        >
          {t("review")}
        </button>
      </div>
      <ul className="py-1">
        {paths.map((path) => {
          const stat = stats[path];
          return (
            <li key={path}>
              <button
                type="button"
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted/50"
                onClick={() => openGitPreview(path)}
              >
                <File className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{path.split("/").pop() ?? path}</span>
                {stat?.added != null ? (
                  <span className="font-mono text-xs text-emerald-600 dark:text-emerald-400">
                    +{stat.added}
                  </span>
                ) : null}
                {stat?.deleted != null ? (
                  <span className="font-mono text-xs text-rose-600 dark:text-rose-400">
                    -{stat.deleted}
                  </span>
                ) : null}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
