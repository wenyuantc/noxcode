import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentSession,
  AgentSessionEvent,
  AgentSessionOutput,
  AgentSessionStarted,
} from "@/lib/types";
import { resolveComposerPlanMode } from "@/lib/planMode";

vi.mock("@/lib/backend", () => ({
  getAgentSessionLogLines: vi.fn(),
}));

import { getAgentSessionLogLines } from "@/lib/backend";
import { useChannelStore } from "./channelStore";
import { useSessionStore } from "./sessionStore";
import { useWorkspaceStore } from "./workspaceStore";

const getLines = vi.mocked(getAgentSessionLogLines);

function event(id: string, sessionId = "s1"): AgentSessionEvent {
  return {
    id,
    session_id: sessionId,
    event_type: "stdout",
    message: id,
    created_at: id,
  };
}

function stdout(sessionId: string, eventId: string): AgentSessionOutput {
  return {
    profile_id: "p",
    workspace_id: null,
    session_kind: "agent",
    session_record_id: sessionId,
    session_event_id: eventId,
    line: eventId,
  };
}

function started(sessionId: string, sessionKind: string): AgentSessionStarted {
  return {
    profile_id: "p",
    workspace_id: "ws-1",
    session_kind: sessionKind,
    session_record_id: sessionId,
  };
}

