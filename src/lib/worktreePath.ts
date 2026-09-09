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
