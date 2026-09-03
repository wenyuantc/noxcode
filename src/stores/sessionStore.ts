import { create } from "zustand";

import { getAgentSessionLogLines } from "@/lib/backend";
import type { RawSessionLine } from "@/lib/sessionLines";
import type {
  AgentSessionExit,
  AgentSessionOutput,
  AgentSessionStarted,
  NativeContextUsage,
  NativePermissionRequest,
  NativePlanQuestionRequest,
  NativeTextDelta,
} from "@/lib/types";
import { useWorkspaceStore } from "@/stores/workspaceStore";

interface SessionState {
  selectedSessionId: string | null;
  liveBySession: Record<string, AgentSessionStarted>;
  lines: Record<string, RawSessionLine[]>;
  turnState: Record<string, string>;
  usage: Record<string, NativeContextUsage>;
  stream: Record<string, { kind: string; text: string }>;
  permission: NativePermissionRequest | null;
  planQuestion: NativePlanQuestionRequest | null;
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
  selectSession: (id) => set({ selectedSessionId: id }),
  ensureHistory: async (sessionId) => {
    if (get().lines[sessionId]) return;
    const events = await getAgentSessionLogLines(sessionId);
    if (get().lines[sessionId]) return;
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
}));
