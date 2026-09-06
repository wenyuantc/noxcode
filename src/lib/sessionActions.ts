import type { AgentSession, NativeBackgroundTask, NativeInputQueue, Workspace } from "@/lib/types";

export function resolveSessionDirectory(session: AgentSession, workspaces: Workspace[]) {
  const workspace = workspaces.find((item) => item.id === session.workspace_id);
  const remote = session.execution_target !== "local";
  const fallback = remote
    ? workspace?.remote_repo_path
    : workspace?.workspace_type === "local"
      ? workspace.repo_path
      : null;
  const path = session.working_dir?.trim() ? session.working_dir : fallback;
  return { path: path?.trim() ? path : null, remote };
}

interface SessionActivity {
  turnState: Record<string, string>;
  liveBySession: Record<string, unknown>;
  backgroundBySession: Record<string, NativeBackgroundTask[]>;
  inputQueueBySession: Record<string, NativeInputQueue>;
  permissions: Record<string, Record<string, unknown>>;
  planQuestions: Record<string, Record<string, unknown>>;
  planApprovals: Record<string, Record<string, unknown>>;
}

export function isSessionBusy(sessionId: string, state: SessionActivity): boolean {
  return Boolean(
    state.turnState[sessionId] === "working" ||
    (state.liveBySession[sessionId] && !state.turnState[sessionId]) ||
    state.backgroundBySession[sessionId]?.some(
      (task) => task.status === "queued" || task.status === "running",
    ) ||
    state.inputQueueBySession[sessionId]?.items.length ||
    Object.keys(state.permissions[sessionId] ?? {}).length ||
    Object.keys(state.planQuestions[sessionId] ?? {}).length ||
    Object.keys(state.planApprovals[sessionId] ?? {}).length,
  );
}

export function mergeSessions(current: AgentSession[], incoming: AgentSession[]): AgentSession[] {
  const sessions = new Map(current.map((session) => [session.id, session]));
  for (const session of incoming) sessions.set(session.id, session);
  return [...sessions.values()];
}
