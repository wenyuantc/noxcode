import { resolveNativePlanApproval } from "@/lib/backend";
import type { PlanApprovalModelArgs } from "@/lib/sessionModel";
import type { AgentSession, ApprovedPlanSnapshot, NativePlanApprovalRequest } from "@/lib/types";

/** `agent_sessions.pending_plan_json` 的形状，由 Rust 侧 `PendingPlanSnapshot` 写入。 */
interface PendingPlanSnapshot {
  request_id: string;
  plan: string;
  created_at: string;
}

/**
 * 把落库的待批准计划还原成审批请求。会话已结束，所以标记 `detached`：
 * 原来挂起的 ExitPlanMode 已经失效，由后端保存计划并安全启动新一轮。
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

/** Persisted authorization also drives retry UI after stopping/reopening the app. */
export function parseApprovedPlan(session: AgentSession | undefined): ApprovedPlanSnapshot | null {
  try {
    const plan = JSON.parse(session?.approved_plan_json ?? "null") as ApprovedPlanSnapshot | null;
    return plan &&
      typeof plan.request_id === "string" &&
      typeof plan.path === "string" &&
      typeof plan.body === "string" &&
      typeof plan.ai_channel_id === "string" &&
      typeof plan.model === "string" &&
      typeof plan.feedback === "string" &&
      ["saving", "failed", "saved", "cancelled"].includes(plan.status)
      ? plan
      : null;
  } catch {
    return null;
  }
}

export function authorizedPlanRetry(
  pending: NativePlanApprovalRequest | undefined,
  saved: ApprovedPlanSnapshot | null,
): ApprovedPlanSnapshot | null {
  return pending &&
    saved &&
    pending.request_id === saved.request_id &&
    pending.plan.trim() === saved.body.trim() &&
    saved.status !== "cancelled"
    ? saved
    : null;
}

/** Live and detached requests share the authoritative backend transaction. */
export function submitPlanApproval(
  request: NativePlanApprovalRequest,
  approved: boolean,
  feedback: string,
  model: PlanApprovalModelArgs,
  resolve = resolveNativePlanApproval,
) {
  return resolve(
    request.session_record_id,
    request.request_id,
    approved,
    feedback.trim() || undefined,
    model.aiChannelId,
    model.model,
    model.reasoningEffort,
  );
}
