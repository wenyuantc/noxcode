import { promptT } from "@/lib/promptI18n";

export type ComposerSlashGroup = "commands" | "skills" | "subagents";

export interface ComposerSlashItem {
  group: ComposerSlashGroup;
  key: string;
  name: string;
  description: string;
  argumentHint?: string;
  sourceLabel?: string;
  token: string;
}

export type ComposerTrigger =
  { kind: "@"; query: string } | { kind: "/"; query: string } | { kind: "$"; query: string };

export const BUILTIN_SLASH_NAMES = [
  "init",
  "fork",
  "compact",
  "new",
  "clear",
  "mode",
  "model",
  "effort",
  "plan",
  "permissions",
  "memory",
  "mcp",
  "plugins",
  "skill",
  "diff",
  "context",
  "help",
  "goal",
  "review",
  "create-skill",
  "create-subagent",
] as const;

export type BuiltinSlashName = (typeof BUILTIN_SLASH_NAMES)[number];

export type BuiltinSlashLabel = { description: string; hint?: string };

export function parseComposerTrigger(draft: string): ComposerTrigger | null {
  const last = draft.split(/\s/).pop() ?? "";
  if (last.startsWith("@")) return { kind: "@", query: last.slice(1) };
  if (last.startsWith("/")) return { kind: "/", query: last.slice(1) };
  if (last.startsWith("$")) return { kind: "$", query: last.slice(1) };
  return null;
}

export function filterComposerSlashItems(
  items: ComposerSlashItem[],
  query: string,
): ComposerSlashItem[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return items;
  return items.filter((item) => {
    const hay =
      `${item.name} ${item.description} ${item.argumentHint ?? ""} ${item.sourceLabel ?? ""}`.toLowerCase();
    return hay.includes(needle);
  });
}

export function groupComposerSlashItems(
  items: ComposerSlashItem[],
): Array<{ group: ComposerSlashGroup; items: ComposerSlashItem[] }> {
  const order: ComposerSlashGroup[] = ["commands", "skills", "subagents"];
  return order
    .map((group) => ({ group, items: items.filter((item) => item.group === group) }))
    .filter((section) => section.items.length > 0);
}

export function builtinSlashCommands(
  labels: (name: BuiltinSlashName) => BuiltinSlashLabel,
): ComposerSlashItem[] {
  return BUILTIN_SLASH_NAMES.map((name) => {
    const label = labels(name);
    const hint = label.hint?.trim();
    return {
      group: "commands" as const,
      key: `builtin:${name}`,
      name,
      description: label.description,
      argumentHint: hint || undefined,
      token: `/${name}`,
    };
  });
}

export function skillInvocationPrompt(name: string, args?: string): string {
  const lines = [promptT("skillInvocation", { name })];
  if (args?.trim()) lines.push(promptT("arguments", { args: args.trim() }));
  return lines.join("\n");
}

export function parseLeadingSlash(prompt: string): { name: string; args: string } | null {
  const match = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(prompt.trim());
  if (!match) return null;
  return { name: match[1], args: (match[2] ?? "").trim() };
}

export function parseSkillInvocation(prompt: string): { name: string; args: string } | null {
  const trimmed = prompt.trim();
  const dollar = /^\$([^\s]+)(?:\s+([\s\S]*))?$/.exec(trimmed);
  if (dollar) return { name: dollar[1], args: (dollar[2] ?? "").trim() };
  const skill = /^\/skill(?:\s+([^\s]+))(?:\s+([\s\S]*))?$/i.exec(trimmed);
  if (skill?.[1]) return { name: skill[1], args: (skill[2] ?? "").trim() };
  return null;
}

export function isBuiltinSlashName(name: string): boolean {
  return (BUILTIN_SLASH_NAMES as readonly string[]).includes(name.toLowerCase());
}

export function subagentDelegationPrompt(name: string, id: string): string {
  return promptT("subagentDelegation", { name, id });
}

/** `/init [补充要求]`：摸底仓库后生成或补充 AGENTS.md。 */
export function buildInitPrompt(extra?: string): string {
  const lines = [
    promptT("init.intro"),
    promptT("init.inspect"),
    promptT("init.output"),
    promptT("init.write"),
  ];
  if (extra?.trim()) lines.push(promptT("init.extra", { extra: extra.trim() }));
  return lines.join("\n");
}

export function parseNamedSlashArgs(args: string): { name: string; rest: string } | null {
  const match = /^([^\s]+)(?:\s+([\s\S]*))?$/.exec(args.trim());
  if (!match?.[1]) return null;
  return { name: match[1], rest: (match[2] ?? "").trim() };
}

export function buildCreateSkillPrompt(name: string, description?: string): string {
  const lines = [
    promptT("createSkill.intro", { name }),
    promptT("createSkill.inspect"),
    promptT("createSkill.frontmatter"),
    promptT("createSkill.body"),
    promptT("createSkill.finish", { name }),
  ];
  if (description?.trim()) {
    lines.push(promptT("createSkill.description", { description: description.trim() }));
  }
  return lines.join("\n");
}

export function buildCreateSubagentPrompt(name: string, description?: string): string {
  const lines = [
    promptT("createSubagent.intro", { name }),
    promptT("createSubagent.source"),
    promptT("createSubagent.frontmatter"),
    promptT("createSubagent.body"),
    promptT("createSubagent.finish"),
  ];
  if (description?.trim()) {
    lines.push(promptT("createSubagent.description", { description: description.trim() }));
  }
  return lines.join("\n");
}

export function buildGoalPrompt(args?: string): string {
  const parsed = parseGoalSlashArgs(args ?? "");
  if (parsed.action === "clear") {
    return promptT("goalClear");
  }
  const lines = [promptT("goal.intro"), promptT("goal.set"), promptT("goal.guard")];
  if (parsed.title) lines.push(promptT("goal.title", { title: parsed.title }));
  return lines.join("\n");
}

export function buildReviewPrompt(scope?: string): string {
  const lines = [promptT("review.status"), promptT("review.inspect"), promptT("review.guard")];
  if (scope?.trim()) lines.push(promptT("review.scope", { scope: scope.trim() }));
  return lines.join("\n");
}

export function parseGoalSlashArgs(args: string): { action: "set" | "clear"; title?: string } {
  const trimmed = args.trim();
  if (!trimmed) return { action: "set" };
  const [first, ...rest] = trimmed.split(/\s+/);
  const token = (first ?? "").toLowerCase();
  if (["clear", "stop", "off", "reset", "none", "cancel"].includes(token)) {
    return { action: "clear" };
  }
  if (token === "set") {
    const title = rest.join(" ").trim();
    return title ? { action: "set", title } : { action: "set" };
  }
  return { action: "set", title: trimmed };
}
