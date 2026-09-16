import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emptyChannelModel } from "@/lib/modelCatalog";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import type { AgentSession } from "@/lib/types";
import { groupSessionLines, buildTurnBlocks, subagentSegmentIdentity } from "@/lib/sessionLines";
import { PlanRow, PendingPlanApproval } from "./PlanRow";
import { PlanAskCard } from "./PlanAskCard";
import { SubagentDrawerContent } from "./SubagentDrawer";
import { SubagentRow } from "./SubagentRow";
import { QueuedInputs } from "./QueuedInputs";

vi.mock("@/lib/backend", () => ({
  getAgentSessionLogLines: vi.fn(),
  resolveNativePlanApproval: vi.fn(),
  answerNativePlanQuestion: vi.fn(),
  listNativeQueuedInputs: vi.fn(),
  updateNativeQueuedInput: vi.fn(),
  removeNativeQueuedInput: vi.fn(),
  startNativeSession: vi.fn(),
  resumeNativeSession: vi.fn(),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, number>) => {
      if (key === "durationSeconds") return `${options?.seconds}秒`;
      if (key === "durationMinutesOnly") return `${options?.minutes}分钟`;
      if (key === "durationMinutesSeconds") return `${options?.minutes}分${options?.seconds}秒`;
      if (key === "durationHoursOnly") return `${options?.hours}小时`;
      if (key === "durationHoursMinutes") return `${options?.hours}小时${options?.minutes}分`;
      if (key === "durationHoursMinutesSeconds") {
        return `${options?.hours}小时${options?.minutes}分${options?.seconds}秒`;
      }
      return key;
    },
  }),
}));
vi.mock("./AssistantMarkdown", () => ({
  AssistantMarkdown: ({ text }: { text: string }) => <p>{text}</p>,
}));
vi.mock("./ChannelModelPicker", () => ({
  ChannelModelPicker: ({
    selection,
  }: {
    selection?: { channelId: string | null; modelId: string | null };
  }) => (
    <button type="button">{`channel-model-picker:${selection?.channelId ?? ""}/${selection?.modelId ?? ""}`}</button>
  ),
}));
vi.mock("./ThinkingLevelPicker", () => ({
  ThinkingLevelPicker: ({ value }: { value: string }) => (
    <button type="button">{`thinking-level-picker:${value}`}</button>
  ),
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
vi.mock("@/stores/channelStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/channelStore")>();
  return {
    useChannelStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useChannelStore.getState>) => unknown) =>
        selector(actual.useChannelStore.getState()),
      actual.useChannelStore,
    ),
  };
});
vi.mock("@/stores/uiStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/uiStore")>();
  return {
    useUiStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useUiStore.getState>) => unknown) =>
        selector(actual.useUiStore.getState()),
      actual.useUiStore,
    ),
  };
});
vi.mock("@/stores/workspaceStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/workspaceStore")>();
  return {
    useWorkspaceStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useWorkspaceStore.getState>) => unknown) =>
        selector(actual.useWorkspaceStore.getState()),
      actual.useWorkspaceStore,
    ),
  };
});

