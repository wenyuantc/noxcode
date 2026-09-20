import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LspServerStatus, LspTestResult, NativeSettings } from "@/lib/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { LspSection, lspRowAction, lspTestSummary, lspTestVariant } from "./LspSection";

vi.mock("@/lib/backend", () => ({
  listLspServers: vi.fn().mockResolvedValue([]),
  installLspServer: vi.fn(),
  testLspServer: vi.fn(),
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

describe("LspSection", () => {
  beforeEach(() => {
    useSettingsStore.setState({ native: sampleNative() });
  });

  it("renders the enable switch and install card", () => {
    const html = renderToStaticMarkup(<LspSection />);
    expect(html).toContain("settings:lsp.enabled");
    expect(html).toContain("settings:lsp.enabledHint");
    expect(html).toContain("settings:lsp.installTitle");
    expect(html).toContain("settings:lsp.installHint");
    expect(html).toContain("settings:lsp.loading");
    expect(html).toContain('id="native-lsp-enabled"');
  });
});

describe("lspRowAction", () => {
  const server = (overrides: Partial<LspServerStatus>): LspServerStatus => ({
    id: "rust",
    label: "Rust",
    commands: ["rust-analyzer"],
    installed_command: null,
    install_command: "rustup component add rust-analyzer",
    installable: true,
    ...overrides,
  });

  it("offers the test button for installed servers", () => {
    expect(lspRowAction(server({ installed_command: "rust-analyzer" }))).toBe("test");
  });

  it("falls back to install, then manual", () => {
    expect(lspRowAction(server({}))).toBe("install");
    expect(lspRowAction(server({ installable: false, install_command: null }))).toBe("manual");
  });
});

describe("lspTestSummary", () => {
  const base: LspTestResult = {
    language: "rust",
    label: "Rust",
    command: "rust-analyzer",
    server_name: null,
    server_version: null,
    elapsed_ms: 820,
    warning: null,
  };

  it("prefers the reported server name and version", () => {
    expect(lspTestSummary({ ...base, server_name: "rust-analyzer", server_version: "1.2.3" })).toBe(
      "rust-analyzer 1.2.3 · 820 ms",
    );
  });

  it("falls back to name only, then to the command", () => {
    expect(lspTestSummary({ ...base, server_name: "gopls" })).toBe("gopls · 820 ms");
    expect(lspTestSummary(base)).toBe("rust-analyzer · 820 ms");
  });
});

describe("lspTestVariant", () => {
  const base: LspTestResult = {
    language: "rust",
    label: "Rust",
    command: "rust-analyzer",
    server_name: null,
    server_version: null,
    elapsed_ms: 820,
    warning: null,
  };

  it("reports a clean test as success", () => {
    expect(lspTestVariant(base)).toBe("success");
  });

  it("downgrades to warning when the language server reports one", () => {
    expect(lspTestVariant({ ...base, warning: "workspace/symbol call failed" })).toBe("warning");
  });
});
