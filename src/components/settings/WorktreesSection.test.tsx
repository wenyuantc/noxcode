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
    expect(html).toContain("settings:worktrees.root");
    expect(html).toContain("settings:worktrees.fetchBeforeCreate");
    expect(html).toContain("settings:worktrees.autoPrune");
    expect(html).toContain("settings:worktrees.autoPruneLimit");
    expect(html).toContain("settings:worktrees.listEmpty");
    expect(html).toContain('value="15"');
  });

  it("shows the configured worktree root", () => {
    useSettingsStore.setState({
      native: { ...sampleNative(), worktree_root: "/data/nox-wt" },
    });
    const html = renderToStaticMarkup(<WorktreesSection />);
    expect(html).toContain("/data/nox-wt");
  });
});
