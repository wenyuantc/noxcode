import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "@/stores/sessionStore";
import { BackgroundNoticeRow } from "./BackgroundNoticeRow";
import { BackgroundTasks } from "./BackgroundTasks";

vi.mock("@/lib/backend", () => ({
  listNativeBackgroundTasks: vi.fn().mockResolvedValue([]),
  sendNativeBackgroundMessage: vi.fn().mockResolvedValue(undefined),
  stopNativeBackgroundTask: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("./AssistantMarkdown", () => ({
  AssistantMarkdown: ({ text }: { text: string }) => (
    <div className="assistant-markdown">{text}</div>
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

describe("BackgroundNoticeRow and BackgroundTasks", () => {
  beforeEach(() => {
    useSessionStore.setState({
      backgroundBySession: {},
      liveBySession: {},
    });
  });

  it("renders message notices and delivery reports with badges and markdown", () => {
    useSessionStore.setState({
      backgroundBySession: {
        s1: [
          {
            task_id: "task-2",
            description: "审核权限和SQLite变更",
            kind: "explore",
            status: "done",
            report: "## 完整核查报告\n全量通过无实质回归。",
          },
        ],
      },
    });

    const items = [
      {
        id: "1",
        sessionId: "s1",
        kind: "system" as const,
        text: `[后台任务提醒]
- 任务 task-4 (审核AI功能和斜杠命令) 留言: 已审12文件变更核心，发现确定问题。
- 后台任务 task-2 (审核权限和SQLite变更) 完成: ## 结论 **本限定范围未发现实质回归**。用 TaskOutput 读取完整结果。`,
        createdAt: "2026-01-01T00:00:00Z",
      },
    ];

    const html = renderToStaticMarkup(<BackgroundNoticeRow items={items} sessionId="s1" />);

    // Check message notice card
    expect(html).toContain("#task-4");
    expect(html).toContain("审核AI功能和斜杠命令");
    expect(html).toContain("backgroundTaskMessage");
    expect(html).toContain("已审12文件变更核心，发现确定问题。");

    // Check delivery notice card
    expect(html).toContain("#task-2");
    expect(html).toContain("审核权限和SQLite变更");
    expect(html).toContain("backgroundTaskCompleted");
  });

  it("renders BackgroundTasks dashboard card with status counts and task rows", () => {
    useSessionStore.setState({
      liveBySession: { s1: {} as unknown as import("@/lib/types").AgentSessionStarted },
      backgroundBySession: {
        s1: [
          {
            task_id: "task-1",
            description: "静态扫描",
            kind: "general",
            status: "running",
            report: null,
          },
          {
            task_id: "task-2",
            description: "代码审核",
            kind: "explore",
            status: "done",
            report: "审核已完成",
          },
        ],
      },
    });

    const html = renderToStaticMarkup(<BackgroundTasks sessionId="s1" />);

    // Master card header and counter chips
    expect(html).toContain("backgroundTasksTitle");
    expect(html).toContain("backgroundTasksCount");
    expect(html).toContain("backgroundTasksRunningCount");
    expect(html).toContain("backgroundTasksDoneCount");

    // Task rows
    expect(html).toContain("#task-1");
    expect(html).toContain("静态扫描");
    expect(html).toContain("backgroundTaskRunning");

    expect(html).toContain("#task-2");
    expect(html).toContain("代码审核");
    expect(html).toContain("backgroundTaskCompleted");
  });
});
