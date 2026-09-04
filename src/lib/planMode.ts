export function resolveComposerPlanMode(
  sessionId: string | null | undefined,
  sessionModes: Record<string, boolean>,
  defaultMode: boolean,
): boolean {
  if (sessionId && Object.prototype.hasOwnProperty.call(sessionModes, sessionId)) {
    return sessionModes[sessionId] === true;
  }
  return defaultMode;
}
