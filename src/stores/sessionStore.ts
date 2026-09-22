import { create } from "zustand";
import { useSteerStore } from "@/stores/steerStore";

import { getAgentSessionLogLines, getSessionSubagents } from "@/lib/backend";
import { resolveHistoricalUsage, resolveHistoryLimitTokens } from "@/lib/contextUsage";
import { hydrateSessionLine, parseSubagentTag, type RawSessionLine } from "@/lib/sessionLines";
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
  NativeTurnState,
  NativeSteerSnapshot,
  NativeRequestResolved,
  NativeBackgroundProcess,
  NativeBackgroundProcesses,
  NativeBackgroundTask,
  NativeBackgroundTasks,
  NativeSessionRuntime,
  NativeInputQueue,
  NativeSessionConfigurationEvent,
  PendingSessionConfiguration,
  SessionSubagentInfo,
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
  resolvedRequests: Record<string, Record<string, true>>;
  permissions: Record<string, Record<string, NativePermissionRequest>>;
  planQuestions: Record<string, Record<string, NativePlanQuestionRequest>>;
  planApprovals: Record<string, Record<string, NativePlanApprovalRequest>>;
  worktreeMergePrompt: WorktreeMergePrompt | null;
  mergedWorktreeBySession: Record<string, boolean>;
  autoPromptedWorktreeBySession: Record<string, boolean>;
  pendingAiMergeResolveBySession: Record<string, boolean>;
  hasMoreEarlier: Record<string, boolean>;
  loadingEarlier: Record<string, boolean>;
  subagentsBySession: Record<string, SessionSubagentInfo[]>;
  fetchSubagents: (sessionId: string) => Promise<void>;
  selectSession: (id: string | null) => void;
  ensureHistory: (sessionId: string) => Promise<void>;
  loadHistory: (sessionId: string) => Promise<void>;
  refreshHistory: (sessionId: string) => Promise<void>;
  loadEarlierHistory: (sessionId: string) => Promise<boolean>;
  onStarted: (session: AgentSessionStarted) => boolean;
  onStdout: (output: AgentSessionOutput) => void;
  onDelta: (delta: NativeTextDelta) => void;
  onUsage: (usage: NativeContextUsage) => void;
  onTurnState: (event: NativeTurnState) => boolean;
  onSteerSnapshot: (snapshot: NativeSteerSnapshot) => void;
  onPlanMode: (sessionId: string, planMode: boolean, inputQueueId?: string | null) => void;
  onExit: (exit: AgentSessionExit) => boolean;
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
  onConfiguration: (payload: NativeSessionConfigurationEvent) => boolean;
  openWorktreeMergePrompt: (prompt: WorktreeMergePrompt) => void;
  closeWorktreeMergePrompt: () => void;
  markWorktreeMerged: (sessionId: string) => void;
  markWorktreeAutoPrompted: (sessionId: string) => void;
  markPendingAiMergeResolve: (sessionId: string) => void;
  clearPendingAiMergeResolve: (sessionId: string) => void;
}

