import { create } from "zustand";

import { getAgentSessionLogLines } from "@/lib/backend";
import { resolveHistoricalUsage, resolveHistoryLimitTokens } from "@/lib/contextUsage";
import type { RawSessionLine } from "@/lib/sessionLines";
import type {
  AgentSessionExit,
  AgentSessionOutput,
  AgentSessionStarted,
  NativeContextUsage,
  NativePermissionRequest,
  NativePlanApprovalRequest,
  NativePlanQuestionRequest,
  NativeTextDelta,
} from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

function hydrateUsage(sessionId: string) {
  const current = useSessionStore.getState();
  if (current.usage[sessionId]) return;
  const workspace = useWorkspaceStore.getState();
  const channels = useChannelStore.getState();
  const session = workspace.sessions.find((item) => item.id === sessionId);
  const usage = resolveHistoricalUsage({
    sessionId,
    contextUsageJson: session?.context_usage_json,
    lines: current.lines[sessionId] ?? [],
    limitTokens: resolveHistoryLimitTokens(
      session,
      channels.channels,
      channels.activeChannelId,
      channels.activeModelId,
    ),
  });
  if (!usage) return;
  useSessionStore.setState({
    usage: { ...useSessionStore.getState().usage, [sessionId]: usage },
  });
}

interface SessionState {
  selectedSessionId: string | null;
  liveBySession: Record<string, AgentSessionStarted>;
  lines: Record<string, RawSessionLine[]>;
  turnState: Record<string, string>;
  usage: Record<string, NativeContextUsage>;
  stream: Record<string, { kind: string; text: string }>;
  permission: NativePermissionRequest | null;
  planQuestion: NativePlanQuestionRequest | null;
  planApproval: NativePlanApprovalRequest | null;
  selectSession: (id: string | null) => void;
  ensureHistory: (sessionId: string) => Promise<void>;
  loadHistory: (sessionId: string) => Promise<void>;
  onStarted: (session: AgentSessionStarted) => void;
  onStdout: (output: AgentSessionOutput) => void;
  onDelta: (delta: NativeTextDelta) => void;
  onUsage: (usage: NativeContextUsage) => void;
  onTurnState: (sessionId: string, state: string) => void;
  onExit: (exit: AgentSessionExit) => void;
  setPermission: (request: NativePermissionRequest | null) => void;
  setPlanQuestion: (request: NativePlanQuestionRequest | null) => void;
  setPlanApproval: (request: NativePlanApprovalRequest | null) => void;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  selectedSessionId: null,
  liveBySession: {},
  lines: {},
  turnState: {},
  usage: {},
  stream: {},
  permission: null,
  planQuestion: null,
  planApproval: null,
  selectSession: (id) => set({ selectedSessionId: id }),
  ensureHistory: async (sessionId) => {
    if (!get().lines[sessionId]) {
      const events = await getAgentSessionLogLines(sessionId);
      if (!get().lines[sessionId]) {
        set({
          lines: {
            ...get().lines,
            [sessionId]: events.map((event) => ({
              id: event.id,
              sessionId,
              text: event.message ?? "",
              createdAt: event.created_at,
            })),
          },
        });
      }
    }
    hydrateUsage(sessionId);
  },
  loadHistory: async (sessionId) => {
    set({ selectedSessionId: sessionId });
    const workspace = useWorkspaceStore.getState();
    const workspaceId = workspace.sessions.find(
      (session) => session.id === sessionId,
    )?.workspace_id;
    if (workspaceId && workspaceId !== workspace.activeWorkspaceId) {
      void workspace.setActive(workspaceId);
    }
    await get().ensureHistory(sessionId);
  },
  onStarted: (session) => {
    set({
      selectedSessionId: session.session_record_id,
      liveBySession: {
        ...get().liveBySession,
        [session.session_record_id]: session,
      },
      turnState: { ...get().turnState, [session.session_record_id]: "working" },
    });
  },
  onStdout: (output) => {
    const current = get().lines[output.session_record_id] ?? [];
    set({
      lines: {
        ...get().lines,
        [output.session_record_id]: [
          ...current,
          {
            id: output.session_event_id,
            sessionId: output.session_record_id,
            text: output.line,
            createdAt: new Date().toISOString(),
          },
        ],
      },
    });
  },
  onDelta: (delta) => {
    if (delta.clear) {
      set({
        stream: { ...get().stream, [delta.session_record_id]: { kind: delta.kind, text: "" } },
      });
      return;
    }
    const current = get().stream[delta.session_record_id] ?? { kind: delta.kind, text: "" };
    set({
      stream: {
        ...get().stream,
        [delta.session_record_id]: {
          kind: delta.kind,
          text: current.kind === delta.kind ? current.text + delta.text : delta.text,
        },
      },
    });
  },
  onUsage: (usage) => set({ usage: { ...get().usage, [usage.session_record_id]: usage } }),
  onTurnState: (sessionId, state) => set({ turnState: { ...get().turnState, [sessionId]: state } }),
  onExit: (exit) => {
    const liveBySession = { ...get().liveBySession };
    delete liveBySession[exit.session_record_id];
    const stream = { ...get().stream };
    delete stream[exit.session_record_id];
    set({
      liveBySession,
      stream,
      turnState: { ...get().turnState, [exit.session_record_id]: "ended" },
    });
  },
  setPermission: (permission) => set({ permission }),
  setPlanQuestion: (planQuestion) => set({ planQuestion }),
  setPlanApproval: (planApproval) => set({ planApproval }),
}));
