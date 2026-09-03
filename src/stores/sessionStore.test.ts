import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSessionEvent, AgentSessionOutput } from "@/lib/types";

vi.mock("@/lib/backend", () => ({
  getAgentSessionLogLines: vi.fn(),
}));

import { getAgentSessionLogLines } from "@/lib/backend";
import { useSessionStore } from "./sessionStore";

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
});
