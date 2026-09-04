import type { NativeToolEvent, NativeToolImage } from "@/lib/types";

export type { NativeToolEvent, NativeToolImage };

export type SessionLineKind = "user" | "assistant" | "tool" | "tool_result" | "system" | "error";

export type TurnSegmentKind =
  | "thinking"
  | "tools"
  | "terminal"
  | "file"
  | "changes"
  | "todo"
  | "subagent"
  | "assistant"
  | "system"
  | "usage"
  | "retry"
  | "compact"
  | "goal"
  | "plan";

export type CompactTrigger = "auto" | "manual" | "reactive" | "downshift";

export interface CompactBoundary {
  trigger: CompactTrigger | string;
  source: "microcompact" | "model" | "local" | "reset" | string;
  pre_tokens: number;
  post_tokens: number;
  pre_messages: number;
  post_messages: number;
  instructions?: string | null;
}

export const COMPACT_BOUNDARY_PREFIX = "[COMPACT_BOUNDARY]";

/** `[COMPACT_BOUNDARY] {json}` → 结构化压缩边界；不是该格式返回 null。 */
export function parseCompactBoundary(text: string): CompactBoundary | null {
  const line = text.trim();
  if (!line.startsWith(COMPACT_BOUNDARY_PREFIX)) return null;
  const json = line.slice(COMPACT_BOUNDARY_PREFIX.length).trim();
  try {
    const value: unknown = JSON.parse(json);
    if (!value || typeof value !== "object") return null;
    const record = value as Record<string, unknown>;
    const num = (key: string) => (typeof record[key] === "number" ? (record[key] as number) : 0);
    return {
      trigger: typeof record.trigger === "string" ? record.trigger : "auto",
      source: typeof record.source === "string" ? record.source : "local",
      pre_tokens: num("pre_tokens"),
      post_tokens: num("post_tokens"),
      pre_messages: num("pre_messages"),
      post_messages: num("post_messages"),
      instructions: typeof record.instructions === "string" ? record.instructions : null,
    };
  } catch {
    return null;
  }
}

export function isCompactBoundaryLine(text: string): boolean {
  return text.trimStart().startsWith(COMPACT_BOUNDARY_PREFIX);
}

export const GOAL_LINE_PREFIX = "[GOAL]";

export interface ParsedGoalLine {
  cleared: boolean;
  title: string;
  status: string;
  checklist: { item: string; done: boolean }[];
  note: string | null;
}

/** `[GOAL] {json}` → 当前目标；`{"cleared":true}` 表示已清除。 */
export function parseGoalLine(text: string): ParsedGoalLine | null {
  const line = text.trim();
  if (!line.startsWith(GOAL_LINE_PREFIX)) return null;
  try {
    const value: unknown = JSON.parse(line.slice(GOAL_LINE_PREFIX.length).trim());
    if (!value || typeof value !== "object") return null;
    const record = value as Record<string, unknown>;
    if (record.cleared === true) {
      return { cleared: true, title: "", status: "cleared", checklist: [], note: null };
    }
    const checklist = Array.isArray(record.checklist)
      ? record.checklist
          .map((entry) => {
            if (!entry || typeof entry !== "object") return null;
            const item = entry as Record<string, unknown>;
            return typeof item.item === "string"
              ? { item: item.item, done: item.done === true }
              : null;
          })
          .filter((entry): entry is { item: string; done: boolean } => entry != null)
      : [];
    return {
      cleared: false,
      title: typeof record.title === "string" ? record.title : "",
      status: typeof record.status === "string" ? record.status : "active",
      checklist,
      note: typeof record.note === "string" ? record.note : null,
    };
  } catch {
    return null;
  }
}

export function isGoalLine(text: string): boolean {
  return text.trimStart().startsWith(GOAL_LINE_PREFIX);
}

export type PlanLineStatus =
  "entered" | "waiting_approval" | "waiting_question" | "execute" | "other";

export interface ParsedPlanLine {
  kind: "document" | "status";
  status: PlanLineStatus | null;
  title: string | null;
  body: string;
  questionSummary: string | null;
}

const PLAN_PREFIX_RE = /^\[(?:PLAN|计划)\]/;

export function isPlanLine(text: string): boolean {
  return PLAN_PREFIX_RE.test(text.trimStart());
}

