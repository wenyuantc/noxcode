import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { SshSettingsSection } from "./SshSettingsSection";

vi.mock("@/lib/backend", () => ({
  createSshConfig: vi.fn(),
  deleteSshConfig: vi.fn(),
  importSshConfigFileHost: vi.fn(),
  listSshConfigFileHosts: vi.fn(),
  listSshConfigs: vi.fn(),
  listSshSupportedAlgorithms: vi.fn(),
  probeSshPasswordAuth: vi.fn(),
  testSshConnection: vi.fn(),
  updateSshConfig: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@/lib/toast", () => ({
  showToast: vi.fn(),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    i18n: { language: "zh-CN" },
  }),
}));

describe("SshSettingsSection", () => {
  it("renders the ssh cards without a page-level operation message bar", () => {
    const html = renderToStaticMarkup(<SshSettingsSection />);

    expect(html).toContain("ssh.title");
    expect(html).toContain("ssh.list.loading");
    expect(html).toContain("ssh.import.title");
    // 保存/删除与测试、探测结果已改为 toast，页面操作消息条不再保留反馈容器。
    expect(html).not.toContain('role="status"');
  });
});
