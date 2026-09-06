import { describe, expect, it } from "vitest";

import { isSessionBusy, mergeSessions, resolveSessionDirectory } from "./sessionActions";
import type { AgentSession, Workspace } from "./types";

const session: AgentSession = {
  id: "session",
  ai_channel_id: null,
  workspace_id: "ws",
  working_dir: null,
  execution_target: "local",
  ssh_config_id: null,
  target_host_label: null,
  session_kind: "execution",
  status: "exited",
  started_at: "2026-09-06",
  ended_at: null,
  exit_code: null,
  resume_session_id: null,
  pinned: 0,
  archived: 0,
  title: "任务",
  input_tokens: null,
  output_tokens: null,
  total_tokens: null,
  reasoning_tokens: null,
  cached_tokens: null,
  created_at: "2026-09-06",
};
const workspace: Workspace = {
  id: "ws",
  name: "Project",
  workspace_type: "local",
  repo_path: "/project",
  ssh_config_id: null,
  remote_repo_path: "/remote",
  created_at: "",
  updated_at: "",
};

describe("resolveSessionDirectory", () => {
  it("uses the target session's directory rather than another active project", () => {
    expect(
      resolveSessionDirectory({ ...session, working_dir: "/session project" }, [workspace]),
    ).toEqual({ path: "/session project", remote: false });
    expect(resolveSessionDirectory(session, [{ ...workspace, id: "other" }])).toEqual({
      path: null,
      remote: false,
    });
    expect(resolveSessionDirectory(session, [workspace]).path).toBe("/project");
  });
  it("supports Windows paths without rewriting them", () => {
    expect(resolveSessionDirectory({ ...session, working_dir: "C:\\Work\\项目" }, []).path).toBe(
      "C:\\Work\\项目",
    );
  });
  it("resolves remote paths without treating them as local directories", () => {
    expect(
      resolveSessionDirectory({ ...session, execution_target: "ssh" }, [
        { ...workspace, workspace_type: "ssh" },
      ]),
    ).toEqual({ path: "/remote", remote: true });
    expect(
      resolveSessionDirectory(session, [{ ...workspace, workspace_type: "ssh" }]).path,
    ).toBeNull();
  });
  it("does not copy blank paths", () => {
    expect(
      resolveSessionDirectory({ ...session, working_dir: "  " }, [{ ...workspace, repo_path: " " }])
        .path,
    ).toBeNull();
  });
});

function activity() {
  return {
    turnState: {},
    liveBySession: {},
    backgroundBySession: {},
    inputQueueBySession: {},
    permissions: {},
    planQuestions: {},
    planApprovals: {},
  };
}

describe("isSessionBusy", () => {
  it("allows idle and exited sessions but rejects working or not-yet-hydrated live sessions", () => {
    expect(isSessionBusy("session", activity())).toBe(false);
    expect(
      isSessionBusy("session", {
        ...activity(),
        turnState: { session: "waiting_input" },
        liveBySession: { session: {} },
      }),
    ).toBe(false);
    expect(isSessionBusy("session", { ...activity(), turnState: { session: "working" } })).toBe(
      true,
    );
    expect(isSessionBusy("session", { ...activity(), liveBySession: { session: {} } })).toBe(true);
  });
  it("rejects queued inputs, pending requests and background tasks", () => {
    expect(
      isSessionBusy("session", {
        ...activity(),
        inputQueueBySession: {
          session: {
            session_record_id: "session",
            queue_id: "q",
            revision: 1,
            items: [{ id: "i", text: "queued", image_count: 0, editing: false }],
          },
        },
      }),
    ).toBe(true);
    for (const field of ["permissions", "planQuestions", "planApprovals"]) {
      expect(
        isSessionBusy("session", { ...activity(), [field]: { session: { request: {} } } }),
      ).toBe(true);
    }
    for (const status of ["queued", "running", "done", "failed", "stopped"] as const) {
      expect(
        isSessionBusy("session", {
          ...activity(),
          backgroundBySession: {
            session: [
              { task_id: "task", description: "task", kind: "agent", status, report: null },
            ],
          },
        }),
      ).toBe(status === "queued" || status === "running");
    }
  });
});

it("merges refreshed records by ID without dropping cached archive metadata", () => {
  const archive = { ...session, id: "archived", archived: 1 };
  const renamed = { ...session, title: "新名称" };
  expect(mergeSessions([session, archive], [renamed])).toEqual([renamed, archive]);
});
