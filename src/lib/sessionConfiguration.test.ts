import { beforeEach, describe, expect, it, vi } from "vitest";
import { finishNativeInput } from "./backend";
import { changeSessionConfiguration } from "./sessionConfiguration";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import type { AgentSession, AgentSessionStarted, NativeSessionRuntime } from "./types";

vi.mock("./backend", () => ({ finishNativeInput: vi.fn(), getAgentSessionLogLines: vi.fn() }));

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

describe("session configuration", () => {
  beforeEach(() => {
    vi.mocked(finishNativeInput).mockReset();
    useSessionStore.setState({
      liveBySession: { s1: live },
      configurationBySession: { s1: runtime, s2: { ...runtime, model: "other" } },
      turnState: { s1: "waiting_input" },
      permissions: {},
      planQuestions: {},
      planApprovals: {},
    });
  });
  it("waits for graceful exit before changing only the selected session", async () => {
    let finish!: () => void;
    vi.mocked(finishNativeInput).mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const pending = changeSessionConfiguration("s1", { model: "new", permission_mode: "edit" });
    expect(useSessionStore.getState().configurationBySession.s1).toEqual(runtime);
    finish();
    await pending;
    expect(useSessionStore.getState().liveBySession.s1).toBeUndefined();
    expect(useSessionStore.getState().configurationBySession.s1).toEqual({
      ...runtime,
      model: "new",
      permission_mode: "edit",
    });
    expect(useSessionStore.getState().configurationBySession.s2.model).toBe("other");
  });
  it("rejects configuration changes while a turn is working", async () => {
    useSessionStore.setState({ turnState: { s1: "working" } });
    await expect(changeSessionConfiguration("s1", { plan_mode: true })).rejects.toThrow("当前任务");
    expect(finishNativeInput).not.toHaveBeenCalled();
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
  });
});
