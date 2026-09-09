import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { listManagedWorktrees } from "@/lib/backend";
import type { NativeSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { WorktreesSection } from "./WorktreesSection";

vi.mock("@/lib/backend", () => ({
  listManagedWorktrees: vi.fn(),
  removeManagedWorktree: vi.fn(),
  updateNativeSettings: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
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

describe("WorktreesSection", () => {
  beforeEach(() => {
    vi.mocked(listManagedWorktrees).mockReset().mockResolvedValue({
      root: "/cfg/worktrees",
      default_root: "/cfg/worktrees",
      items: [],
    });
    useSettingsStore.setState({ native: sampleNative() });
  });

  it("renders the four worktree settings and empty list", () => {
    const html = renderToStaticMarkup(<WorktreesSection />);
    expect(html).toContain("settings:worktrees.configTitle");
    expect(html).toContain("settings:worktrees.root");
    expect(html).toContain("settings:worktrees.fetchBeforeCreate");
    expect(html).toContain("settings:worktrees.autoPrune");
    expect(html).toContain("settings:worktrees.autoPruneLimit");
    expect(html).toContain("settings:worktrees.listEmpty");
    expect(html).toContain('value="15"');
    // Ensure duplicate standalone hint is removed
    expect(html).not.toContain(
      '<p class="text-xs text-muted-foreground">settings:worktrees.hint</p>',
    );
  });

  it("shows the configured worktree root", () => {
    useSettingsStore.setState({
      native: { ...sampleNative(), worktree_root: "/data/nox-wt" },
    });
    const html = renderToStaticMarkup(<WorktreesSection />);
    expect(html).toContain("/data/nox-wt");
  });

  it("renders worktree items with title, badges, workspace tag, and path copy/open buttons", () => {
    const sampleList = {
      root: "/cfg/worktrees",
      default_root: "/cfg/worktrees",
      items: [
        {
          session_id: "s1",
          title: "增加ok2接口",
          workspace_id: "ws1",
          workspace_name: "gb-oms-1",
          status: "active",
          path: "/cfg/worktrees/wt-s1",
          exists: true,
          remote: false,
          in_use: true,
          created_at: "2026-03-01T10:00:00Z",
        },
        {
          session_id: "s2",
          title: "测试远程树",
          workspace_id: "ws2",
          workspace_name: "remote-ws",
          status: "idle",
          path: "/cfg/worktrees/wt-s2",
          exists: true,
          remote: true,
          in_use: false,
          created_at: "2026-03-02T10:00:00Z",
        },
        {
          session_id: "s3",
          title: "缺失目录树",
          workspace_id: null,
          workspace_name: null,
          status: "idle",
          path: "/cfg/worktrees/wt-s3",
          exists: false,
          remote: false,
          in_use: false,
          created_at: "2026-03-03T10:00:00Z",
        },
      ],
    };

    const html = renderToStaticMarkup(<WorktreesSection initialList={sampleList} />);

    // Check card title and count
    expect(html).toContain("settings:worktrees.managedTitle");
    expect(html).toContain(">3<");

    // Check items content
    expect(html).toContain("增加ok2接口");
    expect(html).toContain("gb-oms-1");
    expect(html).toContain("settings:worktrees.inUse");
    expect(html).toContain("/cfg/worktrees/wt-s1");

    expect(html).toContain("测试远程树");
    expect(html).toContain("remote-ws");
    expect(html).toContain("settings:worktrees.remote");

    expect(html).toContain("缺失目录树");
    expect(html).toContain("settings:worktrees.missing");

    // Check copy path and open in folder buttons
    expect(html).toContain("settings:worktrees.copyPath");
    expect(html).toContain("settings:worktrees.openInFolder");

    // In use item has tooltip and disabled attribute
    expect(html).toContain("settings:worktrees.inUseTooltip");
    expect(html).toContain("settings:worktrees.delete");
  });
});