function stoppedPlanSession(pendingPlanJson?: string | null): AgentSession {
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

describe("native interaction rendering", () => {
  beforeEach(() => {
    useSessionStore.setState({
      permissions: {},
      planQuestions: {},
      planApprovals: {},
      configurationBySession: {},
      liveBySession: {},
    });
    useWorkspaceStore.setState({ sessions: [] });
    useChannelStore.setState({
      channels: [],
      activeChannelId: null,
      activeModelId: null,
    });
  });
  it("shows only the approval card for the current persisted plan", () => {
    const approval = {
      session_record_id: "s1",
      request_id: "current",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      plan: "## 目标\n\n- 修改接口",
    };
    useSessionStore.getState().setPlanApproval(approval);
    const item = groupSessionLines([
      {
        id: "current-plan",
        sessionId: "s1",
        text: `[PLAN]\n${approval.plan}`,
        createdAt: "t",
      },
    ])[0];

    expect(renderToStaticMarkup(<PlanRow item={item} sessionId="s1" />)).toBe("");
    const current = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(current).toContain("修改接口");
    expect(current).toContain("planWaitingApproval");
    expect(current).toContain("planApprovalApprove");
    expect(current).toContain("planApprovalReject");

    useSessionStore.getState().resolveRequest({ ...approval, kind: "plan_approval" });
    const historical = renderToStaticMarkup(<PlanRow item={item} sessionId="s1" />);
    expect(historical).toContain("修改接口");
    expect(historical).toContain("planHistoricalBadge");
    expect(historical).not.toContain("planApprovalApprove");
    expect(historical).not.toContain("planApprovalReject");
  });
  it("keeps the approval card from the persisted plan after the session stopped", () => {
    const plan = "## 目标\n落库计划";
    useWorkspaceStore.setState({
      sessions: [
        stoppedPlanSession(
          JSON.stringify({ request_id: "req-1", plan, created_at: "2026-09-15 04:00:00" }),
        ),
      ],
    });

    const card = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(card).toContain("落库计划");
    expect(card).toContain("planContinueBadge");
    expect(card).toContain("planContinueHint");
    expect(card).toContain("planApprovalApprove");
    expect(card).toContain("planApprovalReject");
    expect(card).not.toContain("planWaitingApproval");

    // 同一份计划的历史 [PLAN] 行不再重复渲染
    const item = groupSessionLines([
      { id: "plan-line", sessionId: "s1", text: `[PLAN]\n${plan}`, createdAt: "t" },
    ])[0];
    expect(renderToStaticMarkup(<PlanRow item={item} sessionId="s1" />)).toBe("");
  });
  it("hides the persisted plan card while the session is live again", () => {
    useWorkspaceStore.setState({
      sessions: [stoppedPlanSession(JSON.stringify({ request_id: "req-1", plan: "落库计划" }))],
    });
    useSessionStore.setState({
      liveBySession: {
        s1: {
          profile_id: "p",
          workspace_id: "ws-1",
          session_kind: "plan",
          session_record_id: "s1",
        },
      },
    });
    expect(renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />)).toBe("");
  });
  it("ignores surrounding whitespace when matching the pending plan", () => {
    useSessionStore.getState().setPlanApproval({
      session_record_id: "s1",
      request_id: "current",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      plan: "\ncurrent plan\n",
    });
    const item = groupSessionLines([
      {
        id: "current-plan",
        sessionId: "s1",
        text: "[PLAN]\n  current plan  ",
        createdAt: "t",
      },
    ])[0];

    expect(renderToStaticMarkup(<PlanRow item={item} sessionId="s1" />)).toBe("");
  });
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
    expect(historical).not.toContain("planApprovalApprove");
    expect(historical).not.toContain("planApprovalReject");
    const current = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(current).toContain("new plan");
    expect(current).toContain("planApprovalApprove");
    expect(current).toContain("planApprovalReject");
    expect(current).toContain("planWaitingApproval");
    expect(current).toContain("planAddFeedback");
    expect(current).toContain("planCopy");
    expect(current).toContain("channel-model-picker:/");
    expect(current).not.toContain("thinking-level-picker:");
  });
  it("defaults the approval picker to the current session model", () => {
    useSessionStore.setState({
      configurationBySession: {
        s1: {
          ai_channel_id: "ch-1",
          model: "deepseek-v4-flash",
          reasoning_effort: null,
          permission_mode: "default",
          plan_mode: true,
        },
      },
    });
    useSessionStore.getState().setPlanApproval({
      session_record_id: "s1",
      request_id: "new",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      plan: "new plan",
    });
    const current = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(current).toContain("channel-model-picker:ch-1/deepseek-v4-flash");
    expect(current).not.toContain("thinking-level-picker:");
  });
  it("shows the thinking level picker for a thinking-enabled implementation model", () => {
    useChannelStore.setState({
      channels: [
        {
          id: "ch-1",
          name: "Myai-Ollama",
          protocol: "openai",
          base_url: "http://localhost",
          extra_headers_json: null,
          models: [
            {
              ...emptyChannelModel("deepseek-v4-flash"),
              thinking_enabled: true,
              thinking_level: "high",
              thinking_levels: ["low", "high", "max"],
            },
          ],
          responses_continuation: "auto",
          enabled: true,
          api_key: null,
          api_key_configured: false,
          created_at: "",
          updated_at: "",
        },
      ],
    });
    useSessionStore.setState({
      configurationBySession: {
        s1: {
          ai_channel_id: "ch-1",
          model: "deepseek-v4-flash",
          reasoning_effort: "max",
          permission_mode: "default",
          plan_mode: true,
        },
      },
    });
    useSessionStore.getState().setPlanApproval({
      session_record_id: "s1",
      request_id: "new",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      plan: "new plan",
    });
    const current = renderToStaticMarkup(<PendingPlanApproval sessionId="s1" />);
    expect(current).toContain("channel-model-picker:ch-1/deepseek-v4-flash");
    expect(current).toContain("thinking-level-picker:max");
  });
  it("renders PlanAskCard with modern layout and options", () => {
    useSessionStore.getState().setPlanQuestion({
      session_record_id: "s1",
      request_id: "q1",
      profile_id: "p",
      workspace_id: "ws",
      session_kind: "plan",
      questions: [
        {
          prompt: "Choose architecture style",
          options: ["REST", "GraphQL"],
        },
      ],
    });
    const html = renderToStaticMarkup(<PlanAskCard sessionId="s1" />);
    expect(html).toContain("Choose architecture style");
    expect(html).toContain("REST");
    expect(html).toContain("GraphQL");
    expect(html).toContain("planAskSend");
    expect(html).toContain("planAskCancel");
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
    expect(html).toContain(status === "失败" ? "subagentFailed" : "subagentStopped");
  });
  it("freezes a completed subagent duration while the parent turn is still working", () => {
    const items = groupSessionLines([
      {
        id: "start",
        sessionId: "s1",
        text: "[子 Agent 2(explore) - 核对前端实现] 启动（explore）",
        createdAt: "2026-01-01T00:00:00Z",
      },
      {
        id: "end",
        sessionId: "s1",
        text: "[子 Agent 2(explore) - 核对前端实现] 结束 成功",
        createdAt: "2026-01-01T00:00:30Z",
      },
    ]);
    const segment = buildTurnBlocks(items)
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const nowMs = Date.parse("2026-01-01T00:00:00Z") + 258_000;
    const html = renderToStaticMarkup(<SubagentRow segment={segment!} running nowMs={nowMs} />);
    expect(html).toContain("subagentCompleted");
    expect(html).not.toContain("subagentRunning");
    expect(html).toContain("30秒");
    expect(html).not.toContain("4分18秒");
  });
  it("keeps a running subagent duration live against nowMs", () => {
    const items = groupSessionLines([
      {
        id: "start",
        sessionId: "s1",
        text: "[子 Agent 1(explore) - rust窗口状态链路检查] 启动（explore）",
        createdAt: "2026-01-01T00:00:00Z",
      },
    ]);
    const segment = buildTurnBlocks(items)
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const nowMs = Date.parse("2026-01-01T00:00:00Z") + 258_000;
    const html = renderToStaticMarkup(<SubagentRow segment={segment!} running nowMs={nowMs} />);
    expect(html).toContain("subagentRunning");
    expect(html).not.toContain("subagentCompleted");
    expect(html).toContain("4分18秒");
  });
  it("renders active ring when SubagentRow matches activeSubagent", () => {
    const items = groupSessionLines([
      {
        id: "start",
        sessionId: "s1",
        text: "[子 Agent 1(explore) - rust窗口状态链路检查] 启动（explore）",
        createdAt: "2026-01-01T00:00:00Z",
      },
    ]);
    const segment = buildTurnBlocks(items)
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const identity = subagentSegmentIdentity(segment!.items[0] ?? {})!;
    useUiStore.setState({
      activeSubagent: { sessionId: "s1", identity },
    });
    const html = renderToStaticMarkup(<SubagentRow segment={segment!} sessionId="s1" />);
    expect(html).toContain("ring-primary");
  });
  it("renders SubagentDrawerContent when activeSubagent is set for the session", () => {
    const lines = [
      {
        id: "start",
        sessionId: "s1",
        text: "[子 Agent 1(explore) - 分析后端] 启动（explore）",
        createdAt: "2026-01-01T00:00:00Z",
      },
      {
        id: "report",
        sessionId: "s1",
        text: "[子 Agent 1(explore) - 分析后端] 子 Agent（explore / 分析后端）完成\n\n这是交付报告内容",
        createdAt: "2026-01-01T00:00:10Z",
      },
      {
        id: "end",
        sessionId: "s1",
        text: "[子 Agent 1(explore) - 分析后端] 结束 成功",
        createdAt: "2026-01-01T00:00:11Z",
      },
    ];
    const segment = buildTurnBlocks(groupSessionLines(lines))
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const html = renderToStaticMarkup(
      <SubagentDrawerContent
        segment={segment!}
        peerSegments={[segment!]}
        sessionId="s1"
        activeIdentity="index:1"
        isWorking={false}
        onClose={() => {}}
        onSelectPeer={() => {}}
      />,
    );
    expect(html).toContain("分析后端");
    expect(html).toContain("这是交付报告内容");
    expect(html).toContain("subagentReport");
  });
  it("renders running SubagentDrawerContent with live status and scroll body", () => {
    const lines = [
      {
        id: "start",
        sessionId: "s1",
        text: "[子 Agent 2(explore) - 分析 Rust 后端结构] 启动（explore）",
        createdAt: "2026-01-01T00:00:00Z",
      },
    ];
    const segment = buildTurnBlocks(groupSessionLines(lines))
      .flatMap((block) => block.segments)
      .find((item) => item.kind === "subagent");
    expect(segment).toBeDefined();
    const html = renderToStaticMarkup(
      <SubagentDrawerContent
        segment={segment!}
        peerSegments={[segment!]}
        sessionId="s1"
        activeIdentity="index:2"
        isWorking={true}
        onClose={() => {}}
        onSelectPeer={() => {}}
      />,
    );
    expect(html).toContain("分析 Rust 后端结构");
    expect(html).toContain("subagentRunning");
    expect(html).toContain("overflow-y-auto");
  });
});
