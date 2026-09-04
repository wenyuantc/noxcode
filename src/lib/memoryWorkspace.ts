export function resolveMemoryWorkspaceId(
  current: string | null,
  activeWorkspaceId: string | null,
  workspaceIds: string[],
): string | null {
  if (current && workspaceIds.includes(current)) return current;
  if (activeWorkspaceId && workspaceIds.includes(activeWorkspaceId)) return activeWorkspaceId;
  return workspaceIds[0] ?? null;
}
