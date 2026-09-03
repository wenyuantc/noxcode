export type SessionLineKind = "user" | "assistant" | "tool" | "tool_result" | "system" | "error";

export type TurnSegmentKind =
  | "thinking"
  | "tools"
  | "terminal"
  | "file"
  | "changes"
  | "todo"
  | "assistant"
  | "system"
  | "usage";

export interface ParsedUsage {
  input?: number;
  output?: number;
  cache?: number;
  reasoning?: number;
  total?: number;
}

export interface ReadResultLine {
  line: number | null;
  text: string;
}

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

export interface TurnSegment {
  kind: TurnSegmentKind;
  items: GroupedSessionItem[];
}

export interface SessionTurnBlock {
  id: string;
  user?: GroupedSessionItem;
  tools: GroupedSessionItem[];
  assistant: GroupedSessionItem[];
  system: GroupedSessionItem[];
  segments: TurnSegment[];
  startedAt: string;
  endedAt: string;
}

export interface ToolSummary {
  files: number;
  lists: number;
  searches: number;
}

export type TodoStatus = "pending" | "in_progress" | "completed";

export interface ParsedTodoItem {
  status: TodoStatus;
  content: string;
  priority: string;
}

export interface ParsedTodoList {
  items: ParsedTodoItem[];
  current: ParsedTodoItem | null;
  completed: number;
  total: number;
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

const TODO_STATUSES: TodoStatus[] = ["pending", "in_progress", "completed"];
const TODO_LINE_RE = /^- \[(\w+)\] (.+) \(([^)]+)\)\s*$/;

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

export function commandText(item: GroupedSessionItem): string {
  return item.text.replace(/^\[命令\]\s*/, "").split("\n")[0] ?? "";
}

export function filePathText(item: GroupedSessionItem): string {
  return item.text.replace(/^\[(?:写入|编辑|补丁)\]\s*/, "").split("\n")[0] ?? "";
}

export function lookupPathText(item: GroupedSessionItem): string {
  return item.text.replace(/^\[(?:读取|工具)\]\s*/, "").split("\n")[0] ?? "";
}

export function parseReadResultLines(result: string): ReadResultLine[] {
  return result.split("\n").map((raw) => {
    const match = raw.match(/^\s*(\d+)\t(.*)$/);
    if (!match) return { line: null, text: raw };
    return { line: Number(match[1]), text: match[2] ?? "" };
  });
}

export function permissionHint(text: string): string | null {
  const line = text.trim();
  if (!line.startsWith("[PERMISSION]")) return null;
  return line.slice("[PERMISSION]".length).trim();
}

export interface ParsedAgentBanner {
  channel: string;
  protocol: string;
  model: string;
  effort: string;
  thinking: boolean;
}

const AGENT_BANNER_RE =
  /^\[内置 Agent\] 启动会话 渠道=(\S+) 协议=(\S+) model=(\S+) effort=(\S+) thinking=(on|off)$/;

export function parseAgentBanner(text: string): ParsedAgentBanner | null {
  const match = text.trim().match(AGENT_BANNER_RE);
  if (!match) return null;
  return {
    channel: match[1] ?? "",
    protocol: match[2] ?? "",
    model: match[3] ?? "",
    effort: match[4] ?? "",
    thinking: match[5] === "on",
  };
}

export function stripAgentPrefix(text: string): string {
  return text.replace(/^\[内置 Agent\]\s*/, "");
}

export type ParsedMcpStatus =
  | { kind: "off" }
  | { kind: "on"; servers: string[] }
  | { kind: "pending"; count: number }
  | { kind: "error"; detail: string }
  | { kind: "info"; detail: string };

const MCP_PENDING_RE = /^将连接 (\d+) 个已启用服务器$/;
const MCP_CONNECTED_RE = /^已连接：(.+)$/;

export function parseMcpStatus(text: string): ParsedMcpStatus | null {
  const line = text.trim();
  if (!line.startsWith("[MCP]")) return null;
  const detail = line.slice("[MCP]".length).trim();
  if (detail === "未启用服务器") return { kind: "off" };
  const pending = detail.match(MCP_PENDING_RE);
  if (pending) return { kind: "pending", count: Number(pending[1]) };
  const connected = detail.match(MCP_CONNECTED_RE);
  if (connected) {
    return {
      kind: "on",
      servers: (connected[1] ?? "")
        .split("、")
        .map((name) => name.trim())
        .filter(Boolean),
    };
  }
  if (
    detail.includes("没有成功连接") ||
    detail.includes("无法连接") ||
    detail.includes("握手失败") ||
    detail.includes("读取配置失败")
  ) {
    return { kind: "error", detail };
  }
  return { kind: "info", detail };
}

