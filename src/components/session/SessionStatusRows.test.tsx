import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { WorktreeStatusRow } from "./SessionStatusRows";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => (key === "worktreeIsolated" ? "已隔离到独立工作树" : key),
  }),
}));

describe("WorktreeStatusRow", () => {
  it("renders the isolated worktree line like other status rows", () => {
    const html = renderToStaticMarkup(
      <WorktreeStatusRow text="[WORKTREE] 会话工作目录已隔离到 /Users/me/.noxcode/worktrees/abc-1" />,
    );
    expect(html).toContain("已隔离到独立工作树");
    expect(html).not.toContain("[WORKTREE]");
    expect(html).toContain("lucide-git-fork");
  });
});
