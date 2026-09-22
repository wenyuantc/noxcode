import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NativeMemoryEntry, NativeSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import {
  calculateKindCounts,
  filterMemoryEntries,
  MemorySection,
  shortenPath,
} from "./MemorySection";

vi.mock("@/lib/backend", () => ({
  listNativeMemories: vi.fn().mockResolvedValue({
    dir: "/Users/test/Library/Application Support/com.wenyuan.noxcode/memory/noxcode-12345678",
    index: "- [Sample](sample.md) — A sample memory",
    entries: [],
    extractions: 2,
    dreams: 1,
  }),
  deleteNativeMemory: vi.fn().mockResolvedValue(undefined),
  dreamNativeMemory: vi.fn().mockResolvedValue("dreamed"),
  openNativeMemoryDir: vi.fn().mockResolvedValue(undefined),
  updateNativeSettings: vi.fn().mockResolvedValue({}),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
    i18n: { language: "zh-CN" },
  }),
}));

vi.mock("@/stores/settingsStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/settingsStore")>();
  return {
    useSettingsStore: Object.assign(
      (selector: (state: ReturnType<typeof actual.useSettingsStore.getState>) => unknown) =>
        selector(actual.useSettingsStore.getState()),
      actual.useSettingsStore,
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

function sampleNative(): NativeSettings {
  return {
    max_turns: 40,
    max_subagent_turns: 20,
    permission_mode: "default",
    max_concurrent_subagents: 1,
    subagent_policy: "conservative",
    context_window_tokens: 128000,
    use_custom_context_window: false,
    rollout_token_budget: 10_000_000,
    max_tool_output_tokens: 4096,
    permission_timeout_secs: 300,
    subagent_budget_share_percent: 40,
    auto_checkpoint_after_tool_call: true,
    checkpoint_retention_days: 7,
    desktop_notifications: true,
    artifact_retention_days: 7,
    model_retry_max_retries: 6,
    model_retry_base_delay_ms: 1000,
    model_retry_max_delay_ms: 30000,
    model_retry_backoff_factor: 2,
    bash_default_timeout_secs: 120,
    shell_snapshot_enabled: true,
    rg_sidecar_enabled: true,
    lsp_enabled: true,
    computer_control_enabled: false,
    bash_sandbox_enabled: false,
    auto_compact_threshold_percent: 85,
    microcompact_enabled: true,
    memory_enabled: true,
    memory_dream_interval: 10,
    hooks: [],
    global_prompt_template: "",
    worktree_root: "",
    worktree_fetch_before_create: false,
    worktree_auto_prune: true,
    worktree_auto_prune_limit: 15,
  };
}

const sampleEntries: NativeMemoryEntry[] = [
  {
    file_name: "2026-09-22-arch.md",
    name: "项目架构设计",
    description: "前端采用 Vite + React，原生层 Tauri + Rust",
    kind: "project",
    created_at: "2026-09-22 10:00:00",
    updated_at: "2026-09-22 10:30:00",
    body: "## 架构设计\n- Vite\n- Tauri",
  },
  {
    file_name: "2026-09-21-user-pref.md",
    name: "用户暗黑模式偏好",
    description: "用户倾向于深色界面与等宽字体排版",
    kind: "user",
    created_at: "2026-09-21 09:00:00",
    updated_at: "2026-09-21 09:15:00",
    body: "偏好：深色模式",
  },
  {
    file_name: "2026-09-20-feedback.md",
    name: "测试覆盖率要求",
    description: "每次重构后需保证单测与 clippy 全绿",
    kind: "feedback",
    created_at: "2026-09-20 08:00:00",
    updated_at: "2026-09-20 08:05:00",
    body: "核心规范：必须跑测试",
  },
];

describe("shortenPath", () => {
  it("shortens deep paths to the last two path segments", () => {
    expect(
      shortenPath("/Users/wenyuantc/Library/Application Support/noxcode/memory/noxcode-1234"),
    ).toBe("…/memory/noxcode-1234");
  });

  it("normalizes Windows backslashes", () => {
    expect(shortenPath("C:\\Users\\AppData\\Local\\noxcode\\memory\\project-a")).toBe(
      "…/memory/project-a",
    );
  });

  it("leaves short paths intact", () => {
    expect(shortenPath("memory/project")).toBe("memory/project");
    expect(shortenPath("project")).toBe("project");
  });
});

describe("calculateKindCounts", () => {
  it("correctly counts entries by kind", () => {
    const counts = calculateKindCounts(sampleEntries);
    expect(counts.all).toBe(3);
    expect(counts.project).toBe(1);
    expect(counts.user).toBe(1);
    expect(counts.feedback).toBe(1);
    expect(counts.reference).toBe(0);
  });

  it("handles null or empty list gracefully", () => {
    const counts = calculateKindCounts(null);
    expect(counts.all).toBe(0);
    expect(counts.project).toBe(0);
  });
});

describe("filterMemoryEntries", () => {
  it("returns all entries when query is empty and kind is all", () => {
    expect(filterMemoryEntries(sampleEntries, "", "all")).toEqual(sampleEntries);
  });

  it("filters by category kind", () => {
    const res = filterMemoryEntries(sampleEntries, "", "user");
    expect(res).toHaveLength(1);
    expect(res[0].name).toBe("用户暗黑模式偏好");
  });

  it("filters by search term in name, description, or body", () => {
    expect(filterMemoryEntries(sampleEntries, "架构", "all")).toHaveLength(1);
    expect(filterMemoryEntries(sampleEntries, "深色", "all")).toHaveLength(1);
    expect(filterMemoryEntries(sampleEntries, "clippy", "all")).toHaveLength(1);
    expect(filterMemoryEntries(sampleEntries, "不存在的内容", "all")).toHaveLength(0);
  });

  it("combines category filter and search query", () => {
    const res = filterMemoryEntries(sampleEntries, "测试", "feedback");
    expect(res).toHaveLength(1);

    const noMatch = filterMemoryEntries(sampleEntries, "测试", "project");
    expect(noMatch).toHaveLength(0);
  });
});

describe("MemorySection Component", () => {
  beforeEach(() => {
    useSettingsStore.setState({ native: sampleNative() });
    useWorkspaceStore.setState({
      workspaces: [
        {
          id: "ws-1",
          name: "noxcode",
          workspace_type: "local",
          repo_path: "/test/noxcode",
          ssh_config_id: null,
          remote_repo_path: null,
          created_at: "2026-09-22 00:00:00",
          updated_at: "2026-09-22 00:00:00",
        },
      ],
      activeWorkspaceId: "ws-1",
    });
  });

  it("renders pipeline controls and memory management structure", () => {
    const html = renderToStaticMarkup(<MemorySection />);
    expect(html).toContain("settings:memory.settingsTitle");
    expect(html).toContain("settings:memory.enabled");
    expect(html).toContain("settings:memory.dreamInterval");
    expect(html).toContain("settings:memory.entriesTitle");
    expect(html).toContain("settings:memory.totalEntries");
    expect(html).toContain("settings:memory.extractionsCount");
    expect(html).toContain("settings:memory.dreamsCount");
    expect(html).toContain("settings:memory.storageDir");
    expect(html).toContain('id="memory-enabled"');
    expect(html).toContain('id="memory-dream-interval"');
  });
});
