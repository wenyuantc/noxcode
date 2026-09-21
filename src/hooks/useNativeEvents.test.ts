import { beforeEach, describe, expect, it, vi } from "vitest";
import { useNativeEvents } from "./useNativeEvents";
import { useSessionStore } from "@/stores/sessionStore";
import { useSteerStore } from "@/stores/steerStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { useChannelStore } from "@/stores/channelStore";
import { useUiStore } from "@/stores/uiStore";
import { maybeFinishAiMergeResolve, maybeOpenWorktreeMerge } from "@/lib/worktreeMergePrompt";

const callbacks = vi.hoisted(() => new Map<string, (event: never) => unknown>());
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useEffect: (effect: () => void) => effect(),
}));
vi.mock("@/lib/backend", () =>
  Object.fromEntries(
    [
      "onNativeContextUsage",
      "onNativeExit",
      "onNativePermissionRequest",
      "onNativePlanMode",
      "onNativePlanApprovalRequest",
      "onNativePlanQuestion",
      "onNativeSession",
      "onNativeSessionConfiguration",
      "onNativeSessionTitle",
      "onNativeStdout",
      "onNativeTextDelta",
      "onNativeTurnState",
      "onNativeBackgroundTasks",
      "onNativeBackgroundProcesses",
      "onNativeRequestResolved",
      "onNativeInputQueue",
      "onNativeSteer",
    ].map((name) => [
      name,
      (callback: (event: never) => unknown) => {
        callbacks.set(name, callback);
        return Promise.resolve(() => undefined);
      },
    ]),
  ),
);
vi.mock("@/lib/worktreeMergePrompt", () => ({
  maybeFinishAiMergeResolve: vi.fn().mockResolvedValue(false),
  maybeOpenWorktreeMerge: vi.fn().mockResolvedValue(false),
}));

const started = (instance: string) => ({
  session_record_id: "s",
  input_queue_id: instance,
  profile_id: "",
  workspace_id: "w",
  session_kind: "execution",
});
const turn = (instance = "new", turnId = "turn-2", state = "working", revision = 10) => ({
  session_record_id: "s",
  instance_id: instance,
  turn_id: turnId,
  revision,
  state,
});
async function emit(name: string, payload: unknown) {
  await callbacks.get(name)!(payload as never);
  await Promise.resolve();
  await Promise.resolve();
}

function useEventHarness() {
  useNativeEvents();
}

