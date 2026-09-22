import type { FileRollbackMode } from "@/lib/types";

export function fileRollbackChoices(preview: {
  available: boolean;
  conflicts: string[];
}): Record<FileRollbackMode, boolean> {
  const files = preview.available && preview.conflicts.length === 0;
  return {
    conversation: true,
    files,
    both: files,
  };
}

export function summarizeRollbackPaths(label: string, paths: string[], limit = 6): string {
  if (paths.length === 0) return "";
  const shown = paths.slice(0, limit).join("、");
  const extra = paths.length > limit ? ` 等 ${paths.length} 个` : "";
  return `${label}：${shown}${extra}`;
}
