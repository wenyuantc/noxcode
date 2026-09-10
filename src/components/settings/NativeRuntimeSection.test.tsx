import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NativeSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { NativeRuntimeSection } from "./NativeRuntimeSection";

vi.mock("@/lib/backend", () => ({
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

describe("NativeRuntimeSection", () => {
  beforeEach(() => {
    useSettingsStore.setState({ native: sampleNative() });
  });

  it("no longer hosts LSP toggle or install UI", () => {
    const html = renderToStaticMarkup(<NativeRuntimeSection />);
    expect(html).toContain("settings:runtime.toolRuntime");
    expect(html).toContain("settings:runtime.bashSandbox");
    expect(html).not.toContain("settings:runtime.lsp");
    expect(html).not.toContain("settings:lsp.");
    expect(html).not.toContain('id="native-lsp-enabled"');
  });
});