export function planTitleFromBody(body: string): string | null {
  const heading = body.match(/^#{1,6}\s+(.+)$/m);
  if (heading?.[1]?.trim()) return heading[1].trim();
  const first = body.split("\n").find((line) => line.trim().length > 0);
  return first?.trim() || null;
}

/** `[PLAN]` / `[计划]` → 计划文档或短状态；不是该格式返回 null。 */
export function parsePlanLine(text: string): ParsedPlanLine | null {
  const trimmed = text.trim();
  if (!isPlanLine(trimmed)) return null;
  const rest = trimmed.replace(PLAN_PREFIX_RE, "");
  const newline = rest.indexOf("\n");
  if (newline >= 0) {
    const body = rest.slice(newline + 1).trim();
    if (body.length > 0) {
      return {
        kind: "document",
        status: null,
        title: planTitleFromBody(body),
        body,
        questionSummary: null,
      };
    }
  }
  const statusText = rest.trim();
  let status: PlanLineStatus = "other";
  if (statusText.startsWith("已进入计划模式")) status = "entered";
  else if (statusText.startsWith("等待用户批准")) status = "waiting_approval";
  else if (statusText.startsWith("等待用户回答")) status = "waiting_question";
  else if (statusText.startsWith("开始执行")) status = "execute";
  const questionSummary =
    status === "waiting_question"
      ? statusText.replace(/^等待用户回答[：:]\s*/, "").trim() || null
      : null;
  return {
    kind: "status",
    status,
    title: null,
    body: statusText,
    questionSummary,
  };
}

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
  tool?: NativeToolEvent;
  images?: NativeToolImage[];
}

export interface GroupedSessionItem {
  id: string;
  kind: SessionLineKind;
  text: string;
  createdAt: string;
  toolName?: string;
  result?: string;
  ok?: boolean;
  tool?: NativeToolEvent;
  images?: NativeToolImage[];
  subagentTag?: string;
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
  "[MCP工具]",
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
const SUBAGENT_TAG_RE = /^(\[子 Agent \d+\([^)]+\) - [^\]]*\])\s*/;
const SUBAGENT_STATUS_RE = /^(启动（|结束 |后台任务 )/;
const RETRY_TAIL_RE = /(?:，|,)\s*(\d+)\s*秒后进行第\s*(\d+)\s*\/\s*(\d+)\s*次重试\s*$/;
const HTTP_STATUS_RE = /HTTP\s*(\d{3})/i;

export function stripSubagentPrefix(text: string): { prefix: string | null; body: string } {
  const line = text.trimStart();
  const match = line.match(SUBAGENT_TAG_RE);
  if (!match) return { prefix: null, body: line };
  return { prefix: match[1] ?? null, body: line.slice(match[0].length) };
}

export function sessionLineBody(text: string): string {
  return stripSubagentPrefix(text).body;
}

export interface ParsedSubagentTag {
  raw: string;
  index: number;
  kind: string;
  description: string;
}

const SUBAGENT_PARSE_RE = /^\[子 Agent (\d+)\(([^)]+)\) - ([^\]]*)\]/;

export function parseSubagentTag(tag: string | null | undefined): ParsedSubagentTag | null {
  if (!tag) return null;
  const trimmed = tag.trim();
  const match = trimmed.match(SUBAGENT_PARSE_RE);
  if (!match) return null;
  return {
    raw: match[0],
    index: Number(match[1]),
    kind: match[2] ?? "general",
    description: (match[3] ?? "").trim(),
  };
}

export interface ParsedSubagentResult {
  description: string;
  kind: string;
  success: boolean;
  report?: string;
  error?: string;
}

const SUBAGENT_RESULT_OK_RE =
  /^子 Agent[（(](.+?)\s*[/／]\s*(.+?)[）)]完成[。.]?\s*(?:\n\n([\s\S]*))?$/;
const SUBAGENT_RESULT_ERR_RE = /^子 Agent[（(](.+?)\s*[/／]\s*(.+?)[）)]失败[：:]\s*([\s\S]*)$/;

