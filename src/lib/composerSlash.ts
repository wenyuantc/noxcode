export type ComposerSlashGroup = "commands" | "skills" | "subagents";

export interface ComposerSlashItem {
  group: ComposerSlashGroup;
  key: string;
  name: string;
  description: string;
  sourceLabel?: string;
  token: string;
}

export type ComposerTrigger =
  { kind: "@"; query: string } | { kind: "/"; query: string } | { kind: "$"; query: string };

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
    const hay = `${item.name} ${item.description} ${item.sourceLabel ?? ""}`.toLowerCase();
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

export function builtinSlashCommands(labels: {
  init: string;
  fork: string;
  compact: string;
}): ComposerSlashItem[] {
  return [
    {
      group: "commands",
      key: "builtin:init",
      name: "init",
      description: labels.init,
      token: "/init",
    },
    {
      group: "commands",
      key: "builtin:fork",
      name: "fork",
      description: labels.fork,
      token: "/fork",
    },
    {
      group: "commands",
      key: "builtin:compact",
      name: "compact",
      description: labels.compact,
      token: "/compact",
    },
  ];
}

export function skillInvocationPrompt(name: string, args?: string): string {
  const lines = [`请先调用 Skill 工具加载 \`${name}\`，再按该技能执行。`];
  if (args?.trim()) lines.push(`参数：${args.trim()}`);
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
  return /^(init|fork|compact|skill)$/i.test(name);
}

export function subagentDelegationPrompt(name: string, id: string): string {
  return `请用 Agent 工具委派给子智能体「${name}」（subagent_type=${id}）：`;
}
