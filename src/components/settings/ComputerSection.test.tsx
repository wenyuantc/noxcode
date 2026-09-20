import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ComputerPermissionStatus, NativeSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { ComputerSection, computerFlagLabel } from "./ComputerSection";

vi.mock("@/lib/backend", () => ({
  getComputerPermissionStatus: vi.fn().mockResolvedValue({
    platform: "linux",
    session_type: "x11",
    screenshot: { granted: null, label: "X11 截屏", detail: "X11" },
    input: { granted: null, label: "X11 键鼠注入", detail: "X11" },
    can_open_settings: false,
    hint: "risk",
  } satisfies ComputerPermissionStatus),
  openComputerPrivacySettings: vi.fn(),
  updateNativeSettings: vi.fn(),
  getNativePermissionRules: vi.fn().mockResolvedValue({
    global: { allow: [], deny: [], ask: [] },
    workspace: { allow: [], deny: [], ask: [] },
    workspace_root: null,
  }),
  deleteNativePermissionRule: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "zh-CN" },
  }),
}));

vi.mock("@/stores/workspaceStore", () => ({
  useWorkspaceStore: (selector: (state: { activeWorkspaceId: string | null }) => unknown) =>
    selector({ activeWorkspaceId: null }),
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

describe("ComputerSection", () => {
  beforeEach(() => {
    useSettingsStore.setState({ native: sampleNative() });
  });

  it("renders the master switch and permission cards", () => {
    const html = renderToStaticMarkup(<ComputerSection />);
    expect(html).toContain("settings:computer.enabled");
    expect(html).toContain("settings:computer.enabledHint");
    expect(html).toContain("settings:computer.permissionTitle");
    expect(html).toContain("settings:computer.approvedTitle");
    expect(html).toContain("settings:computer.platformTitle");
    expect(html).toContain("settings:computer.riskTitle");
    expect(html).toContain("settings:computer.backgroundHint");
    expect(html).toContain('id="native-computer-enabled"');
  });
});

describe("computerFlagLabel", () => {
  it("uses the backend label", () => {
    expect(computerFlagLabel({ granted: false, label: "未授权屏幕录制", detail: "need" })).toBe(
      "未授权屏幕录制",
    );
  });
});
