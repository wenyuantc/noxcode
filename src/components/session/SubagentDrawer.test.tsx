import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { SubagentDrawer } from "./SubagentDrawer";

vi.mock("@base-ui/react/dialog", () => {
  const Part = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return {
    Dialog: {
      Root: ({ children, open }: { children?: ReactNode; open?: boolean }) =>
        open ? <div data-testid="drawer-root">{children}</div> : null,
      Portal: Part,
      Backdrop: Part,
      Popup: Part,
      Title: Part,
      Description: Part,
      Close: Part,
    },
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => {
      if (key === "durationSeconds") return `${options?.seconds}秒`;
      if (options?.defaultValue) return options.defaultValue;
      return key;
    },
  }),
}));

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

describe("SubagentDrawer", () => {
  beforeEach(() => {
    useSessionStore.setState({
      lines: {},
      subagentsBySession: {},
      turnState: {},
    });
    useUiStore.setState({ activeSubagent: null });
  });

  it("does not render popup when activeSubagent is null", () => {
    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toBe("");
  });

  it("does not render popup when activeSubagent is for a different session", () => {
    useUiStore.setState({
      activeSubagent: { sessionId: "s2", identity: "index:1" },
    });
    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toBe("");
  });

  it("renders drawer when activeSubagent is set with canonical 'index:1' identity", () => {
    useUiStore.setState({
      activeSubagent: { sessionId: "s1", identity: "index:1" },
    });
    useSessionStore.setState({
      lines: {
        s1: [
          {
            id: "start",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 启动（explore）",
            createdAt: "2026-01-01T00:00:00Z",
          },
          {
            id: "report",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 子 Agent（explore / 探索前端结构）完成\n\n报告内容",
            createdAt: "2026-01-01T00:00:10Z",
          },
          {
            id: "end",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 结束 成功",
            createdAt: "2026-01-01T00:00:11Z",
          },
        ],
      },
    });

    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toContain('data-testid="drawer-root"');
    expect(html).toContain("探索前端结构");
    expect(html).toContain("报告内容");
  });

  it("renders drawer when activeSubagent is set with raw tag identity", () => {
    useUiStore.setState({
      activeSubagent: {
        sessionId: "s1",
        identity: "[子 Agent 1(explore) - 探索前端结构]",
      },
    });
    useSessionStore.setState({
      lines: {
        s1: [
          {
            id: "start",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 启动（explore）",
            createdAt: "2026-01-01T00:00:00Z",
          },
          {
            id: "end",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 结束 成功",
            createdAt: "2026-01-01T00:00:11Z",
          },
        ],
      },
    });

    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toContain('data-testid="drawer-root"');
    expect(html).toContain("探索前端结构");
  });

  it("renders drawer from subagentsBySession fallback when lines have no segments yet", () => {
    useUiStore.setState({
      activeSubagent: { sessionId: "s1", identity: "index:1" },
    });
    useSessionStore.setState({
      lines: { s1: [] },
      subagentsBySession: {
        s1: [
          {
            id: "[子 Agent 1(explore) - 探索前端结构]",
            index: 1,
            kind: "explore",
            description: "探索前端结构",
            status: "running",
            start_time_ms: 1000,
            duration_ms: null,
            error_message: null,
          },
        ],
      },
    });

    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toContain('data-testid="drawer-root"');
    expect(html).toContain("探索前端结构");
  });

  it("highlights selected peer tag in multi-subagent switcher bar", () => {
    useUiStore.setState({
      activeSubagent: {
        sessionId: "s1",
        identity: "[子 Agent 2(explore) - 分析后端结构]",
      },
    });
    useSessionStore.setState({
      lines: {
        s1: [
          {
            id: "s1",
            sessionId: "s1",
            text: "[子 Agent 1(explore) - 探索前端结构] 启动（explore）",
            createdAt: "2026-01-01T00:00:00Z",
          },
          {
            id: "s2",
            sessionId: "s1",
            text: "[子 Agent 2(explore) - 分析后端结构] 启动（explore）",
            createdAt: "2026-01-01T00:00:01Z",
          },
        ],
      },
    });

    const html = renderToStaticMarkup(<SubagentDrawer sessionId="s1" />);
    expect(html).toContain("subagentBatchSwitch");
    expect(html).toContain("bg-primary text-primary-foreground border-primary");
    expect(html).toContain("#2");
  });
});
