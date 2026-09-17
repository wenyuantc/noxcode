import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionSubagentInfo } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { SubagentPopover } from "./SubagentPopover";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { count?: number; duration?: string; defaultValue?: string }) => {
      if (options?.count !== undefined) return `${options.count} items`;
      if (options?.duration !== undefined) return `took ${options.duration}`;
      return options?.defaultValue ?? key;
    },
  }),
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

describe("SubagentPopover", () => {
  beforeEach(() => {
    useSessionStore.setState({ subagentsBySession: {} });
    useUiStore.setState({ activeSubagent: null });
  });

  it("renders null when there are no subagents for the session", () => {
    const html = renderToStaticMarkup(<SubagentPopover sessionId="s1" />);
    expect(html).toBe("");
  });

  it("renders trigger button and count badge when subagents exist", () => {
    const subagents: SessionSubagentInfo[] = [
      {
        id: "[子 Agent 1(general) - 查阅架构文档]",
        index: 1,
        kind: "general",
        description: "查阅架构文档",
        status: "completed",
        start_time_ms: 1000,
        duration_ms: 5000,
        error_message: null,
      },
    ];
    useSessionStore.setState({
      subagentsBySession: { s1: subagents },
    });

    const html = renderToStaticMarkup(<SubagentPopover sessionId="s1" />);
    expect(html).toContain("1");
    expect(html).toContain("子智能体运行状态");
  });

  it("applies running pulse animation and emerald color when any subagent is running", () => {
    const subagents: SessionSubagentInfo[] = [
      {
        id: "[子 Agent 1(explore) - 检索符号]",
        index: 1,
        kind: "explore",
        description: "检索符号",
        status: "running",
        start_time_ms: 1000,
        duration_ms: null,
        error_message: null,
      },
    ];
    useSessionStore.setState({
      subagentsBySession: { s1: subagents },
    });

    const html = renderToStaticMarkup(<SubagentPopover sessionId="s1" />);
    expect(html).toContain("animate-pulse");
    expect(html).toContain("bg-emerald-500");
  });

  it("applies destructive warning badge when subagent failed and none is running", () => {
    const subagents: SessionSubagentInfo[] = [
      {
        id: "[子 Agent 1(general) - 构建代码]",
        index: 1,
        kind: "general",
        description: "构建代码",
        status: "failed",
        start_time_ms: 1000,
        duration_ms: 2000,
        error_message: "编译失败",
      },
    ];
    useSessionStore.setState({
      subagentsBySession: { s1: subagents },
    });

    const html = renderToStaticMarkup(<SubagentPopover sessionId="s1" />);
    expect(html).not.toContain("animate-pulse");
    expect(html).toContain("bg-destructive text-white");
    expect(html).toContain("1");
  });

  it("renders neutral styling when all subagents completed successfully", () => {
    const subagents: SessionSubagentInfo[] = [
      {
        id: "[子 Agent 1(general) - 查阅架构文档]",
        index: 1,
        kind: "general",
        description: "查阅架构文档",
        status: "completed",
        start_time_ms: 1000,
        duration_ms: 5000,
        error_message: null,
      },
    ];
    useSessionStore.setState({
      subagentsBySession: { s1: subagents },
    });

    const html = renderToStaticMarkup(<SubagentPopover sessionId="s1" />);
    expect(html).not.toContain("animate-pulse");
    expect(html).not.toContain("bg-destructive");
    expect(html).toContain("bg-muted");
  });
});
