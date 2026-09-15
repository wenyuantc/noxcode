import type { SessionSubmissionInput } from "@/lib/sessionSubmission";
import type { AgentSession, NativePlanApprovalRequest } from "@/lib/types";

/** `agent_sessions.pending_plan_json` 的形状，由 Rust 侧 `PendingPlanSnapshot` 写入。 */
interface PendingPlanSnapshot {
  request_id: string;
  plan: string;
  created_at: string;
}

/**
 * 把落库的待批准计划还原成审批请求。会话已结束，所以标记 `detached`：
 * 原来挂起的 ExitPlanMode 已经失效，只能以续聊新一轮的方式继续。
 */
export function parsePendingPlan(
  session: AgentSession | undefined,
): NativePlanApprovalRequest | null {
  const raw = session?.pending_plan_json?.trim();
  if (!session || !raw) return null;
  let snapshot: PendingPlanSnapshot;
  try {
    snapshot = JSON.parse(raw) as PendingPlanSnapshot;
  } catch {
    return null;
  }
  const plan = snapshot?.plan?.trim();
  const requestId = snapshot?.request_id?.trim();
  if (!plan || !requestId) return null;
  return {
    session_record_id: session.id,
    request_id: requestId,
    profile_id: "",
    workspace_id: session.workspace_id,
    session_kind: session.session_kind,
    plan,
    detached: true,
  };
}

/**
 * detached 审批要发出的续聊指令。批准时带上完整计划正文：会话停止或强退后
 * transcript 里的 ExitPlanMode 工具对可能已被清理，只说「已批准」模型会不知道要做什么。
 */
export function planApprovalResumePrompt(
  approved: boolean,
  plan: string,
  feedback?: string,
): string {
  const extra = feedback?.trim();
  if (!approved) {
    return `请修改计划：${extra ?? ""}`.trim();
  }
  const base = `已批准计划，请按下面的计划开始实施：\n\n${plan.trim()}`;
  return extra ? `${base}\n\n补充意见：${extra}` : base;
}

/** detached 审批的续聊入参：批准即退出计划模式实施，退回则继续在计划模式里改。 */
export function planApprovalResumeInput(input: {
  approved: boolean;
  sessionId: string;
  workspaceId: string;
  channelId: string;
  modelId?: string | null;
  reasoningEffort?: string | null;
  permissionMode?: string | null;
  plan: string;
  feedback?: string;
}): SessionSubmissionInput {
  return {
    sessionId: input.sessionId,
    workspaceId: input.workspaceId,
    channelId: input.channelId,
    prompt: planApprovalResumePrompt(input.approved, input.plan, input.feedback),
    model: input.modelId ?? null,
    reasoningEffort: input.reasoningEffort ?? null,
    planMode: !input.approved,
    permissionMode: input.permissionMode ?? null,
  };
}
