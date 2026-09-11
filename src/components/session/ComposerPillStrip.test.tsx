import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ComposerPillsState } from "@/lib/composerPills";
import { ComposerPillStrip } from "./ComposerPillStrip";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, opt?: { name?: string }) => opt?.name ?? key }),
}));

describe("ComposerPillStrip", () => {
  it("renders nothing when there are no pills", () => {
    const pills: ComposerPillsState = { target: null, files: [] };
    const html = renderToStaticMarkup(
      <ComposerPillStrip pills={pills} onRemoveTarget={vi.fn()} onRemoveFile={vi.fn()} />,
    );
    expect(html).toBe("");
  });

  it("renders skill pill with package icon and remove button", () => {
    const pills: ComposerPillsState = {
      target: {
        kind: "skill",
        name: "test-runner",
        description: "Run test suite",
        sourceLabel: "workspace",
        token: "$test-runner",
      },
      files: [],
    };
    const html = renderToStaticMarkup(
      <ComposerPillStrip pills={pills} onRemoveTarget={vi.fn()} onRemoveFile={vi.fn()} />,
    );
    expect(html).toContain("test-runner");
    expect(html).toContain("workspace");
    expect(html).toContain("border-cyan-500");
  });

  it("renders subagent pill and file pills together", () => {
    const pills: ComposerPillsState = {
      target: {
        kind: "subagent",
        id: "agent-1",
        name: "Reviewer",
        description: "Code reviewer",
        token: "reviewer",
      },
      files: ["src/auth/login.ts"],
    };
    const html = renderToStaticMarkup(
      <ComposerPillStrip pills={pills} onRemoveTarget={vi.fn()} onRemoveFile={vi.fn()} />,
    );
    expect(html).toContain("Reviewer");
    expect(html).toContain("border-purple-500");
    expect(html).toContain("login.ts");
  });
});