export function parseUsageLine(text: string): ParsedUsage | null {
  const line = text.trim();
  if (!line.startsWith("[用量]")) return null;
  const parsed: ParsedUsage = {};
  for (const part of line.slice("[用量]".length).trim().split(/\s+/)) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const key = part.slice(0, eq);
    const value = Number(part.slice(eq + 1));
    if (!Number.isFinite(value)) continue;
    if (key === "in") parsed.input = value;
    else if (key === "out") parsed.output = value;
    else if (key === "cache") parsed.cache = value;
    else if (key === "reason") parsed.reasoning = value;
    else if (key === "total") parsed.total = value;
  }
  return parsed.input != null ||
    parsed.output != null ||
    parsed.cache != null ||
    parsed.reasoning != null ||
    parsed.total != null
    ? parsed
    : null;
}

export function isUsageItem(item: GroupedSessionItem): boolean {
  return item.text.startsWith("[用量]");
}

const PATCH_PLACEHOLDER = "应用多文件补丁";

export function changedFilesFromItems(items: GroupedSessionItem[]): string[] {
  const seen = new Set<string>();
  const paths: string[] = [];
  const add = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed || trimmed === PATCH_PLACEHOLDER) return;
    if (seen.has(trimmed)) return;
    seen.add(trimmed);
    paths.push(trimmed);
  };
  for (const item of items) {
    if (item.text.startsWith("[写入]") || item.text.startsWith("[编辑]")) {
      add(filePathText(item));
      continue;
    }
    if (!item.text.startsWith("[补丁]")) continue;
    const fromLine = filePathText(item);
    if (fromLine !== PATCH_PLACEHOLDER) add(fromLine);
    for (const row of (item.result ?? "").split("\n")) {
      const match = row.trim().match(/^(?:wrote|deleted)\s+(.+)$/);
      if (match?.[1]) add(match[1]);
    }
  }
  return paths;
}

export function fileActionKey(text: string): "fileWrite" | "fileEdit" | "filePatch" {
  if (text.startsWith("[编辑]")) return "fileEdit";
  if (text.startsWith("[补丁]")) return "filePatch";
  return "fileWrite";
}

export function isThinkingItem(item: GroupedSessionItem): boolean {
  return item.text.startsWith("[思考]");
}

const THINKING_DURATION_RE = /^\[思考\]\s*(\d+)秒(?:\s|$)/;

export function parseThinkingDurationSeconds(text: string): number | null {
  const match = text.trim().match(THINKING_DURATION_RE);
  if (!match) return null;
  const seconds = Number(match[1]);
  return Number.isFinite(seconds) ? seconds : null;
}

export function thinkingDurationSeconds(items: GroupedSessionItem[], nowMs?: number): number {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const parsed = parseThinkingDurationSeconds(items[index]?.text ?? "");
    if (parsed != null) return parsed;
  }
  return segmentDurationSeconds(items, nowMs);
}

export function thinkingText(items: GroupedSessionItem[]): string {
  const live = items.find((item) => !item.text.startsWith("[思考]"));
  if (live) return live.text;
  return items
    .map((item) => item.text.replace(/^\[思考\](?:\s*\d+秒)?\s*/, "").trim())
    .filter((text) => text.length > 0)
    .join("\n\n");
}

export function isCommandTool(item: GroupedSessionItem): boolean {
  return item.kind === "tool" && item.text.startsWith("[命令]");
}

export function isLookupTool(item: GroupedSessionItem): boolean {
  if (item.kind !== "tool") return false;
  return (
    item.text.startsWith("[读取]") ||
    item.text.startsWith("[工具] Glob") ||
    item.text.startsWith("[工具] Grep")
  );
}

export function isFileChangeTool(item: GroupedSessionItem): boolean {
  if (item.kind !== "tool") return false;
  return (
    item.text.startsWith("[写入]") ||
    item.text.startsWith("[编辑]") ||
    item.text.startsWith("[补丁]")
  );
}

