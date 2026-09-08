import { beforeEach, describe, expect, it, vi } from "vitest";
import { finishNativeInput, updateNativeSessionConfiguration } from "./backend";
import {
  changeSessionConfiguration,
  SESSION_CONFIGURATION_SUPERSEDED,
} from "./sessionConfiguration";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
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
});
