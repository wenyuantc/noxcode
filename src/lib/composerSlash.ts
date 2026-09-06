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
  return (BUILTIN_SLASH_NAMES as readonly string[]).includes(name.toLowerCase());
}

export function subagentDelegationPrompt(name: string, id: string): string {
  return `请用 Agent 工具委派给子智能体「${name}」（subagent_type=${id}）：`;
}

/** `/init [补充要求]`：摸底仓库后生成或补充 AGENTS.md。 */
export function buildInitPrompt(extra?: string): string {
  const lines = [
    "请为当前仓库生成或补充 AGENTS.md（若已有 AGENTS.md / CLAUDE.md 则在其基础上补充，不要重复已有内容）。",
    "先用 Glob / Read / Grep 摸底：项目结构与模块职责、构建 / 测试 / lint 命令、编码约定、关键架构约束、常见陷阱。",
    "输出要求：简洁、面向编程 Agent、只写能从仓库验证的事实；每条命令都注明来源文件；不超过 150 行。",
    "完成后用 Write 写入仓库根目录的 AGENTS.md，并在回复里列出你新增或修改的段落。",
  ];
  if (extra?.trim()) lines.push(`补充要求：${extra.trim()}`);
  return lines.join("\n");
}

export function parseNamedSlashArgs(args: string): { name: string; rest: string } | null {
  const match = /^([^\s]+)(?:\s+([\s\S]*))?$/.exec(args.trim());
  if (!match?.[1]) return null;
  return { name: match[1], rest: (match[2] ?? "").trim() };
}

export function buildCreateSkillPrompt(name: string, description?: string): string {
  const lines = [
    `请为当前仓库编写技能「${name}」，用 Write 写入 \`.noxcode/skills/${name}/SKILL.md\`。`,
    "先用 Glob / Grep 确认是否已有同名技能；已有则在其基础上补充，不要覆盖无关内容。",
    "frontmatter 必须包含：name、description、argument-hint、allowed-tools、when-to-use。",
    "正文写可执行工作流、约束和输出格式，不要只复述描述。",
    "完成后在回复里说明如何用 `$" + name + "` 或 `/skill " + name + "` 调用。",
  ];
  if (description?.trim()) lines.push(`技能说明：${description.trim()}`);
  return lines.join("\n");
}

export function buildCreateSubagentPrompt(name: string, description?: string): string {
  const lines = [
    `请为当前仓库编写子智能体「${name}」，用 Write 写入 \`.noxcode/agents/${name}.md\`。`,
    "这是 source=file 的档案（设置页只读），不要改 native-subagents.json。",
    "frontmatter 对齐现有解析：name、description、tools、disallowedTools、permissionMode、maxTurns、skills、injectAgentsMd。",
    "正文即系统提示：单一职责、何时委派、输出格式。",
    "写完后说明可在输入框 `/` 菜单的「子智能体」分组里选到。",
  ];
  if (description?.trim()) lines.push(`子智能体说明：${description.trim()}`);
  return lines.join("\n");
}

export function buildGoalPrompt(args?: string): string {
  const parsed = parseGoalSlashArgs(args ?? "");
  if (parsed.action === "clear") {
    return "请调用 Goal 工具清除当前会话目标：Goal(action=clear)。不要改业务代码。";
  }
  const lines = [
    "请调用 Goal 工具维护当前会话目标。可先 GoalRead 查看是否已有目标。",
    "设置或替换时用 Goal(action=set, title=..., checklist?, note?)。",
    "不要改业务代码，只维护目标与进度清单。",
  ];
  if (parsed.title) lines.push(`目标：${parsed.title}`);
  return lines.join("\n");
}

export function buildReviewPrompt(scope?: string): string {
  const lines = [
    "请审查当前工作区未提交改动（已暂存、未暂存、未跟踪）。",
    "先看 git 状态与 diff，再列出正确性、测试缺口和风险。",
    "不要直接改代码，除非我明确要求修复。",
  ];
  if (scope?.trim()) lines.push(`审查范围：${scope.trim()}`);
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
