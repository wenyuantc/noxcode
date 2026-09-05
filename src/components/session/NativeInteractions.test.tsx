import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "@/stores/sessionStore";
import { groupSessionLines, buildTurnBlocks } from "@/lib/sessionLines";
import { PlanRow, PendingPlanApproval } from "./PlanRow";
import { SubagentRow } from "./SubagentRow";
import { QueuedInputs } from "./QueuedInputs";

vi.mock("@/lib/backend", () => ({
  getAgentSessionLogLines: vi.fn(),
  resolveNativePlanApproval: vi.fn(),
  listNativeQueuedInputs: vi.fn(),
  updateNativeQueuedInput: vi.fn(),
  removeNativeQueuedInput: vi.fn(),
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
  it("renders pending messages in order with edit controls outside the transcript", () => {
    useSessionStore.getState().onStarted({
      profile_id: "",
      workspace_id: "ws",
      session_kind: "execution",
      session_record_id: "s1",
      input_queue_id: "q1",
    });
    useSessionStore.getState().onInputQueue({
      session_record_id: "s1",
      queue_id: "q1",
      revision: 1,
      items: [
        { id: "1", text: "first queued", editing: false, image_count: 0 },
        { id: "2", text: "second queued", editing: true, image_count: 0 },
      ],
    });
    const html = renderToStaticMarkup(<QueuedInputs sessionId="s1" />);
    expect(html.indexOf("first queued")).toBeLessThan(html.indexOf("second queued"));
    expect(html).toContain("queuedInput.edit");
    expect(html).toContain("queuedInput.save");
    expect(html).toContain("queuedInput.cancelEdit");
    expect(html).toContain("<textarea");
    useSessionStore
      .getState()
      .onInputQueue({ session_record_id: "s1", queue_id: "q1", revision: 2, items: [] });
    expect(renderToStaticMarkup(<QueuedInputs sessionId="s1" />)).toBe("");
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
