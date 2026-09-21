import { describe, expect, it, vi } from "vitest";

import {
  parsePendingPlan,
  authorizedPlanRetry,
  parseApprovedPlan,
  submitPlanApproval,
} from "./planApproval";
import type { AgentSession, AgentSessionStarted, ApprovedPlanSnapshot } from "./types";
import { planApprovalModelArgs } from "./sessionModel";

function session(pendingPlanJson?: string | null): AgentSession {
  return {
    id: "s1",
    ai_channel_id: "ch",
    workspace_id: "ws-1",
    working_dir: null,
    execution_target: "local",
    ssh_config_id: null,
    target_host_label: null,
    session_kind: "plan",
    status: "exited",
    started_at: "t",
    ended_at: "t",
    exit_code: 0,
    resume_session_id: null,
    pinned: 0,
    archived: 0,
    input_tokens: null,
    output_tokens: null,
    total_tokens: null,
    reasoning_tokens: null,
    cached_tokens: null,
    created_at: "t",
    pending_plan_json: pendingPlanJson,
  };
}

describe("parsePendingPlan", () => {
  it("restores a detached approval request from the persisted snapshot", () => {
    const snapshot = JSON.stringify({
      request_id: "req-1",
      plan: "## 目标\n落库计划",
      created_at: "2026-09-15 04:00:00",
    });
    expect(parsePendingPlan(session(snapshot))).toEqual({
      session_record_id: "s1",
      request_id: "req-1",
      profile_id: "",
      workspace_id: "ws-1",
      session_kind: "plan",
      plan: "## 目标\n落库计划",
      detached: true,
    });
  });

  it("returns null for a missing, blank, broken or incomplete snapshot", () => {
    expect(parsePendingPlan(undefined)).toBeNull();
    expect(parsePendingPlan(session(null))).toBeNull();
    expect(parsePendingPlan(session("   "))).toBeNull();
    expect(parsePendingPlan(session("not json"))).toBeNull();
    expect(parsePendingPlan(session(JSON.stringify({ request_id: "r" })))).toBeNull();
    expect(parsePendingPlan(session(JSON.stringify({ plan: "p" })))).toBeNull();
  });
});

const snapshot: ApprovedPlanSnapshot = {
  authorization_id: "authorization-1",
  request_id: "request-1",
  body: "plan",
  feedback: "add tests",
  cwd: "/worktree",
  path: "/worktree/.noxcode/plans/plan-s1.md",
  content_hash: "new-hash",
  saved_hash: "prior-hash",
  status: "failed",
  ai_channel_id: "implementation-channel",
  model: "implementation-model",
  reasoning_effort: "high",
  error: "disk full",
};
const pending = parsePendingPlan(
  session(JSON.stringify({ request_id: "request-1", plan: "plan", created_at: "t" })),
)!;

describe("durable plan approval", () => {
  it("retains retry consent and selected configuration when remote cwd resolution failed", () => {
    const unresolved = {
      ...snapshot,
      cwd: "$HOME/worktree",
      cwd_resolved: false,
      saved_path: "/previous/plan.md",
      error: "SSH unavailable",
    };
    const reloaded = {
      ...session(JSON.stringify({ request_id: "request-1", plan: "plan" })),
      approved_plan_json: JSON.stringify(unresolved),
    };
    expect(authorizedPlanRetry(parsePendingPlan(reloaded)!, parseApprovedPlan(reloaded))).toEqual(
      unresolved,
    );
  });
  it("sends the chosen replacement model when a stopped plan is returned for revision", async () => {
    const resolve = vi.fn().mockResolvedValue(null);
    const model = planApprovalModelArgs(
      false,
      { channelId: "replacement-channel", modelId: "chosen-model" },
      "high",
      Boolean(pending.detached),
    );
    await submitPlanApproval(pending, false, "revise", model, resolve);
    expect(resolve).toHaveBeenCalledExactlyOnceWith(
      "s1",
      "request-1",
      false,
      "revise",
      "replacement-channel",
      "chosen-model",
      "high",
    );
  });
  it("recovers authorized model, feedback, path and retry status after reload", () => {
    const reloaded = {
      ...session(JSON.stringify({ request_id: "request-1", plan: "plan" })),
      approved_plan_json: JSON.stringify(snapshot),
    };
    expect(authorizedPlanRetry(parsePendingPlan(reloaded)!, parseApprovedPlan(reloaded))).toEqual(
      snapshot,
    );
    expect(authorizedPlanRetry({ ...pending, request_id: "new-request" }, snapshot)).toBeNull();
    expect(authorizedPlanRetry({ ...pending, plan: "changed plan" }, snapshot)).toBeNull();
    expect(authorizedPlanRetry(pending, { ...snapshot, status: "cancelled" })).toBeNull();
  });

  it("sends detached approval to the backend with request identity and selected model", async () => {
    const started: AgentSessionStarted = {
      session_record_id: "s1",
      workspace_id: "ws-1",
      profile_id: "",
      session_kind: "execution",
    };
    const resolve = vi.fn().mockResolvedValue(started);
    await expect(
      submitPlanApproval(
        pending,
        true,
        " add tests ",
        {
          aiChannelId: "implementation-channel",
          model: "implementation-model",
          reasoningEffort: "high",
        },
        resolve,
      ),
    ).resolves.toEqual(started);
    expect(resolve).toHaveBeenCalledExactlyOnceWith(
      "s1",
      "request-1",
      true,
      "add tests",
      "implementation-channel",
      "implementation-model",
      "high",
    );
  });

  it("preserves a failed approval so retry does not need a newly manufactured user prompt", async () => {
    const resolve = vi.fn().mockRejectedValue(new Error("disk full"));
    await expect(submitPlanApproval(pending, true, "add tests", {}, resolve)).rejects.toThrow(
      "disk full",
    );
    expect(authorizedPlanRetry(pending, snapshot)?.status).toBe("failed");
  });

  it("uses the same backend for live denial, with no implementation selection", async () => {
    const resolve = vi.fn().mockResolvedValue(null);
    await expect(
      submitPlanApproval({ ...pending, detached: false }, false, "revise", {}, resolve),
    ).resolves.toBeNull();
    expect(resolve).toHaveBeenCalledExactlyOnceWith(
      "s1",
      "request-1",
      false,
      "revise",
      undefined,
      undefined,
      undefined,
    );
  });
});