describe("sessionStore history", () => {
  beforeEach(() => {
    getLines.mockReset();
    useSessionStore.setState({
      selectedSessionId: null,
      liveBySession: {},
      planModeBySession: {},
      planModeRunBySession: {},
      lines: {},
      turnState: {},
      usage: {},
      stream: {},
      historyLoaded: {},
      configurationBySession: {},
      backgroundBySession: {},
      inputQueueBySession: {},
      permissions: {},
      planQuestions: {},
      planApprovals: {},
    });
    useWorkspaceStore.setState({ sessions: [] });
    useChannelStore.setState({
      channels: [],
      activeChannelId: null,
      activeModelId: null,
    });
  });

  it("keeps queued input separate from history and rejects stale snapshots", () => {
    useSessionStore.getState().onStarted({ ...started("s1", "execution"), input_queue_id: "q1" });
    useSessionStore.getState().onStarted({ ...started("s2", "execution"), input_queue_id: "q2" });
    const payload = {
      session_record_id: "s1",
      queue_id: "q1",
      revision: 2,
      items: [{ id: "i1", text: "pending", image_count: 0, editing: false }],
    };
    useSessionStore.getState().onInputQueue(payload);
    useSessionStore.getState().onInputQueue({ ...payload, revision: 1, items: [] });
    useSessionStore
      .getState()
      .onInputQueue({ ...payload, queue_id: "old-runtime", revision: 100, items: [] });
    expect(useSessionStore.getState().inputQueueBySession.s1.items[0].text).toBe("pending");
    expect(useSessionStore.getState().inputQueueBySession.s2).toBeUndefined();
    expect(useSessionStore.getState().lines.s1).toBeUndefined();
    useSessionStore.getState().onInputQueue({ ...payload, revision: 3, items: [] });
    useSessionStore.getState().onInputQueue(payload);
    expect(useSessionStore.getState().inputQueueBySession.s1.items).toEqual([]);
  });

  it("clears queued input on exit and rejects old runtime events after restart", () => {
    const session = { ...started("s1", "execution"), input_queue_id: "q1" };
    const payload = { session_record_id: "s1", queue_id: "q1", revision: 1, items: [] };
    useSessionStore.getState().onStarted(session);
    useSessionStore.getState().onInputQueue(payload);
    useSessionStore.getState().onExit({ ...session, code: 0 });
    useSessionStore.getState().onInputQueue({ ...payload, revision: 2 });
    expect(useSessionStore.getState().inputQueueBySession.s1).toBeUndefined();
    useSessionStore.getState().onStarted({ ...session, input_queue_id: "q2" });
    useSessionStore.getState().onInputQueue({ ...payload, revision: 3 });
    expect(useSessionStore.getState().inputQueueBySession.s1).toBeUndefined();
    useSessionStore.getState().onInputQueue({ ...payload, queue_id: "q2" });
    useSessionStore.getState().onStarted({ ...session, input_queue_id: "q2" });
    expect(useSessionStore.getState().inputQueueBySession.s1.queue_id).toBe("q2");
  });

  it("selects immediately before history returns", async () => {
    let resolve: ((value: AgentSessionEvent[]) => void) | undefined;
    getLines.mockReturnValue(
      new Promise((next) => {
        resolve = next;
      }),
    );

    const pending = useSessionStore.getState().loadHistory("s1");
    expect(useSessionStore.getState().selectedSessionId).toBe("s1");
    expect(useSessionStore.getState().lines.s1).toBeUndefined();

    resolve?.([event("e1")]);
    await pending;
    expect(useSessionStore.getState().lines.s1?.map((line) => line.id)).toEqual(["e1"]);
  });

  it("tracks runtime plan mode per session and preserves false values", () => {
    useSessionStore.getState().onPlanMode("s1", false);
    useSessionStore.getState().onStarted(started("s1", "plan"));
    expect(useSessionStore.getState().planModeBySession.s1).toBe(false);

    useSessionStore.getState().onPlanMode("s2", true);
    expect(useSessionStore.getState().planModeBySession).toEqual({ s1: false, s2: true });
  });

  it("keeps background session mode changes isolated from the active session", () => {
    useSessionStore.getState().onStarted(started("active", "execution"));
    useSessionStore.getState().onPlanMode("background", true);
    const modes = useSessionStore.getState().planModeBySession;

    expect(resolveComposerPlanMode("active", modes, true)).toBe(false);
    expect(resolveComposerPlanMode("background", modes, false)).toBe(true);
  });

  it("keeps mode events authoritative over delayed startup snapshots for the same run", () => {
    const snapshot = {
      ...started("s1", "plan"),
      input_queue_id: "q1",
      runtime: {
        ai_channel_id: "ch",
        model: "model",
        reasoning_effort: null,
        permission_mode: "yolo",
        plan_mode: true,
      },
    };
    useSessionStore.getState().onPlanMode("s1", false, "q1");
    useSessionStore.getState().onStarted(snapshot);
    useSessionStore.getState().onStarted(snapshot);
    let state = useSessionStore.getState();
    expect(state.planModeBySession.s1).toBe(false);
    expect(state.configurationBySession.s1.plan_mode).toBe(false);
    expect(state.liveBySession.s1.runtime?.plan_mode).toBe(false);
    state.onPlanMode("s1", true, "q1");
    state.onStarted({ ...snapshot, runtime: { ...snapshot.runtime, plan_mode: false } });
    state = useSessionStore.getState();
    expect(state.planModeBySession.s1).toBe(true);
    expect(state.configurationBySession.s1.plan_mode).toBe(true);
    expect(state.liveBySession.s1.runtime?.plan_mode).toBe(true);
  });

  it("initializes a new run from runtime and ignores mode events from the previous run", () => {
    const snapshot = {
      ...started("s1", "plan"),
      input_queue_id: "q1",
      runtime: {
        ai_channel_id: "ch",
        model: "model",
        reasoning_effort: null,
        permission_mode: "default",
        plan_mode: false,
      },
    };
    useSessionStore.getState().onStarted(snapshot);
    useSessionStore.getState().onPlanMode("s1", true, "q1");
    useSessionStore.getState().onExit({ ...snapshot, code: 0 });
    useSessionStore.getState().onStarted({ ...snapshot, input_queue_id: "q2" });
    useSessionStore.getState().onPlanMode("s1", true, "q1");
    useSessionStore.getState().selectSession("s1");
    const state = useSessionStore.getState();
    expect(state.planModeBySession.s1).toBe(false);
    expect(state.configurationBySession.s1.plan_mode).toBe(false);
    expect(state.liveBySession.s1.runtime?.plan_mode).toBe(false);
  });

  it("initializes a selected historical session from its session kind", () => {
    useWorkspaceStore.setState({
      sessions: [
        {
          id: "s-plan",
          ai_channel_id: null,
          workspace_id: null,
          working_dir: null,
          execution_target: "local",
          ssh_config_id: null,
          target_host_label: null,
          session_kind: "plan",
          status: "exited",
          started_at: "t",
          ended_at: null,
          exit_code: null,
          resume_session_id: null,
          pinned: 0,
          archived: 0,
          input_tokens: null,
          output_tokens: null,
          total_tokens: null,
          reasoning_tokens: null,
          cached_tokens: null,
          created_at: "t",
        } satisfies AgentSession,
      ],
    });

    useSessionStore.getState().selectSession("s-plan");
    expect(useSessionStore.getState().planModeBySession["s-plan"]).toBe(true);
  });

  it("unwraps persisted tool envelopes when loading history", async () => {
    getLines.mockResolvedValue([
      {
        id: "e1",
        session_id: "s1",
        event_type: "stdout",
        message: JSON.stringify({
          nox: 1,
          line: "[读取] a.ts",
          tool: { phase: "start", call_id: "c1", name: "Read", title: "读取 a.ts" },
        }),
        created_at: "t",
      },
    ]);
    await useSessionStore.getState().ensureHistory("s1");
    const loaded = useSessionStore.getState().lines.s1?.[0];
    expect(loaded?.text).toBe("[读取] a.ts");
    expect(loaded?.tool?.call_id).toBe("c1");
  });

  it("skips fetch when history is already cached", async () => {
    useSessionStore.setState({
      lines: { s1: [{ id: "cached", sessionId: "s1", text: "cached", createdAt: "t" }] },
      historyLoaded: { s1: true },
    });
    await useSessionStore.getState().loadHistory("s1");
    expect(getLines).not.toHaveBeenCalled();
    expect(useSessionStore.getState().selectedSessionId).toBe("s1");
    expect(useSessionStore.getState().lines.s1?.[0]?.id).toBe("cached");
  });

  it("merges history with live events received while fetch is in flight", async () => {
    let resolve: ((value: AgentSessionEvent[]) => void) | undefined;
    getLines.mockReturnValue(
      new Promise((next) => {
        resolve = next;
      }),
    );

    const pending = useSessionStore.getState().loadHistory("s1");
    useSessionStore.getState().onStdout(stdout("s1", "live"));
    resolve?.([event("old")]);
    await pending;

    expect(useSessionStore.getState().lines.s1?.map((line) => line.id)).toEqual(["old", "live"]);
  });

  it("preserves live-only images when the same persisted event arrives", async () => {
    const images = [
      { name: "image.png", mime_type: "image/png", data_url: "data:image/png;base64,aW1hZ2U=" },
    ];
    useSessionStore.getState().onStdout({ ...stdout("s1", "image"), images });
    getLines.mockResolvedValue([event("image")]);
    await useSessionStore.getState().ensureHistory("s1");
    expect(useSessionStore.getState().lines.s1).toHaveLength(1);
    expect(useSessionStore.getState().lines.s1[0].images).toEqual(images);
  });

  it("does not discard unpersisted output lacking an event id", () => {
    useSessionStore.getState().onStdout({ ...stdout("s1", ""), line: "first" });
    useSessionStore.getState().onStdout({ ...stdout("s1", ""), line: "second" });
    expect(useSessionStore.getState().lines.s1.map((line) => line.text)).toEqual([
      "first",
      "second",
    ]);
  });

  it("closes background tasks on exit and clears them on a new runtime", () => {
    useSessionStore.getState().onStarted(started("s1", "execution"));
    useSessionStore.getState().onBackgroundTasks({
      session_record_id: "s1",
      tasks: [
        {
          task_id: "task-1",
          description: "test",
          kind: "general",
          status: "running",
          report: null,
        },
      ],
    });
    useSessionStore.getState().onExit({ ...started("s1", "execution"), code: 0 });
    expect(useSessionStore.getState().backgroundBySession.s1[0].status).toBe("stopped");
    useSessionStore.getState().onStarted(started("s1", "execution"));
    expect(useSessionStore.getState().backgroundBySession.s1).toEqual([]);
  });

  it("fetches history even when stdout arrived before opening a session", async () => {
    useSessionStore.getState().onStdout(stdout("s1", "live"));
    getLines.mockResolvedValue([event("old"), event("live")]);
    await useSessionStore.getState().ensureHistory("s1");
    useSessionStore.getState().onStdout(stdout("s1", "live"));
    expect(useSessionStore.getState().lines.s1.map((line) => line.id)).toEqual(["old", "live"]);
    expect(getLines).toHaveBeenCalledTimes(1);
  });

  it("does not change selection when a background session starts", () => {
    useSessionStore.getState().selectSession("active");
    useSessionStore.getState().onStarted(started("background", "execution"));
    expect(useSessionStore.getState().selectedSessionId).toBe("active");
  });

  it("keeps concurrent requests and only removes the resolved request", () => {
    const request = {
      session_record_id: "s1",
      request_id: "r1",
      profile_id: "p",
      workspace_id: "ws-1",
      session_kind: "execution",
      plan: "plan",
    };
    useSessionStore.getState().setPlanApproval(request);
    useSessionStore.getState().setPlanApproval({ ...request, request_id: "r2" });
    useSessionStore.getState().setPlanApproval({ ...request, session_record_id: "s2" });
    useSessionStore.getState().resolveRequest({ ...request, kind: "plan_approval" });
    expect(Object.keys(useSessionStore.getState().planApprovals.s1)).toEqual(["r2"]);
    expect(Object.keys(useSessionStore.getState().planApprovals.s2)).toEqual(["r1"]);
    useSessionStore.getState().onExit({ ...started("s1", "execution"), code: 0 });
    expect(useSessionStore.getState().planApprovals.s1).toBeUndefined();
    expect(useSessionStore.getState().planApprovals.s2.r1).toBeDefined();
  });

  it("keeps a previous fetch in cache after switching away", async () => {
    let resolveA: ((value: AgentSessionEvent[]) => void) | undefined;
    let resolveB: ((value: AgentSessionEvent[]) => void) | undefined;
    getLines.mockImplementation((sessionId: string) => {
      return new Promise((next) => {
        if (sessionId === "a") resolveA = next;
        else resolveB = next;
      });
    });

    const pendingA = useSessionStore.getState().loadHistory("a");
    const pendingB = useSessionStore.getState().loadHistory("b");
    expect(useSessionStore.getState().selectedSessionId).toBe("b");

    resolveA?.([event("ea", "a")]);
    await pendingA;
    expect(useSessionStore.getState().selectedSessionId).toBe("b");
    expect(useSessionStore.getState().lines.a?.map((line) => line.id)).toEqual(["ea"]);
    expect(useSessionStore.getState().lines.b).toBeUndefined();

    resolveB?.([event("eb", "b")]);
    await pendingB;
    expect(useSessionStore.getState().lines.b?.map((line) => line.id)).toEqual(["eb"]);
  });

  it("hydrates usage from persisted context_usage_json", async () => {
    getLines.mockResolvedValue([]);
    useWorkspaceStore.setState({
      sessions: [
        {
          id: "s1",
          ai_channel_id: null,
          workspace_id: null,
          working_dir: null,
          execution_target: "local",
          ssh_config_id: null,
          target_host_label: null,
          session_kind: "execution",
          status: "exited",
          started_at: "t",
          ended_at: null,
          exit_code: null,
          resume_session_id: null,
          pinned: 0,
          archived: 0,
          input_tokens: null,
          output_tokens: null,
          total_tokens: null,
          reasoning_tokens: null,
          cached_tokens: null,
          created_at: "t",
          context_usage_json: JSON.stringify({
            session_record_id: "s1",
            used_tokens: 28000,
            limit_tokens: 500000,
            generation: 1,
            compactions: 0,
            prompt_tokens: 27000,
            cached_tokens: 22410,
          }),
        } satisfies AgentSession,
      ],
    });

    await useSessionStore.getState().loadHistory("s1");
    expect(useSessionStore.getState().usage.s1).toMatchObject({
      used_tokens: 28000,
      limit_tokens: 500000,
      cached_tokens: 22410,
    });
  });

  it("falls back to the last 用量 line when no snapshot exists", async () => {
    getLines.mockResolvedValue([
      event("e1"),
      {
        ...event("e2"),
        message: "[用量] in=28000 out=120 cache=22410 total=28120",
      },
    ]);

    await useSessionStore.getState().loadHistory("s1");
    expect(useSessionStore.getState().usage.s1).toMatchObject({
      used_tokens: 28000,
      limit_tokens: 128000,
      prompt_tokens: 28000,
      cached_tokens: 22410,
    });
  });

  it("does not overwrite live usage when loading history", async () => {
    useSessionStore.setState({
      usage: {
        s1: {
          session_record_id: "s1",
          used_tokens: 9,
          limit_tokens: 100,
          generation: 2,
          compactions: 0,
        },
      },
    });
    useWorkspaceStore.setState({
      sessions: [
        {
          id: "s1",
          ai_channel_id: null,
          workspace_id: null,
          working_dir: null,
          execution_target: "local",
          ssh_config_id: null,
          target_host_label: null,
          session_kind: "execution",
          status: "running",
          started_at: "t",
          ended_at: null,
          exit_code: null,
          resume_session_id: null,
          pinned: 0,
          archived: 0,
          input_tokens: null,
          output_tokens: null,
          total_tokens: null,
          reasoning_tokens: null,
          cached_tokens: null,
          created_at: "t",
          context_usage_json: JSON.stringify({
            session_record_id: "s1",
            used_tokens: 1,
            limit_tokens: 2,
            generation: 0,
            compactions: 0,
          }),
        } satisfies AgentSession,
      ],
    });
    getLines.mockResolvedValue([event("e1")]);

    await useSessionStore.getState().loadHistory("s1");
    expect(useSessionStore.getState().usage.s1?.used_tokens).toBe(9);
  });

  it("loads earlier history with beforeEventId and prepends to existing lines", async () => {
    // Return 2000 items to trigger hasMoreEarlier: true
    const batch1: AgentSessionEvent[] = Array.from({ length: 2000 }, (_, i) =>
      event(`e${i + 2000}`),
    );
    getLines.mockResolvedValueOnce(batch1);

    await useSessionStore.getState().ensureHistory("s1");
    expect(useSessionStore.getState().hasMoreEarlier.s1).toBe(true);
    expect(useSessionStore.getState().lines.s1?.length).toBe(2000);
    expect(useSessionStore.getState().lines.s1?.[0]?.id).toBe("e2000");

    // Load earlier page before e2000
    const batch2: AgentSessionEvent[] = [event("e100"), event("e101")];
    getLines.mockResolvedValueOnce(batch2);

    const loaded = await useSessionStore.getState().loadEarlierHistory("s1");
    expect(loaded).toBe(true);
    expect(getLines).toHaveBeenLastCalledWith("s1", undefined, 1000, "e2000");
    expect(useSessionStore.getState().lines.s1?.length).toBe(2002);
    expect(useSessionStore.getState().lines.s1?.[0]?.id).toBe("e100");
    expect(useSessionStore.getState().lines.s1?.[1]?.id).toBe("e101");
    expect(useSessionStore.getState().lines.s1?.[2]?.id).toBe("e2000");
    // Since batch2.length < 1000, hasMoreEarlier is now false
    expect(useSessionStore.getState().hasMoreEarlier.s1).toBe(false);
    expect(useSessionStore.getState().loadingEarlier.s1).toBe(false);
  });

  it("keeps reasoning and text fragments until persisted lines cover them", () => {
    const store = useSessionStore.getState();
    store.onDelta({
      session_record_id: "s1",
      kind: "reasoning",
      text: "先看入口",
      clear: false,
    });
    store.onDelta({
      session_record_id: "s1",
      kind: "text",
      text: "正文",
      clear: false,
    });
    expect(useSessionStore.getState().stream.s1?.map((part) => part.kind)).toEqual([
      "reasoning",
      "text",
    ]);
    store.onStdout({
      ...stdout("s1", "think-1"),
      line: "[思考] 8秒\n先看入口",
    });
    expect(useSessionStore.getState().stream.s1?.map((part) => part.kind)).toEqual(["text"]);
    store.onStdout({
      ...stdout("s1", "text-1"),
      line: "正文",
    });
    expect(useSessionStore.getState().stream.s1).toEqual([]);
  });

  it("clears live fragments on retry reset", () => {
    const store = useSessionStore.getState();
    store.onDelta({
      session_record_id: "s1",
      kind: "reasoning",
      text: "旧思考",
      clear: false,
    });
    store.onDelta({
      session_record_id: "s1",
      kind: "text",
      text: "",
      clear: true,
    });
    expect(useSessionStore.getState().stream.s1).toEqual([]);
  });
});
