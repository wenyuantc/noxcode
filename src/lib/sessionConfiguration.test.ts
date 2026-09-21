import { beforeEach, describe, expect, it, vi } from "vitest";
import { finishNativeInput, updateNativeSessionConfiguration } from "./backend";
import {
  changeSessionConfiguration,
  SESSION_CONFIGURATION_SUPERSEDED,
} from "./sessionConfiguration";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSteerStore } from "@/stores/steerStore";
import { useUiStore } from "@/stores/uiStore";
import { handleNativeExit } from "@/lib/nativeLifecycle";
import { maybeFinishAiMergeResolve, maybeOpenWorktreeMerge } from "@/lib/worktreeMergePrompt";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import type {
  AgentSession,
  AgentSessionStarted,
  NativeSessionConfigurationEvent,
  NativeSessionRuntime,
} from "./types";

vi.mock("./backend", () => ({
  finishNativeInput: vi.fn(),
  getAgentSessionLogLines: vi.fn(),
  updateNativeSessionConfiguration: vi.fn(),
}));

vi.mock("@/lib/worktreeMergePrompt", () => ({
  maybeFinishAiMergeResolve: vi.fn().mockResolvedValue(false),
  maybeOpenWorktreeMerge: vi.fn().mockResolvedValue(false),
}));

const runtime: NativeSessionRuntime = {
  ai_channel_id: "channel",
  model: "model",
  reasoning_effort: "high",
  permission_mode: "default",
  plan_mode: false,
};
const live: AgentSessionStarted = {
  profile_id: "p",
  workspace_id: "ws",
  session_kind: "execution",
  session_record_id: "s1",
  runtime,
};

const session = (id: string, model: string | null): AgentSession => ({
  id,
  ai_channel_id: "old-ch",
  workspace_id: "ws",
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
  model,
});

function configurationEvent(
  overrides: Partial<NativeSessionConfigurationEvent> = {},
): NativeSessionConfigurationEvent {
  return {
    session_record_id: "s1",
    request_id: "req-1",
    revision: 1,
    runtime: { ...runtime, model: "new" },
    compacted: false,
    ...overrides,
  };
}

