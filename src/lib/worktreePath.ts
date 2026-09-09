export function isManagedWorktreePath(
  path: string | null | undefined,
  sessionId: string | null | undefined,
): boolean {
  const trimmed = path?.trim() ?? "";
  const id = sessionId?.trim() ?? "";
  return (
    id.length > 0 &&
    trimmed.includes("worktrees") &&
    (trimmed.endsWith(id) || trimmed.includes(`worktrees/${id}`))
  );
}

export function relativeWorktreeFilePath(path: string, sessionId?: string | null): string {
  const normalized = path.replaceAll("\\", "/");
  const id = sessionId?.trim() ?? "";
  if (id) {
    const marker = `/worktrees/${id}/`;
    const index = normalized.indexOf(marker);
    if (index >= 0) return normalized.slice(index + marker.length);
  }
  const match = normalized.match(/\/worktrees\/[^/]+\/(.+)$/);
  return match?.[1] ?? path;
}
