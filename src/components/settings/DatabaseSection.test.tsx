import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { healthCheck } from "@/lib/backend";
import type { AppHealthCheck } from "@/lib/types";
import { DatabaseSection } from "./DatabaseSection";

vi.mock("@/lib/backend", () => ({
  backupDatabase: vi.fn(),
  healthCheck: vi.fn(),
  openDatabaseFolder: vi.fn(),
  restoreDatabase: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
  message: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}));

vi.mock("@/lib/toast", () => ({
  showToast: vi.fn(),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { returnObjects?: boolean }) => {
      if (opts?.returnObjects) return [];
      return key;
    },
    i18n: { language: "zh-CN" },
  }),
}));

function sampleHealth(): AppHealthCheck {
  return {
    database_loaded: true,
    database_path: "/data/noxcode.db",
    database_current_version: 12,
    database_current_description: "schema v12",
    database_latest_version: 12,
    database_stats: null,
    git_available: true,
    git_version: "2.43.0",
    checked_at: "2026-03-01T10:00:00Z",
    media_notice: null,
  };
}

describe("DatabaseSection", () => {
  beforeEach(() => {
    vi.mocked(healthCheck).mockReset().mockResolvedValue(sampleHealth());
  });

  it("renders the health card and backup/restore actions without layout feedback", () => {
    const html = renderToStaticMarkup(<DatabaseSection />);

    expect(html).toContain("database.maintenance.title");
    expect(html).toContain("备份与还原");
    expect(html).toContain("database.actions.exportSql");
    expect(html).toContain("database.actions.importSql");
    // 备份/恢复的操作结果与失败已改为 toast，页面内只保留加载失败与恢复结果信息。
    expect(html).not.toContain('role="status"');
  });
});
