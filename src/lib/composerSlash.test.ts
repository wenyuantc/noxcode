import { describe, expect, it } from "vitest";

import {
  builtinSlashCommands,
  filterComposerSlashItems,
  groupComposerSlashItems,
  isBuiltinSlashName,
  parseComposerTrigger,
  parseLeadingSlash,
  parseSkillInvocation,
  skillInvocationPrompt,
  subagentDelegationPrompt,
} from "./composerSlash";

describe("parseComposerTrigger", () => {
  it("detects @ / and $ on the last token", () => {
    expect(parseComposerTrigger("hello @src")).toEqual({ kind: "@", query: "src" });
    expect(parseComposerTrigger("/re")).toEqual({ kind: "/", query: "re" });
    expect(parseComposerTrigger("use $rev")).toEqual({ kind: "$", query: "rev" });
    expect(parseComposerTrigger("plain text")).toBeNull();
  });
});

describe("filter and group slash items", () => {
  const items = [
    ...builtinSlashCommands({ init: "init repo", fork: "fork", compact: "compact" }),
    {
      group: "skills" as const,
      key: "skill:review",
      name: "review",
      description: "Review diffs",
      token: "$review",
    },
    {
      group: "subagents" as const,
      key: "agent:explore",
      name: "explore",
      description: "Read only",
      token: "agent",
    },
  ];

  it("filters by name or description", () => {
    expect(filterComposerSlashItems(items, "rev").map((item) => item.name)).toEqual(["review"]);
    expect(filterComposerSlashItems(items, "init").map((item) => item.name)).toEqual(["init"]);
  });

  it("keeps group order and drops empty groups", () => {
    const grouped = groupComposerSlashItems(filterComposerSlashItems(items, "expl"));
    expect(grouped.map((section) => section.group)).toEqual(["subagents"]);
    expect(groupComposerSlashItems(items).map((section) => section.group)).toEqual([
      "commands",
      "skills",
      "subagents",
    ]);
  });
});

describe("send parsers", () => {
  it("parses $skill and /skill invocations", () => {
    expect(parseSkillInvocation("$review extra")).toEqual({ name: "review", args: "extra" });
    expect(parseSkillInvocation("/skill review --strict")).toEqual({
      name: "review",
      args: "--strict",
    });
    expect(parseSkillInvocation("/init")).toBeNull();
  });

  it("parses a leading slash command", () => {
    expect(parseLeadingSlash("/frontend:component Button")).toEqual({
      name: "frontend:component",
      args: "Button",
    });
  });

  it("builds skill and subagent prompts", () => {
    expect(skillInvocationPrompt("review", "pr 12")).toContain("`review`");
    expect(skillInvocationPrompt("review", "pr 12")).toContain("pr 12");
    expect(subagentDelegationPrompt("Explore", "explore")).toContain("subagent_type=explore");
  });

  it("recognizes builtin slash names", () => {
    expect(isBuiltinSlashName("init")).toBe(true);
    expect(isBuiltinSlashName("skill")).toBe(true);
    expect(isBuiltinSlashName("frontend:component")).toBe(false);
  });
});