describe("native lifecycle event admission", () => {
  beforeEach(() => {
    callbacks.clear();
    vi.mocked(maybeFinishAiMergeResolve).mockReset().mockResolvedValue(false);
    vi.mocked(maybeOpenWorktreeMerge).mockReset().mockResolvedValue(false);
    useSteerStore.setState(useSteerStore.getInitialState(), true);
    useSessionStore.setState(useSessionStore.getInitialState(), true);
    useWorkspaceStore.setState({
      sessions: [],
      refreshSessions: vi.fn().mockResolvedValue(undefined),
    });
    useEventHarness();
  });

  it("ignores an old runtime idle event before state changes or completion effects", async () => {
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    await emit("onNativeTurnState", turn("old", "old-turn", "waiting_input", 999));
    expect(useSessionStore.getState().turnState.s).toBe("working");
    expect(maybeFinishAiMergeResolve).not.toHaveBeenCalled();
  });

  it("ignores a previous turn idle event in the same runtime", async () => {
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    await emit("onNativeTurnState", turn("new", "turn-1", "waiting_input", 9));
    expect(useSessionStore.getState().turnState.s).toBe("working");
    expect(maybeFinishAiMergeResolve).not.toHaveBeenCalled();
  });

  it("a delayed old exit cannot remove or retire the healthy new runtime", async () => {
    useSessionStore.getState().onStarted(started("old"));
    await emit("onNativePermissionRequest", {
      session_record_id: "s",
      request_id: "old-visible",
      instance_id: "old",
    });
    useSessionStore.getState().onStarted(started("new"));
    expect(useSessionStore.getState().permissions.s["old-visible"]).toBeUndefined();
    await emit("onNativeTurnState", turn());
    await emit("onNativeExit", { ...started("old"), instance_id: "old", code: 0 });
    expect(useSessionStore.getState().liveBySession.s.input_queue_id).toBe("new");
    expect(useSteerStore.getState().retired.s?.new).toBeUndefined();
    useSessionStore.getState().onStarted(started("new"));
    expect(useSessionStore.getState().turnState.s).toBe("working");
    expect(maybeFinishAiMergeResolve).not.toHaveBeenCalled();
  });

  it("a new turn during asynchronous completion prevents a later merge prompt", async () => {
    let finish!: (done: boolean) => void;
    vi.mocked(maybeFinishAiMergeResolve).mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    const completing = emit("onNativeTurnState", turn("new", "turn-2", "waiting_input", 11));
    await Promise.resolve();
    await emit("onNativeTurnState", turn("new", "turn-3", "working", 12));
    finish(false);
    await completing;
    expect(maybeOpenWorktreeMerge).not.toHaveBeenCalled();
  });

  it("resolved requests cannot be resurrected by delayed request delivery", async () => {
    for (const [kind, event, key] of [
      ["permission", "onNativePermissionRequest", "permissions"],
      ["question", "onNativePlanQuestion", "planQuestions"],
      ["plan_approval", "onNativePlanApprovalRequest", "planApprovals"],
    ] as const) {
      await emit("onNativeRequestResolved", { session_record_id: "s", request_id: kind, kind });
      await emit(event, { session_record_id: "s", request_id: kind });
      expect(useSessionStore.getState()[key].s?.[kind]).toBeUndefined();
    }
  });
  it("admits valid transitions once and orders idle compaction within the completed turn", async () => {
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    const idle = turn("new", "turn-2", "waiting_input", 11);
    await emit("onNativeTurnState", idle);
    await emit("onNativeTurnState", idle);
    expect(maybeFinishAiMergeResolve).toHaveBeenCalledTimes(1);
    await emit("onNativeTurnState", turn("new", "turn-2", "working", 12));
    await emit("onNativeTurnState", idle);
    expect(useSessionStore.getState().turnState.s).toBe("working");
    await emit("onNativeTurnState", turn("new", "turn-2", "waiting_input", 13));
    expect(useSessionStore.getState().turnState.s).toBe("waiting_input");
    expect(maybeFinishAiMergeResolve).toHaveBeenCalledTimes(2);
  });

  it("hydrates a missed turn event and rejects older lifecycle snapshots and broadcasts", async () => {
    const lifecycle = turn("new", "turn-3", "working", 20);
    await emit("onNativeSteer", {
      session_record_id: "s",
      instance_id: "new",
      turn_id: "turn-3",
      revision: 20,
      lifecycle,
      receipts: [],
    });
    await emit("onNativeSession", started("new"));
    expect(useSessionStore.getState().turnState.s).toBe("working");
    await emit("onNativeTurnState", turn("new", "turn-2", "waiting_input", 19));
    await emit("onNativeSteer", {
      session_record_id: "s",
      instance_id: "new",
      turn_id: "turn-2",
      revision: 18,
      receipts: [],
    });
    expect(useSteerStore.getState().snapshots.s.turn_id).toBe("turn-3");
    expect(maybeFinishAiMergeResolve).not.toHaveBeenCalled();
  });

  it("ignores stale live text/clear while preserving committed historical and child output", async () => {
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    await emit("onNativeTextDelta", {
      session_record_id: "s",
      instance_id: "new",
      turn_id: "turn-2",
      kind: "text",
      text: "current",
      clear: false,
    });
    const stream = useSessionStore.getState().stream.s;
    for (const [instance, turnId] of [
      ["old", "old-turn"],
      ["new", "turn-1"],
    ]) {
      await emit("onNativeTextDelta", {
        session_record_id: "s",
        instance_id: instance,
        turn_id: turnId,
        kind: "text",
        text: "",
        clear: true,
      });
      await emit("onNativeTextDelta", {
        session_record_id: "s",
        instance_id: instance,
        turn_id: turnId,
        kind: "text",
        text: "stale",
        clear: false,
      });
    }
    expect(useSessionStore.getState().stream.s).toBe(stream);
    await emit("onNativeStdout", {
      session_record_id: "s",
      session_event_id: "historical",
      profile_id: "",
      workspace_id: "w",
      session_kind: "execution",
      line: "child report",
      assistant: { chain_id: "old-child", part: 0, subagent_tag: "[子 Agent 1(general) - report]" },
    });
    expect(useSessionStore.getState().lines.s.some((line) => line.text === "child report")).toBe(
      true,
    );
  });

  it("rejects old-runtime interactions but retains a valid child permission across main turns", async () => {
    useSessionStore.getState().onStarted(started("new"));
    await emit("onNativeTurnState", turn());
    await emit("onNativePermissionRequest", {
      session_record_id: "s",
      request_id: "old",
      instance_id: "old",
    });
    await emit("onNativePermissionRequest", {
      session_record_id: "s",
      request_id: "child",
      instance_id: "new",
    });
    await emit("onNativeTurnState", turn("new", "turn-3", "working", 12));
    expect(useSessionStore.getState().permissions.s.old).toBeUndefined();
    expect(useSessionStore.getState().permissions.s.child).toBeDefined();
    await emit("onNativeExit", { ...started("old"), instance_id: "old", code: 0 });
    expect(useSessionStore.getState().permissions.s.child).toBeDefined();
  });

  it("rechecks a valid exit after refresh before acting on a newly started runtime", async () => {
    let refresh!: () => void;
    useWorkspaceStore.setState({
      refreshSessions: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            refresh = resolve;
          }),
      ),
    });
    useSessionStore.getState().onStarted(started("old"));
    const exiting = emit("onNativeExit", { ...started("old"), instance_id: "old", code: 0 });
    useSessionStore.getState().onStarted(started("new"));
    refresh();
    await exiting;
    expect(useSessionStore.getState().liveBySession.s.input_queue_id).toBe("new");
    expect(maybeFinishAiMergeResolve).not.toHaveBeenCalled();
    expect(maybeOpenWorktreeMerge).not.toHaveBeenCalled();
  });
  it("rejected configuration events cannot change global model or effort selection", async () => {
    useSessionStore.getState().onStarted(started("new"));
    useSessionStore.getState().selectSession("s");
    const runtime = {
      ai_channel_id: "new-channel",
      model: "new-model",
      reasoning_effort: "high",
      permission_mode: "default",
      plan_mode: false,
    };
    await emit("onNativeSessionConfiguration", {
      session_record_id: "s",
      input_queue_id: "new",
      request_id: "current",
      revision: 2,
      runtime,
    });
    for (const [instance, revision] of [
      ["old", 99],
      ["new", 1],
    ]) {
      await emit("onNativeSessionConfiguration", {
        session_record_id: "s",
        input_queue_id: instance,
        request_id: "stale",
        revision,
        runtime: {
          ...runtime,
          ai_channel_id: "old-channel",
          model: "old-model",
          reasoning_effort: "low",
        },
      });
      expect(useChannelStore.getState().activeChannelId).toBe("new-channel");
      expect(useChannelStore.getState().activeModelId).toBe("new-model");
      expect(useUiStore.getState().composerThinkingLevel).toBe("high");
    }
  });
});