export function parseSubagentResult(text: string): ParsedSubagentResult | null {
  const trimmed = text.trim();
  const matchOk = trimmed.match(SUBAGENT_RESULT_OK_RE);
  if (matchOk) {
    return {
      description: (matchOk[1] ?? "").trim(),
      kind: (matchOk[2] ?? "").trim(),
      success: true,
      report: (matchOk[3] ?? "").trim(),
    };
  }
  const matchErr = trimmed.match(SUBAGENT_RESULT_ERR_RE);
  if (matchErr) {
    return {
      description: (matchErr[1] ?? "").trim(),
      kind: (matchErr[2] ?? "").trim(),
      success: false,
      error: (matchErr[3] ?? "").trim(),
    };
  }
  return null;
}

function parseEnvelopeImages(value: unknown): NativeToolImage[] | undefined {
  if (!Array.isArray(value) || value.length === 0) return undefined;
  const images: NativeToolImage[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const record = item as { name?: unknown; mime_type?: unknown; data_url?: unknown };
    if (typeof record.name !== "string" || typeof record.data_url !== "string") continue;
    images.push({
      name: record.name,
      mime_type: typeof record.mime_type === "string" ? record.mime_type : "image/png",
      data_url: record.data_url,
    });
  }
  return images.length > 0 ? images : undefined;
}

export function parseStdoutEnvelope(
  message: string | null | undefined,
): { line: string; tool?: NativeToolEvent; images?: NativeToolImage[] } | null {
  if (!message) return null;
  const trimmed = message.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const value: unknown = JSON.parse(trimmed);
    if (!value || typeof value !== "object") return null;
    const record = value as {
      nox?: unknown;
      line?: unknown;
      tool?: NativeToolEvent;
      images?: unknown;
    };
    if (record.nox !== 1 || typeof record.line !== "string") return null;
    return { line: record.line, tool: record.tool, images: parseEnvelopeImages(record.images) };
  } catch {
    return null;
  }
}

export function hydrateSessionLine(input: RawSessionLine): RawSessionLine {
  const envelope = parseStdoutEnvelope(input.text);
  if (!envelope) return input;
  return {
    ...input,
    text: envelope.line,
    tool: input.tool ?? envelope.tool,
    images: input.images ?? envelope.images,
  };
}

export function isRetryLine(text: string): boolean {
  return stripSubagentPrefix(text).body.startsWith("[重试]");
}

export function isRetryFailureLine(text: string): boolean {
  const body = stripSubagentPrefix(text).body;
  return body.startsWith("[ERROR]") && body.includes("模型请求失败");
}

export interface ParsedRetryLine {
  agentPrefix?: string;
  failed: boolean;
  status?: number;
  delaySeconds?: number;
  attempt?: number;
  maxRetries?: number;
  title: string;
  message?: string;
  json?: string;
}

function extractJsonSnippet(text: string): string | null {
  const start = text.indexOf("{");
  if (start < 0) return null;
  const end = text.lastIndexOf("}");
  return end > start ? text.slice(start, end + 1) : text.slice(start);
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

function extractApiMessage(raw: string): string | undefined {
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object") return undefined;
    const record = value as { message?: unknown; error?: { message?: unknown } };
    const message = record.error?.message ?? record.message;
    return typeof message === "string" && message.trim() ? message : undefined;
  } catch {
    return undefined;
  }
}

export function parseRetryLine(text: string): ParsedRetryLine | null {
  const { prefix, body } = stripSubagentPrefix(text);
  const retry = body.startsWith("[重试]");
  const failed = isRetryFailureLine(text);
  if (!retry && !failed) return null;
  const payload = (retry ? body.slice("[重试]".length) : body.slice("[ERROR]".length)).trim();
  const tail = payload.match(RETRY_TAIL_RE);
  const withoutTail = (tail ? payload.slice(0, tail.index) : payload).trim();
  const jsonRaw = extractJsonSnippet(withoutTail);
  const statusMatch = withoutTail.match(HTTP_STATUS_RE);
  let title = jsonRaw ? withoutTail.replace(jsonRaw, "") : withoutTail;
  title = title
    .replace(/[（(]HTTP\s*\d{3}[）)]/gi, "")
    .replace(/\s*[:：]\s*$/, "")
    .trim();
  return {
    agentPrefix: prefix ?? undefined,
    failed,
    status: statusMatch ? Number(statusMatch[1]) : undefined,
    delaySeconds: tail ? Number(tail[1]) : undefined,
    attempt: tail ? Number(tail[2]) : undefined,
    maxRetries: tail ? Number(tail[3]) : undefined,
    title,
    message: jsonRaw ? extractApiMessage(jsonRaw) : undefined,
    json: jsonRaw ? prettyJson(jsonRaw) : undefined,
  };
}

