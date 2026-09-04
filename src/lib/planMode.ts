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

/** 计划模式是叠在权限模式上的开关：关掉后回到此前的 default/edit/build/yolo。 */
export function applyComposerPlanMode(options: {
  enabled: boolean;
  sessionId: string | null | undefined;
  setDefault: (value: boolean) => void;
  setSession: (sessionId: string, enabled: boolean) => void;
}): void {
  options.setDefault(options.enabled);
  if (options.sessionId) options.setSession(options.sessionId, options.enabled);
}
