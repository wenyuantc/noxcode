import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { GitFilePreview } from "@/lib/types";
import { GitFilePreviewBody } from "./GitDiffDialog";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/components/code/CodeBlock", () => ({
  CodeBlock: ({ code, language }: { code: string; language: string }) => (
    <pre data-language={language}>{code}</pre>
  ),
}));
vi.mock("./DiffView", () => ({
  DiffView: ({ diff }: { diff: { patch: string } }) => <pre>{diff.patch}</pre>,
}));

const contentPreview: Extract<GitFilePreview, { kind: "content" }> = {
  kind: "content",
  path: ".workflow/.scratchpad/level-scope.json",
  content: '{"scope":"review"}',
  is_binary: false,
  truncated: false,
  reason: "ignored",
};

describe("file preview states", () => {
  it.each(["ignored", "unchanged", "not_repository"] as const)(
    "shows current contents with the %s reason instead of an empty diff",
    (reason) => {
      const html = renderToStaticMarkup(
        <GitFilePreviewBody preview={{ ...contentPreview, reason }} />,
      );
      expect(html).toContain(`previewReason.${reason}`);
      expect(html).toContain('data-language="json"');
      expect(html).toContain("review");
      expect(html).not.toContain("emptyDiff");
    },
  );

  it("distinguishes a missing file from an empty file", () => {
    const missing = renderToStaticMarkup(
      <GitFilePreviewBody preview={{ kind: "missing", path: "deleted.json" }} />,
    );
    const empty = renderToStaticMarkup(
      <GitFilePreviewBody preview={{ ...contentPreview, content: "" }} />,
    );
    expect(missing).toContain("fileMissing");
    expect(missing).not.toContain("fileEmpty");
    expect(empty).toContain("fileEmpty");
    expect(empty).not.toContain("fileMissing");
  });

  it("shows binary and truncation states without treating binary data as code", () => {
    const binary = renderToStaticMarkup(
      <GitFilePreviewBody preview={{ ...contentPreview, content: "", is_binary: true }} />,
    );
    expect(binary).toContain("binary");
    expect(binary).not.toContain("<pre");
    const truncated = renderToStaticMarkup(
      <GitFilePreviewBody preview={{ ...contentPreview, truncated: true }} />,
    );
    expect(truncated).toContain("contentTruncated");
    expect(truncated).toContain("review");
  });

  it("preserves explicit diff and empty-diff rendering", () => {
    const preview: GitFilePreview = {
      kind: "diff",
      scope: "staged",
      diff: { path: "a.txt", old_path: null, patch: "+change", is_binary: false, truncated: false },
    };
    expect(renderToStaticMarkup(<GitFilePreviewBody preview={preview} />)).toContain("+change");
    expect(
      renderToStaticMarkup(
        <GitFilePreviewBody preview={{ ...preview, diff: { ...preview.diff, patch: "" } }} />,
      ),
    ).toContain("emptyDiff");
  });
});
