import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend", () => ({
  checkWorkspaceHealth: vi.fn(),
  createWorkspace: vi.fn(),
  deleteWorkspace: vi.fn(),
  listAgentSessions: vi.fn(),
  listWorkspaces: vi.fn(),
  renameAgentSession: vi.fn(),
  setAgentSessionArchived: vi.fn(),
  setAgentSessionPinned: vi.fn(),
  updateWorkspace: vi.fn(),
  getAgentSessionLogLines: vi.fn(),
}));

import { listAgentSessions, renameAgentSession, setAgentSessionArchived } from "@/lib/backend";
import type { AgentSession } from "@/lib/types";
import { useSessionStore } from "./sessionStore";
import { useWorkspaceStore } from "./workspaceStore";

function session(id: string, archived = 0): AgentSession {
  return {
    id,
    archived,
    ai_channel_id: null,
    workspace_id: "ws",
    working_dir: "/project",
    execution_target: "local",
    ssh_config_id: null,
    target_host_label: null,
    session_kind: "execution",
    status: "exited",
    started_at: "2026-09-06",
    ended_at: null,
    exit_code: null,
    resume_session_id: null,
    pinned: 1,
    title: id,
    input_tokens: null,
    output_tokens: null,
    total_tokens: null,
    reasoning_tokens: null,
    cached_tokens: null,
    created_at: "2026-09-06",
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetAllMocks();
  useWorkspaceStore.setState(useWorkspaceStore.getInitialState(), true);
  useSessionStore.setState(useSessionStore.getInitialState(), true);
  vi.mocked(listAgentSessions).mockResolvedValue([]);
  vi.mocked(renameAgentSession).mockImplementation(async (id, title) => ({
    ...session(id),
    title,
  }));
  vi.mocked(setAgentSessionArchived).mockImplementation(async (id, archived) =>
    session(id, archived ? 1 : 0),
  );
});