describe("session configuration", () => {
  beforeEach(() => {
    useSteerStore.setState(useSteerStore.getInitialState(), true);
    useWorkspaceStore.setState({
      sessions: [],
      refreshSessions: vi.fn().mockResolvedValue(undefined),
    });
    vi.mocked(maybeFinishAiMergeResolve).mockClear();
    vi.mocked(maybeOpenWorktreeMerge).mockClear();
    vi.mocked(finishNativeInput).mockReset();
    vi.mocked(updateNativeSessionConfiguration).mockReset();
    useSessionStore.setState({
      liveBySession: { s1: live },
      configurationBySession: { s1: runtime, s2: { ...runtime, model: "other" } },
      pendingConfigurationBySession: {},
      configurationRevisionBySession: {},
      turnState: { s1: "waiting_input" },
      permissions: {},
      planQuestions: {},
      planApprovals: {},
    });
  });
  it("waits for graceful exit before changing permission on the selected session", async () => {
    let finish!: () => void;
    vi.mocked(finishNativeInput).mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const pending = changeSessionConfiguration("s1", { permission_mode: "edit" });
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
    finish();
    await pending;
    expect(useSessionStore.getState().liveBySession.s1).toBeUndefined();
    expect(useSessionStore.getState().configurationBySession.s1).toEqual({
      ...runtime,
      permission_mode: "edit",
    });
    expect(useSessionStore.getState().configurationBySession.s2.model).toBe("other");
  });
  it("rejects permission changes while a turn is working", async () => {
    useSessionStore.setState({ turnState: { s1: "working" } });
    await expect(changeSessionConfiguration("s1", { plan_mode: true })).rejects.toThrow("当前任务");
    expect(finishNativeInput).not.toHaveBeenCalled();
    expect(updateNativeSessionConfiguration).not.toHaveBeenCalled();
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
  });
  it("preserves the live instance and displayed configuration when finishing fails", async () => {
    vi.mocked(finishNativeInput).mockRejectedValue(new Error("finish failed"));
    await expect(changeSessionConfiguration("s1", { reasoning_effort: "low" })).rejects.toThrow(
      "finish failed",
    );
    expect(useSessionStore.getState().liveBySession.s1).toEqual(live);
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
  });
  it("creates configuration for a historical session without touching others", async () => {
    useSessionStore.setState({
      liveBySession: {},
      configurationBySession: { s2: { ...runtime, model: "other" } },
      turnState: {},
      planModeBySession: {},
    });
    useWorkspaceStore.setState({ sessions: [session("s3", "old-model")] });
    useChannelStore.setState({ activeChannelId: "global-ch", activeModelId: "global-model" });
    await changeSessionConfiguration("s3", { model: "new" });
    expect(useSessionStore.getState().configurationBySession.s3).toEqual({
      ai_channel_id: "old-ch",
      model: "new",
      reasoning_effort: null,
      permission_mode: "default",
      plan_mode: false,
    });
    expect(useSessionStore.getState().configurationBySession.s2.model).toBe("other");
    expect(finishNativeInput).not.toHaveBeenCalled();
    expect(updateNativeSessionConfiguration).not.toHaveBeenCalled();
  });
  it("queues a live model switch without ending the current turn", async () => {
    useSessionStore.setState({ turnState: { s1: "working" }, selectedSessionId: "s1" });
    let resolveConfig!: (value: NativeSessionConfigurationEvent) => void;
    vi.mocked(updateNativeSessionConfiguration).mockReturnValue(
      new Promise((resolve) => {
        resolveConfig = resolve;
      }),
    );
    const pending = changeSessionConfiguration("s1", { model: "new" });
    expect(finishNativeInput).not.toHaveBeenCalled();
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
    expect(useSessionStore.getState().liveBySession.s1).toEqual(live);
    const queued = useSessionStore.getState().pendingConfigurationBySession.s1;
    expect(queued).toMatchObject({ ai_channel_id: "channel", model: "new" });
    expect(queued?.request_id).toBeTruthy();
    resolveConfig(
      configurationEvent({
        request_id: queued!.request_id,
        runtime: { ...runtime, model: "new" },
      }),
    );
    await pending;
    const state = useSessionStore.getState();
    expect(state.liveBySession.s1.session_record_id).toBe("s1");
    expect(state.liveBySession.s1.runtime?.model).toBe("new");
    expect(state.configurationBySession.s1.model).toBe("new");
    expect(state.pendingConfigurationBySession.s1).toBeUndefined();
    expect(useChannelStore.getState().activeModelId).toBe("new");
  });
  it("rolls back pending configuration when the live switch fails", async () => {
    useSessionStore.setState({ turnState: { s1: "working" }, selectedSessionId: "s1" });
    vi.mocked(updateNativeSessionConfiguration).mockRejectedValue(new Error("channel down"));
    await expect(changeSessionConfiguration("s1", { model: "new" })).rejects.toThrow(
      "channel down",
    );
    const state = useSessionStore.getState();
    expect(state.configurationBySession.s1).toEqual(runtime);
    expect(state.pendingConfigurationBySession.s1).toBeUndefined();
    expect(state.liveBySession.s1).toEqual(live);
  });
  it("ignores a superseded model switch without clearing a newer pending request", async () => {
    useSessionStore.setState({ turnState: { s1: "working" }, selectedSessionId: "s1" });
    let rejectFirst!: (reason: Error) => void;
    let resolveSecond!: (value: NativeSessionConfigurationEvent) => void;
    vi.mocked(updateNativeSessionConfiguration)
      .mockReturnValueOnce(
        new Promise((_, reject) => {
          rejectFirst = reject;
        }),
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveSecond = resolve;
        }),
      );
    const first = changeSessionConfiguration("s1", { model: "first" });
    const second = changeSessionConfiguration("s1", { model: "second" });
    const newer = useSessionStore.getState().pendingConfigurationBySession.s1;
    expect(newer?.model).toBe("second");
    rejectFirst(new Error(SESSION_CONFIGURATION_SUPERSEDED));
    await first;
    expect(useSessionStore.getState().pendingConfigurationBySession.s1).toEqual(newer);
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
    resolveSecond(
      configurationEvent({
        request_id: newer!.request_id,
        runtime: { ...runtime, model: "second" },
      }),
    );
    await second;
    expect(useSessionStore.getState().configurationBySession.s1.model).toBe("second");
    expect(useSessionStore.getState().pendingConfigurationBySession.s1).toBeUndefined();
  });
  it.each(["old-runtime", "superseded-revision"])(
    "does not apply a delayed %s IPC result to global selection",
    async (kind) => {
      useSessionStore.getState().onStarted({ ...live, input_queue_id: "old" });
      useSessionStore.setState({ selectedSessionId: "s1" });
      let resolve!: (event: NativeSessionConfigurationEvent) => void;
      vi.mocked(updateNativeSessionConfiguration).mockReturnValue(
        new Promise((done) => {
          resolve = done;
        }),
      );
      const changing = changeSessionConfiguration("s1", { model: "requested" });
      const requestId = useSessionStore.getState().pendingConfigurationBySession.s1.request_id;
      const latest = {
        ...runtime,
        ai_channel_id: "latest-channel",
        model: "latest-model",
        reasoning_effort: "high",
      };
      if (kind === "old-runtime") {
        useSessionStore.getState().onStarted({ ...live, input_queue_id: "new", runtime: latest });
      } else {
        useSessionStore.getState().onConfiguration(
          configurationEvent({
            input_queue_id: "old",
            request_id: "newer",
            revision: 2,
            runtime: latest,
          }),
        );
      }
      useChannelStore.getState().setSelection(latest.ai_channel_id, latest.model);
      useUiStore.getState().setComposerThinkingLevel("high");
      resolve(
        configurationEvent({
          input_queue_id: "old",
          request_id: requestId,
          revision: 1,
          runtime: { ...runtime, model: "stale-model", reasoning_effort: "low" },
        }),
      );
      expect(await changing).toBeUndefined();
      expect(useChannelStore.getState().activeChannelId).toBe("latest-channel");
      expect(useChannelStore.getState().activeModelId).toBe("latest-model");
      expect(useUiStore.getState().composerThinkingLevel).toBe("high");
      expect(useSessionStore.getState().pendingConfigurationBySession.s1).toBeUndefined();
    },
  );

  it("returns success when the equivalent configuration event beat its IPC response", async () => {
    useSessionStore.getState().onStarted({ ...live, input_queue_id: "current" });
    useSessionStore.setState({ selectedSessionId: "s1" });
    let resolve!: (event: NativeSessionConfigurationEvent) => void;
    vi.mocked(updateNativeSessionConfiguration).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const changing = changeSessionConfiguration("s1", { model: "new" });
    const requestId = useSessionStore.getState().pendingConfigurationBySession.s1.request_id;
    const event = configurationEvent({ input_queue_id: "current", request_id: requestId });
    expect(useSessionStore.getState().onConfiguration(event)).toBe(true);
    useChannelStore.getState().setSelection(event.runtime!.ai_channel_id, event.runtime!.model);
    resolve(event);
    expect(await changing).toEqual(event);
    expect(useSessionStore.getState().pendingConfigurationBySession.s1).toBeUndefined();
    expect(useChannelStore.getState().activeModelId).toBe("new");
  });

  it.each(["synthetic-first", "broadcast-first"])(
    "handles %s exit completion effects exactly once",
    async (ordering) => {
      useSessionStore.getState().onStarted({
        ...live,
        input_queue_id: "current",
        runtime: { ...runtime, worktree_path: "/cfg/worktrees/s1" },
      });
      useSessionStore.setState({ turnState: { s1: "waiting_input" } });
      const exit = { ...live, instance_id: "current", worktree_path: "/cfg/worktrees/s1", code: 0 };
      vi.mocked(finishNativeInput).mockImplementation(async () => {
        if (ordering === "broadcast-first") await handleNativeExit(exit);
      });
      await changeSessionConfiguration("s1", { permission_mode: "edit" });
      if (ordering === "synthetic-first") await handleNativeExit(exit);
      await handleNativeExit(exit);
      expect(maybeFinishAiMergeResolve).toHaveBeenCalledTimes(1);
      expect(maybeOpenWorktreeMerge).toHaveBeenCalledTimes(1);
      expect(maybeOpenWorktreeMerge).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: "ws",
          worktreePath: "/cfg/worktrees/s1",
          reason: "exit",
        }),
      );
      expect(useWorkspaceStore.getState().refreshSessions).toHaveBeenCalledTimes(1);
    },
  );
  it("synthetic exit retains workspace and worktree fallback captured before finishing", async () => {
    useSessionStore.getState().onStarted({ ...live, workspace_id: "", input_queue_id: "current" });
    useSessionStore.setState({ turnState: { s1: "waiting_input" } });
    useWorkspaceStore.setState({
      sessions: [
        {
          ...session("s1", "model"),
          workspace_id: "fallback-ws",
          working_dir: "/fallback/worktrees/s1",
        },
      ],
    });
    vi.mocked(finishNativeInput).mockImplementation(async () => {
      useWorkspaceStore.setState({ sessions: [] });
    });
    await changeSessionConfiguration("s1", { permission_mode: "edit" });
    expect(maybeFinishAiMergeResolve).toHaveBeenCalledWith(
      expect.objectContaining({ workspaceId: "fallback-ws" }),
    );
    expect(maybeOpenWorktreeMerge).toHaveBeenCalledWith(
      expect.objectContaining({
        workspaceId: "fallback-ws",
        worktreePath: "/fallback/worktrees/s1",
      }),
    );
  });

  it("a rejected IPC result does not clear a newer pending configuration", async () => {
    useSessionStore.getState().onStarted({ ...live, input_queue_id: "current" });
    let resolve!: (event: NativeSessionConfigurationEvent) => void;
    vi.mocked(updateNativeSessionConfiguration).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const changing = changeSessionConfiguration("s1", { model: "old-request" });
    const oldRequest = useSessionStore.getState().pendingConfigurationBySession.s1.request_id;
    useSessionStore
      .getState()
      .onConfiguration(
        configurationEvent({ input_queue_id: "current", revision: 2, request_id: "applied-newer" }),
      );
    const pending = { request_id: "still-newer", ai_channel_id: "channel", model: "pending-model" };
    useSessionStore.getState().setPendingConfiguration("s1", pending);
    resolve(configurationEvent({ input_queue_id: "current", revision: 1, request_id: oldRequest }));
    expect(await changing).toBeUndefined();
    expect(useSessionStore.getState().pendingConfigurationBySession.s1).toEqual(pending);
  });
});
