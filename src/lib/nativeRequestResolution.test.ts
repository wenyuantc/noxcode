import { beforeEach, describe, expect, it, vi } from "vitest";
import { resolveSessionRequest } from "./nativeRequestResolution";
import { useSessionStore } from "@/stores/sessionStore";
import type {
  NativePermissionRequest,
  NativePlanApprovalRequest,
  NativePlanQuestionRequest,
} from "./types";

vi.mock("@/lib/backend", () => ({ getAgentSessionLogLines: vi.fn() }));

const base = {
  session_record_id: "s1",
  request_id: "r1",
  profile_id: "p",
  workspace_id: "ws",
  session_kind: "execution",
};
const permission: NativePermissionRequest = {
  ...base,
  tool_name: "Bash",
  kind: "opaque",
  summary: "command",
  remote: false,
  mcp_server_id: null,
};
const question: NativePlanQuestionRequest = {
  ...base,
  questions: [{ prompt: "question", options: [] }],
};
const approval: NativePlanApprovalRequest = { ...base, plan: "current plan" };

describe("native request resolution", () => {
  beforeEach(() =>
    useSessionStore.setState({ permissions: {}, planQuestions: {}, planApprovals: {} }),
  );
  it.each(["permission", "question", "plan_approval"] as const)(
    "retains %s after an IPC failure so it can be retried",
    async (kind) => {
      const state = useSessionStore.getState();
      state.setPermission(permission);
      state.setPlanQuestion(question);
      state.setPlanApproval(approval);
      const send = vi
        .fn()
        .mockRejectedValueOnce(new Error("IPC failed"))
        .mockResolvedValueOnce(undefined);
      const key =
        kind === "permission"
          ? "permissions"
          : kind === "question"
            ? "planQuestions"
            : "planApprovals";
      await expect(resolveSessionRequest({ ...base, kind }, send)).rejects.toThrow("IPC failed");
      expect(useSessionStore.getState()[key].s1.r1).toBeDefined();
      await resolveSessionRequest({ ...base, kind }, send);
      expect(useSessionStore.getState()[key].s1.r1).toBeUndefined();
      expect(send).toHaveBeenCalledTimes(2);
    },
  );
  it("does not clear a newer request while an older response is in flight", async () => {
    useSessionStore.getState().setPlanApproval(approval);
    let complete!: () => void;
    const pending = resolveSessionRequest(
      { ...base, kind: "plan_approval" },
      () =>
        new Promise((resolve) => {
          complete = resolve;
        }),
    );
    expect(useSessionStore.getState().planApprovals.s1.r1).toBeDefined();
    useSessionStore.getState().setPlanApproval({ ...approval, request_id: "r2" });
    complete();
    await pending;
    expect(Object.keys(useSessionStore.getState().planApprovals.s1)).toEqual(["r2"]);
  });
});