export function summarizeRetry(items: GroupedSessionItem[]): {
  status?: number;
  attempt?: number;
  maxRetries?: number;
  count: number;
  failed: boolean;
} {
  const parsed = items
    .map((item) => parseRetryLine(item.text))
    .filter((item): item is ParsedRetryLine => item != null);
  const retries = parsed.filter((item) => !item.failed);
  const withAttempt = [...retries].reverse().find((item) => item.attempt != null);
  const withStatus = [...parsed].reverse().find((item) => item.status != null);
  return {
    status: withStatus?.status,
    attempt: withAttempt?.attempt ?? (retries.length > 0 ? retries.length : undefined),
    maxRetries: withAttempt?.maxRetries,
    count: retries.length,
    failed: parsed.some((item) => item.failed),
  };
}

export function classifyLine(text: string): SessionLineKind {
  const line = text.trimStart();
  const body = sessionLineBody(line);
  if (body.startsWith("[USER_INPUT]") || body.startsWith("[用户输入]")) return "user";
  if (isRetryLine(line)) return "system";
  if (body.startsWith("[ERROR]")) return "error";
  if (body.startsWith("[工具结果]")) return "tool_result";
  if (SUBAGENT_STATUS_RE.test(body)) return "system";
  if (body.startsWith("[子 Agent")) return "tool";
  if (TOOL_PREFIXES.some((prefix) => body.startsWith(prefix))) return "tool";
  if (SYSTEM_PREFIXES.some((prefix) => body.startsWith(prefix))) return "system";
  if (body.startsWith("[") && body.includes("]")) return "system";
  return "assistant";
}

export function toolTitle(text: string): string {
  const first = (sessionLineBody(text).split("\n")[0] ?? text).trim();
  const match = first.match(/^\[([^\]]+)\]\s*(.*)$/);
  if (!match) return first;
  const label = (match[1] ?? "").trim();
  const rest = (match[2] ?? "").trim();
  return rest ? `${label} ${rest}` : label;
}

export function stripUserPrefix(text: string): string {
  return text.replace(/^\[USER_INPUT\]\s*/, "").replace(/^\[用户输入\]\s*/, "");
}

export function commandText(item: GroupedSessionItem): string {
  return (
    sessionLineBody(item.text)
      .replace(/^\[命令\]\s*/, "")
      .split("\n")[0] ?? ""
  );
}

export function filePathText(item: GroupedSessionItem): string {
  return (
    sessionLineBody(item.text)
      .replace(/^\[(?:写入|编辑|补丁)\]\s*/, "")
      .split("\n")[0] ?? ""
  );
}

export function lookupPathText(item: GroupedSessionItem): string {
  return (
    sessionLineBody(item.text)
      .replace(/^\[(?:读取|工具)\]\s*/, "")
      .split("\n")[0] ?? ""
  );
}

export function parseReadResultLines(result: string): ReadResultLine[] {
  return result.split("\n").map((raw) => {
    const match = raw.match(/^\s*(\d+)\t(.*)$/);
    if (!match) return { line: null, text: raw };
    return { line: Number(match[1]), text: match[2] ?? "" };
  });
}

export function permissionHint(text: string): string | null {
  const line = sessionLineBody(text).trim();
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
  const line = sessionLineBody(text).trim();
  if (!line.startsWith("[MCP]") || line.startsWith("[MCP工具]")) return null;
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
  const line = sessionLineBody(text).trim();
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
  return sessionLineBody(item.text).startsWith("[用量]");
}

