function normalizePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

export function isManagedWorktreePath(
  path: string | null | undefined,
  sessionId: string | null | undefined,
  configuredRoot?: string | null,
): boolean {
  const trimmed = normalizePath(path ?? "");
  const id = sessionId?.trim() ?? "";
  if (!id || !trimmed) return false;
  if (
    trimmed.includes("worktrees") &&
    (trimmed.endsWith(id) || trimmed.includes(`worktrees/${id}`))
  ) {
    return true;
  }
  const root = configuredRoot?.trim() ? normalizePath(configuredRoot) : "";
  return root.length > 0 && trimmed === `${root}/${id}`;
}

export function relativeWorktreeFilePath(
  path: string,
  sessionId?: string | null,
  configuredRoot?: string | null,
): string {
  const normalized = path.replace(/\\/g, "/");
  const id = sessionId?.trim() ?? "";
  if (id) {
    const marker = `/worktrees/${id}/`;
    const index = normalized.indexOf(marker);
    if (index >= 0) return normalized.slice(index + marker.length);
    const root = configuredRoot?.trim() ? normalizePath(configuredRoot) : "";
    if (root) {
      const prefix = `${root}/${id}/`;
      if (normalized.startsWith(prefix)) return normalized.slice(prefix.length);
    }
  }
  const match = normalized.match(/\/worktrees\/[^/]+\/(.+)$/);
  return match?.[1] ?? path;
}