export function summarizeTools(items: GroupedSessionItem[]): ToolSummary {
  const summary: ToolSummary = { files: 0, lists: 0, searches: 0 };
  for (const item of items) {
    if (item.text.startsWith("[读取]")) summary.files += 1;
    else if (item.text.startsWith("[工具] Glob")) summary.lists += 1;
    else if (item.text.startsWith("[工具] Grep")) summary.searches += 1;
  }
  return summary;
}

export function parseTodoList(text: string): ParsedTodoList | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("[待办]")) return null;
  const body = trimmed.slice("[待办]".length).trim();
  if (!body || body === "读取任务清单" || body === "(空)" || body === "更新任务清单") {
    return null;
  }
  const items: ParsedTodoItem[] = [];
  for (const line of body.split("\n")) {
    const match = line.trim().match(TODO_LINE_RE);
    if (!match) continue;
    const status = match[1];
    const content = match[2]?.trim() ?? "";
    const priority = match[3]?.trim() ?? "medium";
    if (!content || !TODO_STATUSES.includes(status as TodoStatus)) continue;
    items.push({ status: status as TodoStatus, content, priority });
  }
  if (items.length === 0) return null;
  const current =
    items.find((item) => item.status === "in_progress") ?? items[items.length - 1] ?? null;
  return {
    items,
    current,
    completed: items.filter((item) => item.status === "completed").length,
    total: items.length,
  };
}

export function latestTodos(items: { text: string }[]): ParsedTodoList | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const parsed = parseTodoList(items[index]?.text ?? "");
    if (parsed) return parsed;
  }
  return null;
}

export function displaySessionTitle(title: string | null | undefined): string {
  const trimmed = title?.trim() ?? "";
  if (!trimmed) return "";
  return Array.from(trimmed).slice(0, 30).join("");
}

export function isHiddenSessionCeremonyLine(text: string): boolean {
  const line = text.trim();
  if (line.startsWith("[续聊]")) return true;
  return line === "内置 Agent 会话已恢复" || line === "内置 Agent 会话已创建";
}

