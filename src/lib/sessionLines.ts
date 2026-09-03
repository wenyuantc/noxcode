export type SessionLineKind = "user" | "assistant" | "tool" | "tool_result" | "system" | "error";

export interface RawSessionLine {
  id: string;
  sessionId: string;
  text: string;
  createdAt: string;
}

export interface GroupedSessionItem {
  id: string;
  kind: SessionLineKind;
  text: string;
  createdAt: string;
  toolName?: string;
  result?: string;
}

export interface SessionTurnBlock {
  id: string;
  user?: GroupedSessionItem;
  tools: GroupedSessionItem[];
  assistant: GroupedSessionItem[];
  system: GroupedSessionItem[];
  startedAt: string;
  endedAt: string;
}

const TOOL_PREFIXES = [
  "[读取]",
  "[写入]",
  "[编辑]",
  "[命令]",
  "[工具]",
  "[技能]",
  "[待办]",
  "[补丁]",
];
const SYSTEM_PREFIXES = [
  "[思考]",
  "[PLAN]",
  "[计划]",
  "[PERMISSION]",
  "[MCP]",
  "[续聊]",
  "[重试]",
  "[用量]",
];

export function classifyLine(text: string): SessionLineKind {
  const line = text.trimStart();
  if (line.startsWith("[USER_INPUT]") || line.startsWith("[用户输入]")) return "user";
  if (line.startsWith("[ERROR]")) return "error";
  if (line.startsWith("[工具结果]")) return "tool_result";
  if (line.startsWith("[子 Agent")) return "tool";
  if (TOOL_PREFIXES.some((prefix) => line.startsWith(prefix))) return "tool";
  if (SYSTEM_PREFIXES.some((prefix) => line.startsWith(prefix))) return "system";
  if (line.startsWith("[") && line.includes("]")) return "system";
  return "assistant";
}

export function toolTitle(text: string): string {
  const first = text.split("\n")[0] ?? text;
  return first.replace(/^\[|\]$/g, "").trim() || first;
}

export function stripUserPrefix(text: string): string {
  return text.replace(/^\[USER_INPUT\]\s*/, "").replace(/^\[用户输入\]\s*/, "");
}

export function groupSessionLines(lines: RawSessionLine[]): GroupedSessionItem[] {
  const grouped: GroupedSessionItem[] = [];
  for (const line of lines) {
    const kind = classifyLine(line.text);
    if (kind === "tool_result") {
      const lastTool = [...grouped].reverse().find((item) => item.kind === "tool" && !item.result);
      const result = line.text.replace(/^\[工具结果\]\s*/, "");
      if (lastTool) {
        lastTool.result = result;
        continue;
      }
    }
    grouped.push({
      id: line.id,
      kind,
      text: kind === "user" ? stripUserPrefix(line.text) : line.text,
      createdAt: line.createdAt,
      toolName: kind === "tool" ? toolTitle(line.text) : undefined,
    });
  }
  return grouped;
}

export function buildTurnBlocks(items: GroupedSessionItem[]): SessionTurnBlock[] {
  const blocks: SessionTurnBlock[] = [];
  let current: SessionTurnBlock | null = null;

  const startBlock = (item: GroupedSessionItem): SessionTurnBlock => {
    const block: SessionTurnBlock = {
      id: item.id,
      tools: [],
      assistant: [],
      system: [],
      startedAt: item.createdAt,
      endedAt: item.createdAt,
    };
    blocks.push(block);
    return block;
  };

  for (const item of items) {
    if (item.kind === "user" || !current) {
      current = startBlock(item);
    }
    current.endedAt = item.createdAt;
    if (item.kind === "user") current.user = item;
    else if (item.kind === "tool") current.tools.push(item);
    else if (item.kind === "assistant") current.assistant.push(item);
    else current.system.push(item);
  }
  return blocks;
}

export function workDurationSeconds(block: SessionTurnBlock): number {
  const start = Date.parse(block.startedAt);
  const end = Date.parse(block.endedAt);
  if (Number.isNaN(start) || Number.isNaN(end)) return 0;
  return Math.max(0, Math.round((end - start) / 1000));
}

export function lineToneClass(kind: SessionLineKind, text: string): string {
  if (kind === "error" || text.startsWith("[ERROR]")) return "text-red-600 dark:text-red-400";
  if (kind === "user") return "text-sky-700 dark:text-sky-300";
  if (kind === "tool" || kind === "tool_result") return "text-cyan-700 dark:text-cyan-400";
  if (text.startsWith("[思考]")) return "text-muted-foreground";
  if (text.startsWith("[PLAN]") || text.startsWith("[计划]") || text.startsWith("[待办]")) {
    return "text-violet-700 dark:text-violet-400";
  }
  if (text.startsWith("[子 Agent")) return "text-teal-700 dark:text-teal-400";
  return "text-foreground";
}