function acceptsInteraction(
  state: SessionState,
  request: {
    session_record_id: string;
    request_id: string;
    instance_id?: string | null;
    detached?: boolean;
  },
  kind: string,
): boolean {
  return (
    !state.resolvedRequests[request.session_record_id]?.[`${kind}:${request.request_id}`] &&
    (request.detached ||
      !request.instance_id ||
      useSteerStore.getState().acceptsRuntime(request.session_record_id, request.instance_id))
  );
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
  resolvedRequests: {},
  permissions: {},
  planQuestions: {},
  planApprovals: {},
  worktreeMergePrompt: null,
  mergedWorktreeBySession: {},
  autoPromptedWorktreeBySession: {},
  pendingAiMergeResolveBySession: {},
  subagentsBySession: {},
  fetchSubagents: async (sessionId: string) => {
    try {
      const subagents = await getSessionSubagents(sessionId);
      set((state) => ({
        subagentsBySession: {
          ...state.subagentsBySession,
          [sessionId]: subagents,
        },
      }));
    } catch (error) {
      console.error("Failed to fetch session subagents:", error);
    }
  },
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
    void get().fetchSubagents(sessionId);
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
  refreshHistory: async (sessionId) => {
    const limit = 2000;
    const events = await getAgentSessionLogLines(sessionId, undefined, limit);
    const hasMore = events.length >= limit;
    set((state) => {
      const liveLines = new Map((state.lines[sessionId] ?? []).map((line) => [line.id, line]));
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
    if (!useSteerStore.getState().onStarted(session)) return false;
    const id = session.session_record_id;
    const current = get();
    const replacingRuntime = Boolean(
      current.liveBySession[id] &&
      current.liveBySession[id].input_queue_id !== session.input_queue_id,
    );
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
    // 新一轮开始让上一份 detached 计划作废；live 会话续聊也会走这里，不能动仍在等待的审批。
    const planApprovals = {
      ...current.planApprovals,
      [id]: Object.fromEntries(
        Object.entries(current.planApprovals[id] ?? {}).filter(
          ([, request]) =>
            !request.detached &&
            (!replacingRuntime || request.instance_id === session.input_queue_id),
        ),
      ),
    };
    set({
      liveBySession: { ...current.liveBySession, [id]: { ...session, runtime } },
      inputQueueBySession,
      planApprovals,
      permissions: replacingRuntime ? { ...current.permissions, [id]: {} } : current.permissions,
      planQuestions: replacingRuntime
        ? { ...current.planQuestions, [id]: {} }
        : current.planQuestions,
      stream: replacingRuntime ? { ...current.stream, [id]: [] } : current.stream,
      planModeBySession: { ...current.planModeBySession, [id]: planMode },
      planModeRunBySession,
      configurationRevisionBySession,
      configurationBySession: runtime
        ? { ...current.configurationBySession, [id]: runtime }
        : current.configurationBySession,
      backgroundBySession:
        current.liveBySession[id] && !replacingRuntime
          ? current.backgroundBySession
          : { ...current.backgroundBySession, [id]: [] },
      processesBySession:
        current.liveBySession[id] && !replacingRuntime
          ? current.processesBySession
          : { ...current.processesBySession, [id]: [] },
      turnState: {
        ...current.turnState,
        [id]:
          useSteerStore.getState().lifecycles[id]?.state ??
          (current.liveBySession[id] ? (current.turnState[id] ?? "working") : "working"),
      },
    });
    return true;
  },
  onStdout: (output) => {
    const current = get().lines[output.session_record_id] ?? [];
    if (output.session_event_id && current.some((line) => line.id === output.session_event_id))
      return;
    const parts = get().stream[output.session_record_id];
    const pruned = parts?.length
      ? pruneCoveredFragments(parts, output.line, output.assistant)
      : parts;
    const subagentTag = parseSubagentTag(
      output.assistant ? (output.assistant.subagent_tag ?? "") : output.line,
    );
    let nextSubagents = get().subagentsBySession[output.session_record_id];
    if (subagentTag) {
      const list = [...(nextSubagents ?? [])];
      const idx = list.findIndex((item) => item.id === subagentTag.raw);
      const isEndFailed = output.line.includes("结束 失败");
      const isEndSuccess = output.line.includes("结束 成功");
      const isEndStopped = output.line.includes("结束 停止") || output.line.includes("结束 已停止");
      const status: "running" | "completed" | "failed" | "stopped" = isEndFailed
        ? "failed"
        : isEndSuccess
          ? "completed"
          : isEndStopped
            ? "stopped"
            : "running";
      const now = Date.now();
      if (idx >= 0) {
        const existing = list[idx]!;
        const nextStatus = status === "running" ? existing.status : status;
        list[idx] = {
          ...existing,
          status: nextStatus,
          duration_ms:
            nextStatus !== "running" && existing.start_time_ms
              ? Math.max(0, now - existing.start_time_ms)
              : existing.duration_ms,
          error_message: isEndFailed ? output.line : existing.error_message,
        };
      } else {
        list.push({
          id: subagentTag.raw,
          index: subagentTag.index,
          kind: subagentTag.kind,
          description: subagentTag.description,
          status,
          start_time_ms: now,
          duration_ms: null,
          error_message: isEndFailed ? output.line : null,
        });
      }
      nextSubagents = list;
    }
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
            assistant: output.assistant ?? undefined,
          }),
        ],
      },
      stream:
        pruned && pruned !== parts
          ? { ...get().stream, [output.session_record_id]: pruned }
          : get().stream,
      subagentsBySession: nextSubagents
        ? { ...get().subagentsBySession, [output.session_record_id]: nextSubagents }
        : get().subagentsBySession,
    });
  },
  onDelta: (delta) => {
    const lifecycle = useSteerStore.getState();
    if (!lifecycle.acceptsRuntime(delta.session_record_id, delta.instance_id)) return;
    const turn = lifecycle.lifecycles[delta.session_record_id];
    if (turn && delta.turn_id !== turn.turn_id) return;

    const current = get().stream[delta.session_record_id] ?? [];
    set({
      stream: {
        ...get().stream,
        [delta.session_record_id]: applyTextDelta(current, delta, new Date().toISOString()),
      },
    });
  },
  onUsage: (usage) => set({ usage: { ...get().usage, [usage.session_record_id]: usage } }),
  onTurnState: (event) => {
    if (!useSteerStore.getState().onTurnState(event)) return false;
    set({ turnState: { ...get().turnState, [event.session_record_id]: event.state } });
    return true;
  },
  onSteerSnapshot: (snapshot) => {
    if (!useSteerStore.getState().onSnapshot(snapshot)) return;
    const lifecycle = useSteerStore.getState();
    const turn = lifecycle.lifecycles[snapshot.session_record_id];
    if (turn && !lifecycle.ended[snapshot.session_record_id])
      set({ turnState: { ...get().turnState, [snapshot.session_record_id]: turn.state } });
  },
  onPlanMode: (sessionId, planMode, inputQueueId) =>
    set((state) => {
      const live = state.liveBySession[sessionId];
      if (inputQueueId && !useSteerStore.getState().acceptsRuntime(sessionId, inputQueueId))
        return {};
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
    if (!useSteerStore.getState().onExit(exit.session_record_id, exit.instance_id)) return false;
    const liveBySession = { ...get().liveBySession };
    delete liveBySession[exit.session_record_id];
    const stream = { ...get().stream };
    delete stream[exit.session_record_id];
    const permissions = { ...get().permissions };
    const planQuestions = { ...get().planQuestions };
    delete permissions[exit.session_record_id];
    delete planQuestions[exit.session_record_id];
    // 待批准的计划已落库，这里标记 detached 而不是删除，卡片可以原位继续。
    const planApprovals = {
      ...get().planApprovals,
      [exit.session_record_id]: Object.fromEntries(
        Object.entries(get().planApprovals[exit.session_record_id] ?? {}).map(
          ([requestId, request]) => [requestId, { ...request, detached: true }],
        ),
      ),
    };
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
      subagentsBySession: {
        ...get().subagentsBySession,
        [exit.session_record_id]: (get().subagentsBySession[exit.session_record_id] ?? []).map(
          (subagent) =>
            subagent.status === "running" ? { ...subagent, status: "stopped" } : subagent,
        ),
      },
      turnState: { ...get().turnState, [exit.session_record_id]: "ended" },
    });
    return true;
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
    set((state) =>
      acceptsInteraction(state, request, "permission")
        ? {
            permissions: {
              ...state.permissions,
              [request.session_record_id]: {
                ...state.permissions[request.session_record_id],
                [request.request_id]: request,
              },
            },
          }
        : state,
    ),
  setPlanQuestion: (request) =>
    set((state) =>
      acceptsInteraction(state, request, "question")
        ? {
            planQuestions: {
              ...state.planQuestions,
              [request.session_record_id]: {
                ...state.planQuestions[request.session_record_id],
                [request.request_id]: request,
              },
            },
          }
        : state,
    ),
  setPlanApproval: (request) =>
    set((state) =>
      acceptsInteraction(state, request, "plan_approval")
        ? {
            planApprovals: {
              ...state.planApprovals,
              [request.session_record_id]: {
                ...state.planApprovals[request.session_record_id],
                [request.request_id]: request,
              },
            },
          }
        : state,
    ),
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
      return {
        [key]: { ...state[key], [id]: requests },
        resolvedRequests: {
          ...state.resolvedRequests,
          [id]: { ...state.resolvedRequests[id], [`${kind}:${requestId}`]: true },
        },
      };
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
  onConfiguration: (payload) => {
    const state = get();
    const id = payload.session_record_id;
    if (
      payload.input_queue_id &&
      !useSteerStore.getState().acceptsRuntime(id, payload.input_queue_id)
    )
      return false;
    const live = state.liveBySession[id];
    if (
      payload.input_queue_id &&
      live?.input_queue_id &&
      payload.input_queue_id !== live.input_queue_id
    )
      return false;
    if (payload.error) {
      if (state.pendingConfigurationBySession[id]?.request_id !== payload.request_id) return false;
      const pendingConfigurationBySession = { ...state.pendingConfigurationBySession };
      delete pendingConfigurationBySession[id];
      set({ pendingConfigurationBySession });
      return true;
    }
    if (!payload.runtime || payload.revision <= (state.configurationRevisionBySession[id] ?? 0))
      return false;
    const pendingConfigurationBySession = { ...state.pendingConfigurationBySession };
    if (pendingConfigurationBySession[id]?.request_id === payload.request_id)
      delete pendingConfigurationBySession[id];
    set({
      pendingConfigurationBySession,
      configurationRevisionBySession: {
        ...state.configurationRevisionBySession,
        [id]: payload.revision,
      },
      configurationBySession: { ...state.configurationBySession, [id]: payload.runtime },
      liveBySession: live
        ? { ...state.liveBySession, [id]: { ...live, runtime: payload.runtime } }
        : state.liveBySession,
    });
    return true;
  },
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