export function aggregateUsages(usages: (ParsedUsage | null | undefined)[]): ParsedUsage | null {
  let hasAny = false;
  let input = 0;
  let output = 0;
  let cache = 0;
  let reasoning = 0;
  let total = 0;
  let hasInput = false;
  let hasOutput = false;
  let hasCache = false;
  let hasReasoning = false;
  let hasTotal = false;

  for (const u of usages) {
    if (!u) continue;
    hasAny = true;
    if (u.input != null) {
      input += u.input;
      hasInput = true;
    }
    if (u.output != null) {
      output += u.output;
      hasOutput = true;
    }
    if (u.cache != null) {
      cache += u.cache;
      hasCache = true;
    }
    if (u.reasoning != null) {
      reasoning += u.reasoning;
      hasReasoning = true;
    }
    if (u.total != null) {
      total += u.total;
      hasTotal = true;
    }
  }

  if (!hasAny) return null;
  return {
    input: hasInput ? input : undefined,
    output: hasOutput ? output : undefined,
    cache: hasCache ? cache : undefined,
    reasoning: hasReasoning ? reasoning : undefined,
    total: hasTotal ? total : hasInput && hasOutput ? input + output : undefined,
  };
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
    const body = sessionLineBody(item.text);
    if (body.startsWith("[写入]") || body.startsWith("[编辑]")) {
      add(filePathText(item));
      continue;
    }
    if (!body.startsWith("[补丁]")) continue;
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
  const body = sessionLineBody(text);
  if (body.startsWith("[编辑]")) return "fileEdit";
  if (body.startsWith("[补丁]")) return "filePatch";
  return "fileWrite";
}

export function isThinkingItem(item: GroupedSessionItem): boolean {
  return sessionLineBody(item.text).startsWith("[思考]");
}

const THINKING_DURATION_RE = /^\[思考\]\s*(\d+)秒(?:\s|$)/;

export function parseThinkingDurationSeconds(text: string): number | null {
  const match = sessionLineBody(text).trim().match(THINKING_DURATION_RE);
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
  const live = items.find((item) => !sessionLineBody(item.text).startsWith("[思考]"));
  if (live) return live.text;
  return items
    .map((item) =>
      sessionLineBody(item.text)
        .replace(/^\[思考\](?:\s*\d+秒)?\s*/, "")
        .trim(),
    )
    .filter((text) => text.length > 0)
    .join("\n\n");
}

export function isCommandTool(item: GroupedSessionItem): boolean {
  return item.kind === "tool" && sessionLineBody(item.text).startsWith("[命令]");
}

export function isLookupTool(item: GroupedSessionItem): boolean {
  if (item.kind !== "tool") return false;
  const body = sessionLineBody(item.text);
  return (
    body.startsWith("[读取]") ||
    body.startsWith("[工具] Glob") ||
    body.startsWith("[工具] Grep") ||
    body.startsWith("[工具] WebFetch") ||
    body.startsWith("[工具] WebSearch")
  );
}

export function isFileChangeTool(item: GroupedSessionItem): boolean {
  if (item.kind !== "tool") return false;
  const body = sessionLineBody(item.text);
  return body.startsWith("[写入]") || body.startsWith("[编辑]") || body.startsWith("[补丁]");
}

