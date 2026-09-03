import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSession, AgentSessionEvent, AgentSessionOutput } from "@/lib/types";

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

describe("sessionStore history", () => {
  beforeEach(() => {
    getLines.mockReset();
    useSessionStore.setState({
      selectedSessionId: null,
      liveBySession: {},
      lines: {},
      turnState: {},
      usage: {},
      stream: {},
      permission: null,
      planQuestion: null,
    });
    useWorkspaceStore.setState({ sessions: [] });
    useChannelStore.setState({
      channels: [],
      activeChannelId: null,
      activeModelId: null,
    });
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

  it("skips fetch when history is already cached", async () => {
    useSessionStore.setState({
      lines: { s1: [{ id: "cached", sessionId: "s1", text: "cached", createdAt: "t" }] },
    });
    await useSessionStore.getState().loadHistory("s1");
    expect(getLines).not.toHaveBeenCalled();
    expect(useSessionStore.getState().selectedSessionId).toBe("s1");
    expect(useSessionStore.getState().lines.s1?.[0]?.id).toBe("cached");
  });

  it("does not overwrite lines filled while fetch is in flight", async () => {
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

    expect(useSessionStore.getState().lines.s1?.map((line) => line.id)).toEqual(["live"]);
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
});