describe("workspace session actions", () => {
  it("loads archive pages separately and preserves selected archive metadata during refresh", async () => {
    const archived = Array.from({ length: 51 }, (_, index) => session(`archive-${index}`, 1));
    vi.mocked(listAgentSessions).mockImplementation(
      async (_workspace, limit = 50, archive = false, offset = 0) =>
        archive ? archived.slice(offset, offset + limit) : [session("active")],
    );
    await useWorkspaceStore.getState().loadArchivedSessions();
    expect(useWorkspaceStore.getState().archivedSessionIds).toHaveLength(50);
    expect(useWorkspaceStore.getState().archivedHasMore).toBe(true);
    await useWorkspaceStore.getState().loadArchivedSessions(true);
    expect(useWorkspaceStore.getState().archivedSessionIds).toHaveLength(51);
    expect(useWorkspaceStore.getState().archivedHasMore).toBe(false);
    useSessionStore.getState().selectSession("archive-50");
    await useWorkspaceStore.getState().refreshSessions();
    expect(useWorkspaceStore.getState().sessions).toHaveLength(52);
    expect(useSessionStore.getState().selectedSessionId).toBe("archive-50");
  });

  it("renames the target without changing the selection and retains the full title", async () => {
    useWorkspaceStore.setState({ sessions: [session("target"), session("selected")] });
    useSessionStore.getState().selectSession("selected");
    const title = "完整任务标题".repeat(20);
    await useWorkspaceStore.getState().renameSession("target", `  ${title}  `);
    expect(useWorkspaceStore.getState().sessions.find((item) => item.id === "target")?.title).toBe(
      title,
    );
    expect(useSessionStore.getState().selectedSessionId).toBe("selected");
  });

  it("does not let a stale list overwrite a successful rename", async () => {
    useWorkspaceStore.setState({ sessions: [session("target")] });
    const stale = deferred<AgentSession[]>();
    vi.mocked(listAgentSessions).mockReturnValueOnce(stale.promise);
    const refresh = useWorkspaceStore.getState().refreshSessions();
    await useWorkspaceStore.getState().renameSession("target", "new");
    stale.resolve([session("target")]);
    await refresh;
    expect(useWorkspaceStore.getState().sessions[0]?.title).toBe("new");
  });

  it("preserves the record and clears pending state on failed mutations", async () => {
    useWorkspaceStore.setState({ sessions: [session("target")] });
    vi.mocked(renameAgentSession).mockRejectedValueOnce(new Error("save failed"));
    await expect(useWorkspaceStore.getState().renameSession("target", "new")).rejects.toThrow(
      "save failed",
    );
    expect(useWorkspaceStore.getState().sessions[0]?.title).toBe("target");
    expect(useWorkspaceStore.getState().sessionMutations).toEqual({});
  });

  it("archives the current session to the empty view without losing pin or history", async () => {
    useWorkspaceStore.setState({ sessions: [session("target")] });
    useSessionStore.getState().selectSession("target");
    useSessionStore.setState({ historyLoaded: { target: true } });
    await useWorkspaceStore.getState().setSessionArchived("target", true);
    expect(useSessionStore.getState().selectedSessionId).toBeNull();
    expect(useSessionStore.getState().historyLoaded.target).toBe(true);
    expect(useWorkspaceStore.getState().sessions[0]).toMatchObject({ archived: 1, pinned: 1 });
    useSessionStore.getState().selectSession("target");
    await useWorkspaceStore.getState().setSessionArchived("target", false);
    expect(useSessionStore.getState().selectedSessionId).toBe("target");
    expect(useWorkspaceStore.getState().sessions[0]?.archived).toBe(0);
  });

  it("does not clear another selection or clear early on a duplicate pending archive", async () => {
    useWorkspaceStore.setState({ sessions: [session("target"), session("other")] });
    useSessionStore.getState().selectSession("target");
    const pending = deferred<AgentSession>();
    vi.mocked(setAgentSessionArchived).mockReturnValueOnce(pending.promise);
    const first = useWorkspaceStore.getState().setSessionArchived("target", true);
    await useWorkspaceStore.getState().setSessionArchived("target", true);
    expect(setAgentSessionArchived).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().selectedSessionId).toBe("target");
    useSessionStore.getState().selectSession("other");
    pending.resolve(session("target", 1));
    await first;
    expect(useSessionStore.getState().selectedSessionId).toBe("other");
  });

  it("can retry a failed archive load without discarding cached records", async () => {
    useWorkspaceStore.setState({ sessions: [session("cached", 1)] });
    vi.mocked(listAgentSessions).mockRejectedValueOnce(new Error("offline"));
    await useWorkspaceStore.getState().loadArchivedSessions();
    expect(useWorkspaceStore.getState().archivedError).toBe("offline");
    expect(useWorkspaceStore.getState().sessions[0]?.id).toBe("cached");
    vi.mocked(listAgentSessions).mockResolvedValueOnce([session("cached", 1)]);
    await useWorkspaceStore.getState().loadArchivedSessions();
    expect(useWorkspaceStore.getState().archivedError).toBeNull();
    expect(useWorkspaceStore.getState().archivedSessionIds).toEqual(["cached"]);
  });

  it("keeps a newly archived task in an already expanded archive group", async () => {
    useWorkspaceStore.setState({ sessions: [session("target")], archivedLoaded: true });
    vi.mocked(listAgentSessions).mockImplementation(async (_workspace, _limit, archived) =>
      archived ? [session("target", 1)] : [],
    );
    await useWorkspaceStore.getState().setSessionArchived("target", true);
    await vi.waitFor(() => {
      expect(useWorkspaceStore.getState().archivedSessionIds).toEqual(["target"]);
    });
    expect(useWorkspaceStore.getState().sessions[0]?.archived).toBe(1);
  });

  it("ignores an archive page that arrives after its task was restored", async () => {
    useWorkspaceStore.setState({
      sessions: [session("target", 1)],
      archivedSessionIds: ["target"],
      archivedLoaded: true,
    });
    useSessionStore.getState().selectSession("target");
    const stale = deferred<AgentSession[]>();
    vi.mocked(listAgentSessions).mockReturnValueOnce(stale.promise);
    const loading = useWorkspaceStore.getState().loadArchivedSessions();
    await useWorkspaceStore.getState().setSessionArchived("target", false);
    stale.resolve([session("target", 1)]);
    await loading;
    expect(useWorkspaceStore.getState().archivedSessionIds).toEqual([]);
    expect(useWorkspaceStore.getState().sessions[0]?.archived).toBe(0);
    expect(useSessionStore.getState().selectedSessionId).toBe("target");
    expect(useWorkspaceStore.getState().archivedLoading).toBe(false);
  });
});
