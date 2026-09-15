import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { listNativeSubagents } from "@/lib/backend";
import { importNotice, SubagentsSettingsTab } from "./SubagentsSettingsTab";

vi.mock("@/lib/backend", () => ({
  listNativeSubagents: vi.fn(),
  createNativeSubagent: vi.fn(),
  deleteNativeSubagent: vi.fn(),
  updateNativeSubagent: vi.fn(),
  listAiChannels: vi.fn(),
  listWorkspaces: vi.fn(),
  generateNativeSubagent: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    i18n: { language: "zh-CN" },
  }),
}));

vi.mock("@/lib/toast", () => ({
  showToast: vi.fn(),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

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

describe("importNotice", () => {
  it("keeps the plain success message when the import has no warnings", () => {
    expect(importNotice("已导入 2 个子智能体", [])).toEqual({
      variant: "success",
      description: "已导入 2 个子智能体",
    });
  });

  it("downgrades to warning and appends every warning when present", () => {
    expect(importNotice("已导入 1 个子智能体", ["忽略重复项 a", "字段缺失 b"])).toEqual({
      variant: "warning",
      description: "已导入 1 个子智能体 忽略重复项 a 字段缺失 b",
    });
  });
});

describe("SubagentsSettingsTab", () => {
  beforeEach(() => {
    vi.mocked(listNativeSubagents).mockReset().mockResolvedValue([]);
  });

  it("renders the builtin and custom subagent sections without layout feedback", () => {
    const html = renderToStaticMarkup(<SubagentsSettingsTab />);

    expect(html).toContain("subagents.title");
    expect(html).toContain("系统内置");
    expect(html).toContain(">general<");
    expect(html).toContain(">explore<");
    expect(html).toContain("subagents.list.loading");
    // 一次性成功/失败提示已改为 toast，页面内不再预留反馈容器。
    expect(html).not.toContain('role="status"');
  });
});
