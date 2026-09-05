import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "@/stores/sessionStore";
import { groupSessionLines, buildTurnBlocks } from "@/lib/sessionLines";
import { PlanRow, PendingPlanApproval } from "./PlanRow";
import { SubagentRow } from "./SubagentRow";

vi.mock("@/lib/backend", () => ({
  getAgentSessionLogLines: vi.fn(),
  resolveNativePlanApproval: vi.fn(),
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("./AssistantMarkdown", () => ({
  AssistantMarkdown: ({ text }: { text: string }) => <p>{text}</p>,
}));
vi.mock("@/stores/sessionStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/sessionStore")>();
  return {
    useSessionStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useSessionStore.getState>) => unknown) =>
        selector(actual.useSessionStore.getState()),
      actual.useSessionStore,
    ),
  };
});

describe("native interaction rendering", () => {
  beforeEach(() =>
    useSessionStore.setState({ permissions: {}, planQuestions: {}, planApprovals: {} }),
  );
  it("never attaches current approval controls to a historical plan", () => {
    useSessionStore.getState().setPlanApproval({
      session_record_id: "s1",
      request_id: "new",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      plan: "new plan",
    });
    const item = groupSessionLines([
      { id: "old", sessionId: "s1", text: "[PLAN]\nold plan", createdAt: "t" },
    ])[0];
    const historical = renderToStaticMarkup(<PlanRow item={item} sessionId="s1" />);
    expect(historical).toContain("old plan");
    expect(historical).not.toContain("<button");
    const current = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(current).toContain("new plan");
    expect(current).toContain("planContinueTask");
  });
  it.each(["失败", "停止"])("shows a background task ending with %s as terminal", (status) => {
    const items = groupSessionLines([
      {
        id: "end",
        sessionId: "s1",
        text: `[子 Agent 1(general) - test] 后台任务 task-1 结束 ${status}`,
        createdAt: "t",
      },
    ]);
    const blocks = buildTurnBlocks(items);
    const segment = blocks
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const html = renderToStaticMarkup(<SubagentRow segment={segment!} running />);
    expect(html).not.toContain("subagentRunning");
    expect(html).not.toContain("subagentCompleted");
    expect(html).toContain(status === "失败" ? "subagentFailed" : "已停止");
  });
});