export function groupSessionLines(lines: RawSessionLine[]): GroupedSessionItem[] {
  const grouped: GroupedSessionItem[] = [];
  for (const line of lines) {
    if (isHiddenSessionCeremonyLine(line.text)) continue;
    const kind = classifyLine(line.text);
    if (kind === "tool_result") {
      const result = line.text.replace(/^\[工具结果\]\s*/, "");
      let paired = false;
      for (let index = grouped.length - 1; index >= 0; index -= 1) {
        const item = grouped[index];
        if (item?.kind === "tool" && !item.result) {
          item.result = result;
          paired = true;
          break;
        }
      }
      if (paired) continue;
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

function segmentKey(item: GroupedSessionItem): TurnSegmentKind | "skip" | "file_change" {
  if (item.kind === "user") return "skip";
  if (isThinkingItem(item)) return "thinking";
  if (isUsageItem(item)) return "usage";
  if (isCommandTool(item)) return "terminal";
  if (parseTodoList(item.text)) return "todo";
  if (isFileChangeTool(item)) return "file_change";
  if (item.kind === "tool") return "tools";
  if (item.kind === "assistant") return "assistant";
  return "system";
}

function flushFileChanges(items: GroupedSessionItem[]): TurnSegment {
  return {
    kind: items.length > 1 ? "changes" : "file",
    items,
  };
}

export function buildTurnSegments(items: GroupedSessionItem[]): TurnSegment[] {
  const segments: TurnSegment[] = [];
  let currentKey: TurnSegmentKind | "file_change" | null = null;
  let currentItems: GroupedSessionItem[] = [];

  const flush = () => {
    if (!currentKey || currentItems.length === 0) {
      currentKey = null;
      currentItems = [];
      return;
    }
    if (currentKey === "file_change") {
      segments.push(flushFileChanges(currentItems));
    } else if (currentKey === "terminal" || currentKey === "todo" || currentKey === "usage") {
      for (const item of currentItems) {
        segments.push({ kind: currentKey, items: [item] });
      }
    } else {
      segments.push({ kind: currentKey, items: currentItems });
    }
    currentKey = null;
    currentItems = [];
  };

  for (const item of items) {
    const key = segmentKey(item);
    if (key === "skip") continue;
    const mergeable = key !== "terminal" && key !== "todo";
    if (currentKey !== key || !mergeable) {
      flush();
      currentKey = key;
    }
    currentItems.push(item);
  }
  flush();
  return segments;
}

export function isSessionStartLine(text: string): boolean {
  const line = text.trim();
  if (line === "内置 Agent 会话已恢复" || line === "内置 Agent 会话已创建") {
    return true;
  }
  if (line.startsWith("[续聊]")) return true;
  if (line.startsWith("[PLAN] 已进入计划模式")) return true;
  if (parseAgentBanner(line)) return true;
  if (
    line.startsWith("[PERMISSION] 已在设置中关闭高风险确认") ||
    line.startsWith("[PERMISSION] 已开启自动编辑")
  ) {
    return true;
  }
  return line.startsWith("[MCP] 未启用服务器") || line.startsWith("[MCP] 将连接 ");
}

export function buildTurnBlocks(items: GroupedSessionItem[]): SessionTurnBlock[] {
  const blocks: SessionTurnBlock[] = [];
  let current: SessionTurnBlock | null = null;
  let currentItems: GroupedSessionItem[] = [];

  const finish = (block: SessionTurnBlock, collected: GroupedSessionItem[]) => {
    block.segments = buildTurnSegments(collected);
  };

  const startBlock = (item: GroupedSessionItem): SessionTurnBlock => {
    const block: SessionTurnBlock = {
      id: item.id,
      tools: [],
      assistant: [],
      system: [],
      segments: [],
      startedAt: item.createdAt,
      endedAt: item.createdAt,
    };
    blocks.push(block);
    return block;
  };

  const append = (block: SessionTurnBlock, item: GroupedSessionItem) => {
    block.endedAt = item.createdAt;
    currentItems.push(item);
    if (item.kind === "user") block.user = item;
    else if (item.kind === "tool") block.tools.push(item);
    else if (item.kind === "assistant") block.assistant.push(item);
    else block.system.push(item);
  };

  for (const item of items) {
    const startUserTurn = item.kind === "user" && Boolean(current?.user);
    const startResumeTurn = Boolean(current?.user && isSessionStartLine(item.text));
    if (startUserTurn || startResumeTurn || !current) {
      if (current) finish(current, currentItems);
      current = startBlock(item);
      currentItems = [];
    }
    if (item.kind === "user" && currentItems.length > 0) {
      current.user = item;
      current.id = item.id;
      current.startedAt = item.createdAt;
      currentItems = [item, ...currentItems];
      continue;
    }
    append(current, item);
  }
  if (current) finish(current, currentItems);
  return blocks;
}

export type DurationTranslate = (key: string, options?: Record<string, number>) => string;

export function splitSessionDuration(totalSeconds: number): {
  hours: number;
  minutes: number;
  seconds: number;
} {
  const safe = Math.max(0, Math.round(totalSeconds));
  return {
    hours: Math.floor(safe / 3600),
    minutes: Math.floor((safe % 3600) / 60),
    seconds: safe % 60,
  };
}

export function formatSessionDuration(t: DurationTranslate, totalSeconds: number): string {
  const { hours, minutes, seconds } = splitSessionDuration(totalSeconds);
  if (hours === 0 && minutes === 0) return t("durationSeconds", { seconds });
  if (hours === 0) {
    return seconds === 0
      ? t("durationMinutesOnly", { minutes })
      : t("durationMinutesSeconds", { minutes, seconds });
  }
  if (minutes === 0 && seconds === 0) return t("durationHoursOnly", { hours });
  if (seconds === 0) return t("durationHoursMinutes", { hours, minutes });
  return t("durationHoursMinutesSeconds", { hours, minutes, seconds });
}

export function workDurationSeconds(block: SessionTurnBlock, nowMs?: number): number {
  const start = Date.parse(block.startedAt);
  const end = nowMs ?? Date.parse(block.endedAt);
  if (Number.isNaN(start) || Number.isNaN(end)) return 0;
  return Math.max(0, Math.round((end - start) / 1000));
}

export function segmentDurationSeconds(items: GroupedSessionItem[], nowMs?: number): number {
  if (items.length === 0) return 0;
  const start = Date.parse(items[0]!.createdAt);
  const end = nowMs ?? Date.parse(items[items.length - 1]!.createdAt);
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
