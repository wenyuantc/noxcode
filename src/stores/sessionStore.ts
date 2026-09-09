import { create } from "zustand";

import { getAgentSessionLogLines } from "@/lib/backend";
import { resolveHistoricalUsage, resolveHistoryLimitTokens } from "@/lib/contextUsage";
import { hydrateSessionLine, type RawSessionLine } from "@/lib/sessionLines";
import {
  applyTextDelta,
  pruneCoveredFragments,
  type SessionStreamFragment,
} from "@/lib/sessionStream";
import type {
  AgentSessionExit,
  AgentSessionOutput,
  AgentSessionStarted,
  NativeContextUsage,
  NativePermissionRequest,
  NativePlanApprovalRequest,
  NativePlanQuestionRequest,
  NativeTextDelta,
  NativeRequestResolved,
  NativeBackgroundProcess,
  NativeBackgroundProcesses,
  NativeBackgroundTask,
  NativeBackgroundTasks,
  NativeSessionRuntime,
  NativeInputQueue,
  NativeSessionConfigurationEvent,
  PendingSessionConfiguration,
  WorktreeMergePrompt,
} from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

function hydrateUsage(sessionId: string) {
  const current = useSessionStore.getState();
  if (current.usage[sessionId]) return;
  const workspace = useWorkspaceStore.getState();
  const channels = useChannelStore.getState();
  const session = workspace.sessions.find((item) => item.id === sessionId);
  const runtime = current.configurationBySession[sessionId];
  const usage = resolveHistoricalUsage({
    sessionId,
    contextUsageJson: session?.context_usage_json,
    lines: current.lines[sessionId] ?? [],
    limitTokens: resolveHistoryLimitTokens(
      {
        ai_channel_id: runtime?.ai_channel_id ?? session?.ai_channel_id ?? null,
        model: runtime?.model ?? session?.model ?? null,
      },
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
  planModeBySession: Record<string, boolean>;
  planModeRunBySession: Record<string, string | null>;
  lines: Record<string, RawSessionLine[]>;
  historyLoaded: Record<string, boolean>;
  configurationBySession: Record<string, NativeSessionRuntime>;
  pendingConfigurationBySession: Record<string, PendingSessionConfiguration>;
  configurationRevisionBySession: Record<string, number>;
  backgroundBySession: Record<string, NativeBackgroundTask[]>;
  processesBySession: Record<string, NativeBackgroundProcess[]>;
  inputQueueBySession: Record<string, NativeInputQueue>;
  turnState: Record<string, string>;
  usage: Record<string, NativeContextUsage>;
  stream: Record<string, SessionStreamFragment[]>;
  permissions: Record<string, Record<string, NativePermissionRequest>>;
  planQuestions: Record<string, Record<string, NativePlanQuestionRequest>>;
  planApprovals: Record<string, Record<string, NativePlanApprovalRequest>>;
  worktreeMergePrompt: WorktreeMergePrompt | null;
  mergedWorktreeBySession: Record<string, boolean>;
  autoPromptedWorktreeBySession: Record<string, boolean>;
  pendingAiMergeResolveBySession: Record<string, boolean>;
  hasMoreEarlier: Record<string, boolean>;
  loadingEarlier: Record<string, boolean>;
  selectSession: (id: string | null) => void;
  ensureHistory: (sessionId: string) => Promise<void>;
  loadHistory: (sessionId: string) => Promise<void>;
  loadEarlierHistory: (sessionId: string) => Promise<boolean>;
  onStarted: (session: AgentSessionStarted) => void;
  onStdout: (output: AgentSessionOutput) => void;
  onDelta: (delta: NativeTextDelta) => void;
  onUsage: (usage: NativeContextUsage) => void;
  onTurnState: (sessionId: string, state: string) => void;
  onPlanMode: (sessionId: string, planMode: boolean, inputQueueId?: string | null) => void;
  onExit: (exit: AgentSessionExit) => void;
  setPermission: (request: NativePermissionRequest) => void;
  setPlanQuestion: (request: NativePlanQuestionRequest) => void;
  setPlanApproval: (request: NativePlanApprovalRequest) => void;
  resolveRequest: (request: NativeRequestResolved) => void;
  onBackgroundTasks: (payload: NativeBackgroundTasks) => void;
  onBackgroundProcesses: (payload: NativeBackgroundProcesses) => void;
  onInputQueue: (payload: NativeInputQueue) => void;
  setConfiguration: (sessionId: string, runtime: NativeSessionRuntime) => void;
  setPendingConfiguration: (sessionId: string, pending: PendingSessionConfiguration) => void;
  clearPendingConfiguration: (sessionId: string) => void;
  onConfiguration: (payload: NativeSessionConfigurationEvent) => void;
  openWorktreeMergePrompt: (prompt: WorktreeMergePrompt) => void;
  closeWorktreeMergePrompt: () => void;
  markWorktreeMerged: (sessionId: string) => void;
  markWorktreeAutoPrompted: (sessionId: string) => void;
  markPendingAiMergeResolve: (sessionId: string) => void;
  clearPendingAiMergeResolve: (sessionId: string) => void;
}

const historyRequests = new Map<string, Promise<void>>();

export const useSessionStore = create<SessionState>((set, get) => ({
  selectedSessionId: null,
  liveBySession: {},
  planModeBySession: {},
  planModeRunBySession: {},
  lines: {},
  historyLoaded: {},
  hasMoreEarlier: {},
  loadingEarlier: {},
  configurationBySession: {},
  pendingConfigurationBySession: {},
  configurationRevisionBySession: {},
  backgroundBySession: {},
  processesBySession: {},
  inputQueueBySession: {},
  turnState: {},
  usage: {},
  stream: {},
  permissions: {},
  planQuestions: {},
  planApprovals: {},
  worktreeMergePrompt: null,
  mergedWorktreeBySession: {},
  autoPromptedWorktreeBySession: {},
  pendingAiMergeResolveBySession: {},
  selectSession: (id) => {
    const session = id
      ? useWorkspaceStore.getState().sessions.find((item) => item.id === id)
      : undefined;
    const fallback = session ? session.session_kind === "plan" : undefined;
    set((state) => {
      if (
        !id ||
        fallback === undefined ||
        Object.prototype.hasOwnProperty.call(state.planModeBySession, id)
      ) {
        return { selectedSessionId: id };
      }
      return {
        selectedSessionId: id,
        planModeBySession: { ...state.planModeBySession, [id]: fallback },
      };
    });
  },
  ensureHistory: async (sessionId) => {
    if (!get().historyLoaded[sessionId]) {
      let request = historyRequests.get(sessionId);
      if (!request) {
        request = (async () => {
          const limit = 2000;
          const events = await getAgentSessionLogLines(sessionId, undefined, limit);
          const hasMore = events.length >= limit;
          set((state) => {
            const liveLines = new Map(
              (state.lines[sessionId] ?? []).map((line) => [line.id, line]),
            );
            const history = events.map(
              (event) =>
                liveLines.get(event.id) ??
                hydrateSessionLine({
                  id: event.id,
                  sessionId,
                  text: event.message ?? "",
                  createdAt: event.created_at,
                }),
            );
            const ids = new Set(history.map((line) => line.id));
            return {
              lines: {
                ...state.lines,
                [sessionId]: [
                  ...history,
                  ...(state.lines[sessionId] ?? []).filter((line) => !ids.has(line.id)),
                ],
              },
              historyLoaded: { ...state.historyLoaded, [sessionId]: true },
              hasMoreEarlier: { ...state.hasMoreEarlier, [sessionId]: hasMore },
            };
          });
        })().finally(() => historyRequests.delete(sessionId));
        historyRequests.set(sessionId, request);
      }
      await request;
    }
    hydrateUsage(sessionId);
  },
  loadEarlierHistory: async (sessionId: string) => {
    const state = get();
    if (state.loadingEarlier[sessionId] || !state.hasMoreEarlier[sessionId]) {
      return false;
    }
    const currentLines = state.lines[sessionId] ?? [];
    const firstLineId = currentLines[0]?.id;
    if (!firstLineId) return false;

    set((s) => ({
      loadingEarlier: { ...s.loadingEarlier, [sessionId]: true },
    }));

    try {
      const limit = 1000;
      const events = await getAgentSessionLogLines(sessionId, undefined, limit, firstLineId);
      const hasMore = events.length >= limit;
      const history = events.map((event) =>
        hydrateSessionLine({
          id: event.id,
          sessionId,
          text: event.message ?? "",
          createdAt: event.created_at,
        }),
      );
      const newIds = new Set(history.map((line) => line.id));
      set((s) => ({
        lines: {
          ...s.lines,
          [sessionId]: [
            ...history,
            ...(s.lines[sessionId] ?? []).filter((line) => !newIds.has(line.id)),
          ],
        },
        hasMoreEarlier: { ...s.hasMoreEarlier, [sessionId]: hasMore },
        loadingEarlier: { ...s.loadingEarlier, [sessionId]: false },
      }));
      return history.length > 0;
    } catch {
      set((s) => ({
        loadingEarlier: { ...s.loadingEarlier, [sessionId]: false },
      }));
      return false;
    }
  },
  loadHistory: async (sessionId) => {
    get().selectSession(sessionId);
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
    const id = session.session_record_id;
    const current = get();
    const inputQueueBySession = { ...current.inputQueueBySession };
    if (inputQueueBySession[id]?.queue_id !== session.input_queue_id) {
      delete inputQueueBySession[id];
    }
    const planModeRunBySession = { ...current.planModeRunBySession };
    const hasModeEvent =
      Object.prototype.hasOwnProperty.call(planModeRunBySession, id) &&
      planModeRunBySession[id] === (session.input_queue_id ?? null);
    const planMode = hasModeEvent
      ? current.planModeBySession[id]
      : (session.runtime?.plan_mode ??
        current.planModeBySession[id] ??
        session.session_kind === "plan");
    if (!hasModeEvent) delete planModeRunBySession[id];
    const runtime = session.runtime ? { ...session.runtime, plan_mode: planMode } : session.runtime;
    const configurationRevisionBySession = { ...current.configurationRevisionBySession };
    if (current.liveBySession[id]?.input_queue_id !== session.input_queue_id) {
      delete configurationRevisionBySession[id];
    }
    set({
      liveBySession: { ...current.liveBySession, [id]: { ...session, runtime } },
      inputQueueBySession,
      planModeBySession: { ...current.planModeBySession, [id]: planMode },
      planModeRunBySession,
      configurationRevisionBySession,
      configurationBySession: runtime
        ? { ...current.configurationBySession, [id]: runtime }
        : current.configurationBySession,
      backgroundBySession: current.liveBySession[id]
        ? current.backgroundBySession
        : { ...current.backgroundBySession, [id]: [] },
      processesBySession: current.liveBySession[id]
        ? current.processesBySession
        : { ...current.processesBySession, [id]: [] },
      turnState: {
        ...current.turnState,
        [id]: current.liveBySession[id] ? (current.turnState[id] ?? "working") : "working",
      },
    });
  },
  onStdout: (output) => {
    const current = get().lines[output.session_record_id] ?? [];
    if (output.session_event_id && current.some((line) => line.id === output.session_event_id))
      return;
    const parts = get().stream[output.session_record_id];
    const pruned = parts?.length ? pruneCoveredFragments(parts, output.line) : parts;
    set({
      lines: {
        ...get().lines,
        [output.session_record_id]: [
          ...current,
          hydrateSessionLine({
            id: output.session_event_id,
            sessionId: output.session_record_id,
            text: output.line,
            createdAt: new Date().toISOString(),
            tool: output.tool ?? undefined,
            images: output.images ?? undefined,
          }),
        ],
      },
      stream:
        pruned && pruned !== parts
          ? { ...get().stream, [output.session_record_id]: pruned }
          : get().stream,
    });
  },
  onDelta: (delta) => {
    const current = get().stream[delta.session_record_id] ?? [];
    set({
      stream: {
        ...get().stream,
        [delta.session_record_id]: applyTextDelta(current, delta, new Date().toISOString()),
      },
    });
  },
  onUsage: (usage) => set({ usage: { ...get().usage, [usage.session_record_id]: usage } }),
  onTurnState: (sessionId, state) => set({ turnState: { ...get().turnState, [sessionId]: state } }),
  onPlanMode: (sessionId, planMode, inputQueueId) =>
    set((state) => {
      const live = state.liveBySession[sessionId];
      if (inputQueueId && live?.input_queue_id && inputQueueId !== live.input_queue_id) return {};
      return {
        planModeBySession: { ...state.planModeBySession, [sessionId]: planMode },
        planModeRunBySession: {
          ...state.planModeRunBySession,
          [sessionId]: inputQueueId ?? live?.input_queue_id ?? null,
        },
        liveBySession: live?.runtime
          ? {
              ...state.liveBySession,
              [sessionId]: { ...live, runtime: { ...live.runtime, plan_mode: planMode } },
            }
          : state.liveBySession,
        configurationBySession: state.configurationBySession[sessionId]
          ? {
              ...state.configurationBySession,
              [sessionId]: { ...state.configurationBySession[sessionId], plan_mode: planMode },
            }
          : state.configurationBySession,
      };
    }),
  onExit: (exit) => {
    const liveBySession = { ...get().liveBySession };
    delete liveBySession[exit.session_record_id];
    const stream = { ...get().stream };
    delete stream[exit.session_record_id];
    const permissions = { ...get().permissions };
    const planQuestions = { ...get().planQuestions };
    const planApprovals = { ...get().planApprovals };
    delete permissions[exit.session_record_id];
    delete planQuestions[exit.session_record_id];
    delete planApprovals[exit.session_record_id];
    const inputQueueBySession = { ...get().inputQueueBySession };
    delete inputQueueBySession[exit.session_record_id];
    const planModeRunBySession = { ...get().planModeRunBySession };
    delete planModeRunBySession[exit.session_record_id];
    const pendingConfigurationBySession = { ...get().pendingConfigurationBySession };
    delete pendingConfigurationBySession[exit.session_record_id];
    set({
      liveBySession,
      stream,
      permissions,
      planQuestions,
      planApprovals,
      inputQueueBySession,
      planModeRunBySession,
      pendingConfigurationBySession,
      backgroundBySession: {
        ...get().backgroundBySession,
        [exit.session_record_id]: (get().backgroundBySession[exit.session_record_id] ?? []).map(
          (task) =>
            task.status === "running" || task.status === "queued"
              ? { ...task, status: "stopped" }
              : task,
        ),
      },
      processesBySession: {
        ...get().processesBySession,
        [exit.session_record_id]: (get().processesBySession[exit.session_record_id] ?? []).map(
          (process) => (process.status === "running" ? { ...process, status: "stopped" } : process),
        ),
      },
      turnState: { ...get().turnState, [exit.session_record_id]: "ended" },
    });
  },
  onInputQueue: (payload) => {
    const state = get();
    const live = state.liveBySession[payload.session_record_id];
    if (!live || (live.input_queue_id && live.input_queue_id !== payload.queue_id)) return;
    const current = state.inputQueueBySession[payload.session_record_id];
    if (current?.queue_id === payload.queue_id && current.revision >= payload.revision) return;
    set({
      inputQueueBySession: { ...state.inputQueueBySession, [payload.session_record_id]: payload },
    });
  },
  setPermission: (request) =>
    set((state) => ({
      permissions: {
        ...state.permissions,
        [request.session_record_id]: {
          ...state.permissions[request.session_record_id],
          [request.request_id]: request,
        },
      },
    })),
  setPlanQuestion: (request) =>
    set((state) => ({
      planQuestions: {
        ...state.planQuestions,
        [request.session_record_id]: {
          ...state.planQuestions[request.session_record_id],
          [request.request_id]: request,
        },
      },
    })),
  setPlanApproval: (request) =>
    set((state) => ({
      planApprovals: {
        ...state.planApprovals,
        [request.session_record_id]: {
          ...state.planApprovals[request.session_record_id],
          [request.request_id]: request,
        },
      },
    })),
  resolveRequest: ({ session_record_id: id, request_id: requestId, kind }) =>
    set((state) => {
      const key =
        kind === "permission"
          ? "permissions"
          : kind === "question"
            ? "planQuestions"
            : "planApprovals";
      const requests = { ...state[key][id] };
      delete requests[requestId];
      return { [key]: { ...state[key], [id]: requests } };
    }),
  onBackgroundTasks: ({ session_record_id, tasks }) =>
    set((state) => ({
      backgroundBySession: { ...state.backgroundBySession, [session_record_id]: tasks },
    })),
  onBackgroundProcesses: ({ session_record_id, processes }) =>
    set((state) => ({
      processesBySession: { ...state.processesBySession, [session_record_id]: processes },
    })),
  setConfiguration: (sessionId, runtime) =>
    set((state) => ({
      configurationBySession: { ...state.configurationBySession, [sessionId]: runtime },
      planModeBySession: { ...state.planModeBySession, [sessionId]: runtime.plan_mode },
    })),
  setPendingConfiguration: (sessionId, pending) =>
    set((state) => ({
      pendingConfigurationBySession: {
        ...state.pendingConfigurationBySession,
        [sessionId]: pending,
      },
    })),
  clearPendingConfiguration: (sessionId) =>
    set((state) => {
      if (!Object.prototype.hasOwnProperty.call(state.pendingConfigurationBySession, sessionId)) {
        return {};
      }
      const pendingConfigurationBySession = { ...state.pendingConfigurationBySession };
      delete pendingConfigurationBySession[sessionId];
      return { pendingConfigurationBySession };
    }),
  onConfiguration: (payload) =>
    set((state) => {
      const id = payload.session_record_id;
      if (payload.error) {
        const pending = state.pendingConfigurationBySession[id];
        if (pending?.request_id !== payload.request_id) return {};
        const pendingConfigurationBySession = { ...state.pendingConfigurationBySession };
        delete pendingConfigurationBySession[id];
        return { pendingConfigurationBySession };
      }
      if (!payload.runtime) return {};
      const live = state.liveBySession[id];
      if (
        payload.input_queue_id &&
        live?.input_queue_id &&
        payload.input_queue_id !== live.input_queue_id
      ) {
        return {};
      }
      const currentRevision = state.configurationRevisionBySession[id] ?? 0;
      if (payload.revision <= currentRevision) return {};
      const pending = state.pendingConfigurationBySession[id];
      const pendingConfigurationBySession = { ...state.pendingConfigurationBySession };
      if (pending?.request_id === payload.request_id) {
        delete pendingConfigurationBySession[id];
      }
      return {
        pendingConfigurationBySession,
        configurationRevisionBySession: {
          ...state.configurationRevisionBySession,
          [id]: payload.revision,
        },
        configurationBySession: {
          ...state.configurationBySession,
          [id]: payload.runtime,
        },
        liveBySession: live
          ? {
              ...state.liveBySession,
              [id]: { ...live, runtime: payload.runtime },
            }
          : state.liveBySession,
      };
    }),
  openWorktreeMergePrompt: (prompt) => set({ worktreeMergePrompt: prompt }),
  closeWorktreeMergePrompt: () => set({ worktreeMergePrompt: null }),
  markWorktreeMerged: (sessionId) =>
    set((state) => ({
      mergedWorktreeBySession: { ...state.mergedWorktreeBySession, [sessionId]: true },
      worktreeMergePrompt:
        state.worktreeMergePrompt?.sessionId === sessionId ? null : state.worktreeMergePrompt,
    })),
  markWorktreeAutoPrompted: (sessionId) =>
    set((state) => ({
      autoPromptedWorktreeBySession: {
        ...state.autoPromptedWorktreeBySession,
        [sessionId]: true,
      },
    })),
  markPendingAiMergeResolve: (sessionId) =>
    set((state) => ({
      pendingAiMergeResolveBySession: {
        ...state.pendingAiMergeResolveBySession,
        [sessionId]: true,
      },
    })),
  clearPendingAiMergeResolve: (sessionId) =>
    set((state) => {
      if (!state.pendingAiMergeResolveBySession[sessionId]) return {};
      const pendingAiMergeResolveBySession = { ...state.pendingAiMergeResolveBySession };
      delete pendingAiMergeResolveBySession[sessionId];
      return { pendingAiMergeResolveBySession };
    }),
}));
