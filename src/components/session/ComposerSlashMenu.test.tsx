import { FileIcon } from "lucide-react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ComposerSlashItem } from "@/lib/composerSlash";
import { ComposerMentionOption } from "./ComposerMentionMenu";
import { ComposerSlashMenu } from "./ComposerSlashMenu";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const items: ComposerSlashItem[] = [
  {
    key: "command:init",
    group: "commands",
    name: "init",
    token: "/init",
    description: "Generate AGENTS.md",
    argumentHint: "AGENTS.md",
  },
  {
    key: "skill:review",
    group: "skills",
    name: "review",
    token: "$review",
    description: "Review workspace changes",
    sourceLabel: "workspace",
  },
  {
    key: "subagent:architect",
    group: "subagents",
    name: "architect",
    token: "delegate to architect",
    description: "Architecture review",
  },
];

describe("composer suggestions", () => {
  it("renders compact options with stable IDs and one selected row across groups", () => {
    const html = renderToStaticMarkup(
      <ComposerSlashMenu
        items={items}
        activeIndex={1}
        listId="suggestions"
        emptyLabel="No matches"
        onHover={vi.fn()}
        onPick={vi.fn()}
      />,
    );

    expect(html.match(/role="option"/g)).toHaveLength(3);
    expect(html.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(html).toMatch(/id="suggestions-1"[^>]*aria-selected="true"/);
    expect(html.match(/tabindex="-1"/g)).toHaveLength(3);
    expect(html).toContain("/init");
    expect(html).toContain("AGENTS.md");
    expect(html).toContain("$review");
    expect(html).toContain("architect");
    expect(html).toContain("Review workspace changes");
    expect(html).toContain("workspace");
    expect(html).toContain('role="group" aria-label="slashSkills"');
    expect(html).not.toMatch(/<p(?:\s|>)/);
  });

  it("renders the trigger-specific empty state without nesting another panel", () => {
    const html = renderToStaticMarkup(
      <ComposerSlashMenu
        items={[]}
        activeIndex={0}
        listId="skills"
        emptyLabel="No matching skills"
        onHover={vi.fn()}
        onPick={vi.fn()}
      />,
    );

    expect(html).toContain('role="status"');
    expect(html).toContain("No matching skills");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("bg-popover");
  });

  it("uses the same option semantics for file mentions and preserves the full path", () => {
    const path = "src/components/session/Composer.tsx";
    const html = renderToStaticMarkup(
      <ComposerMentionOption id="files-0" active icon={FileIcon} label={path} />,
    );

    expect(html).toContain('role="option"');
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('tabindex="-1"');
    expect(html).toContain(`title="${path}"`);
    expect(html).toContain(path);
  });
});
