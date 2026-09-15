import { describe, expect, it } from "vitest";

import {
  parsePendingPlan,
  planApprovalResumeInput,
  planApprovalResumePrompt,
} from "./planApproval";
import { sessionSubmissionPayload } from "./sessionSubmission";
import type { AgentSession } from "./types";

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

describe("planApprovalResumePrompt", () => {
  it("carries the full plan when approving so a trimmed transcript still has it", () => {
    expect(planApprovalResumePrompt(true, "## 目标\n改接口")).toBe(
      "已批准计划，请按下面的计划开始实施：\n\n## 目标\n改接口",
    );
    expect(planApprovalResumePrompt(true, "计划", "  顺便加日志  ")).toBe(
      "已批准计划，请按下面的计划开始实施：\n\n计划\n\n补充意见：顺便加日志",
    );
  });

  it("asks for a revision when sending the plan back", () => {
    expect(planApprovalResumePrompt(false, "计划", "拆成两步")).toBe("请修改计划：拆成两步");
  });
});

describe("planApprovalResumeInput", () => {
  const base = {
    sessionId: "s1",
    workspaceId: "ws-1",
    channelId: "ch",
    modelId: "gpt",
    reasoningEffort: "high",
    permissionMode: "default",
    plan: "计划",
  };

  it("leaves plan mode when approved so the next turn can implement", () => {
    const input = planApprovalResumeInput({ ...base, approved: true });
    expect(input.planMode).toBe(false);
    expect(input.prompt).toContain("已批准计划");
    // 以同一 session id 续聊，不会新建会话
    expect(sessionSubmissionPayload(input).resume_session_id).toBe("s1");
  });

  it("stays in plan mode when sent back", () => {
    const input = planApprovalResumeInput({ ...base, approved: false, feedback: "拆成两步" });
    expect(input.planMode).toBe(true);
    expect(input.prompt).toBe("请修改计划：拆成两步");
  });

  it("passes the picked channel, model and thinking level through", () => {
    expect(planApprovalResumeInput({ ...base, approved: true })).toMatchObject({
      channelId: "ch",
      model: "gpt",
      reasoningEffort: "high",
      permissionMode: "default",
    });
    expect(
      planApprovalResumeInput({
        ...base,
        approved: true,
        modelId: null,
        reasoningEffort: null,
        permissionMode: null,
      }),
    ).toMatchObject({ model: null, reasoningEffort: null, permissionMode: null });
  });
});