export function summarizeTools(items: GroupedSessionItem[]): ToolSummary {
  const summary: ToolSummary = { files: 0, lists: 0, searches: 0 };
  for (const item of items) {
    const body = sessionLineBody(item.text);
    if (body.startsWith("[读取]") || body.startsWith("[工具] WebFetch")) summary.files += 1;
    else if (body.startsWith("[工具] Glob")) summary.lists += 1;
    else if (body.startsWith("[工具] Grep") || body.startsWith("[工具] WebSearch")) {
      summary.searches += 1;
    }
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
  if (
    line.startsWith("附带图片:") ||
    line.startsWith("附带图片：") ||
    line.startsWith("跳过缺失图片:") ||
    line.startsWith("跳过缺失图片：") ||
    line.startsWith("跳过图片:") ||
    line.startsWith("跳过图片：")
  ) {
    return true;
  }
  return (
    line === "内置 Agent 会话已恢复" ||
    line === "内置 Agent 会话已创建" ||
    line === "[ERROR] 已取消"
  );
}

function pairingBucket(item: {
  tool?: NativeToolEvent;
  text: string;
  subagentTag?: string;
}): string {
  return item.tool?.subagent_tag ?? item.subagentTag ?? stripSubagentPrefix(item.text).prefix ?? "";
}

function applyToolResult(item: GroupedSessionItem, result: string, line: RawSessionLine): void {
  item.result = result;
  if (line.tool) {
    item.ok = line.tool.ok ?? item.ok;
    item.tool = item.tool ? { ...item.tool, ...line.tool } : line.tool;
  }
  if (line.images?.length) item.images = line.images;
}

function pairToolResult(
  grouped: GroupedSessionItem[],
  line: RawSessionLine,
  result: string,
): boolean {
  const callId = line.tool?.call_id;
  if (callId) {
    const hit = grouped.find(
      (item) => item.kind === "tool" && !item.result && item.tool?.call_id === callId,
    );
    if (hit) {
      applyToolResult(hit, result, line);
      return true;
    }
  }
  const bucket = pairingBucket({ tool: line.tool, text: line.text });
  for (const item of grouped) {
    if (item.kind !== "tool" || item.result) continue;
    if (pairingBucket(item) !== bucket) continue;
    applyToolResult(item, result, line);
    return true;
  }
  return false;
}

export function groupSessionLines(lines: RawSessionLine[]): GroupedSessionItem[] {
  const grouped: GroupedSessionItem[] = [];
  for (const raw of lines) {
    const line = hydrateSessionLine(raw);
    if (isHiddenSessionCeremonyLine(line.text)) continue;
    const kind = classifyLine(line.text);
    const tag = stripSubagentPrefix(line.text).prefix ?? line.tool?.subagent_tag ?? undefined;
    if (kind === "tool_result") {
      const result = sessionLineBody(line.text).replace(/^\[工具结果\]\s*/, "");
      if (pairToolResult(grouped, line, result)) continue;
    }
    grouped.push({
      id: line.id,
      kind,
      text: kind === "user" ? stripUserPrefix(line.text) : line.text,
      createdAt: line.createdAt,
      toolName: kind === "tool" ? (line.tool?.title ?? toolTitle(line.text)) : undefined,
      ok: line.tool?.ok ?? undefined,
      tool: line.tool,
      images: line.images,
      subagentTag: tag ?? undefined,
    });
  }
  return grouped;
}

function segmentKey(item: GroupedSessionItem): TurnSegmentKind | "skip" | "file_change" {
  if (item.kind === "user") return "skip";
  if (item.subagentTag) return "subagent";
  const body = sessionLineBody(item.text);
  if (isCompactBoundaryLine(body)) return "compact";
  if (isGoalLine(body)) return "goal";
  if (isPlanLine(body)) return "plan";
  if (isRetryLine(item.text)) return "retry";
  if (isThinkingItem(item)) return "thinking";
  if (isUsageItem(item)) return "usage";
  if (isCommandTool(item)) return "terminal";
  if (parseTodoList(body)) return "todo";
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
    } else if (
      currentKey === "terminal" ||
      currentKey === "todo" ||
      currentKey === "usage" ||
      currentKey === "compact" ||
      currentKey === "goal" ||
      currentKey === "plan"
    ) {
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
    let key = segmentKey(item);
    if (currentKey === "retry" && isRetryFailureLine(item.text)) key = "retry";
    if (key === "skip") continue;
    const mergeable =
      key !== "terminal" && key !== "todo" && key !== "compact" && key !== "goal" && key !== "plan";
    const subagentMatch =
      currentKey === "subagent" && key === "subagent"
        ? currentItems[0]?.subagentTag === item.subagentTag
        : true;
    if (currentKey !== key || !mergeable || !subagentMatch) {
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
  if (line.startsWith("[PLAN] 已进入计划模式") || line.startsWith("[计划] 已进入计划模式")) {
    return true;
  }
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

export function lineToneClass(kind: SessionLineKind, text: string, ok?: boolean): string {
  const body = sessionLineBody(text);
  if (kind === "error" || body.startsWith("[ERROR]") || ok === false) {
    return "text-red-600 dark:text-red-400";
  }
  if (kind === "user") return "text-sky-700 dark:text-sky-300";
  if (kind === "tool" || kind === "tool_result") return "text-cyan-700 dark:text-cyan-400";
  if (body.startsWith("[思考]")) return "text-muted-foreground";
  if (body.startsWith("[PLAN]") || body.startsWith("[计划]") || body.startsWith("[待办]")) {
    return "text-violet-700 dark:text-violet-400";
  }
  if (text.startsWith("[子 Agent")) return "text-teal-700 dark:text-teal-400";
  return "text-foreground";
}
