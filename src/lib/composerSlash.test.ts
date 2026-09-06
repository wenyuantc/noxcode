import { describe, expect, it } from "vitest";

import {
  builtinSlashCommands,
  buildCreateSkillPrompt,
  buildCreateSubagentPrompt,
  buildGoalPrompt,
  buildInitPrompt,
  buildReviewPrompt,
  filterComposerSlashItems,
  groupComposerSlashItems,
  isBuiltinSlashName,
  parseComposerTrigger,
  parseGoalSlashArgs,
  parseLeadingSlash,
  parseNamedSlashArgs,
  parseSkillInvocation,
  skillInvocationPrompt,
  subagentDelegationPrompt,
  type BuiltinSlashName,
} from "./composerSlash";

const labels = (name: BuiltinSlashName) => ({
  description: `${name} desc`,
  hint: name === "init" ? "AGENTS.md" : undefined,
});

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
    ...builtinSlashCommands(labels),
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

  it("filters by name, description or argument hint", () => {
    expect(filterComposerSlashItems(items, "AGENTS.md").map((item) => item.name)).toEqual(["init"]);
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

  it("lists all builtin commands", () => {
    expect(builtinSlashCommands(labels).map((item) => item.name)).toContain("create-skill");
    expect(builtinSlashCommands(labels).map((item) => item.name)).toContain("create-subagent");
    expect(builtinSlashCommands(labels).map((item) => item.name)).toContain("help");
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

  it("parses create-skill / create-subagent names", () => {
    expect(parseNamedSlashArgs("code-review 审查 diff")).toEqual({
      name: "code-review",
      rest: "审查 diff",
    });
    expect(parseNamedSlashArgs("")).toBeNull();
  });

  it("builds skill and subagent prompts", () => {
    expect(skillInvocationPrompt("review", "pr 12")).toContain("`review`");
    expect(skillInvocationPrompt("review", "pr 12")).toContain("pr 12");
    expect(subagentDelegationPrompt("Explore", "explore")).toContain("subagent_type=explore");
  });

  it("recognizes builtin slash names", () => {
    expect(isBuiltinSlashName("init")).toBe(true);
    expect(isBuiltinSlashName("skill")).toBe(true);
    expect(isBuiltinSlashName("create-skill")).toBe(true);
    expect(isBuiltinSlashName("create-subagent")).toBe(true);
    expect(isBuiltinSlashName("frontend:component")).toBe(false);
  });
});

describe("prompt builders", () => {
  it("builds init / goal / review / create prompts", () => {
    expect(buildInitPrompt("补充")).toContain("AGENTS.md");
    expect(buildInitPrompt("补充")).toContain("补充要求：补充");
    expect(buildGoalPrompt("clear")).toContain("Goal(action=clear)");
    expect(buildGoalPrompt("修登录")).toContain("修登录");
    expect(buildReviewPrompt("src/lib")).toContain("src/lib");
    expect(buildCreateSkillPrompt("code-review", "审 diff")).toContain(
      ".noxcode/skills/code-review/SKILL.md",
    );
    expect(buildCreateSkillPrompt("code-review")).toContain("when-to-use");
    expect(buildCreateSubagentPrompt("explore", "只读")).toContain(".noxcode/agents/explore.md");
    expect(buildCreateSubagentPrompt("explore")).toContain("injectAgentsMd");
  });

  it("parses goal args", () => {
    expect(parseGoalSlashArgs("clear")).toEqual({ action: "clear" });
    expect(parseGoalSlashArgs("set 修登录")).toEqual({ action: "set", title: "修登录" });
    expect(parseGoalSlashArgs("修登录")).toEqual({ action: "set", title: "修登录" });
  });
});
