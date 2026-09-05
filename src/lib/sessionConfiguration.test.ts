import { beforeEach, describe, expect, it, vi } from "vitest";
import { finishNativeInput } from "./backend";
import { changeSessionConfiguration } from "./sessionConfiguration";
import { useSessionStore } from "@/stores/sessionStore";
import type { AgentSessionStarted, NativeSessionRuntime } from "./types";

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
});
