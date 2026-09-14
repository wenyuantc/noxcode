import type { AiChannel, CreateNativeSubagentInput, NativeSubagent, Workspace } from "@/lib/types";

export const SUBAGENT_JSON_KIND = "noxcode.native-subagent";
export const SUBAGENT_JSON_VERSION = 1;

export type SubagentImportWarningCode =
  "channelMissing" | "workspacesMissing" | "workspacesPartial";

export interface SubagentImportWarning {
  code: SubagentImportWarningCode;
  /** 给 UI 插值，例如缺失的 channel_id / 丢弃的工作区数量 */
  channelId?: string;
  model?: string;
  dropped?: number;
}

export interface SubagentImportDraft {
  name: string;
  description: string;
  model_mode: "inherit" | "channel";
  channel_id: string | null;
  model: string | null;
  reasoning_effort: string | null;
  tool_mode: "all" | "custom";
  tools: string[];
  system_prompt: string;
  inject_agents_md: boolean;
  scope: "all" | "workspaces";
  workspace_ids: string[];
  permission_mode: string | null;
  disallowed_tools: string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function nullableString(value: unknown): string | null {
  const text = asString(value).trim();
  return text.length > 0 ? text : null;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item): item is string => typeof item === "string")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

export function serializeSubagentJson(item: NativeSubagent): string {
  const isChannel = item.model_mode === "channel";
  const isCustom = item.tool_mode === "custom";
  const isWorkspaces = item.scope === "workspaces";

  const payload = {
    kind: SUBAGENT_JSON_KIND,
    version: SUBAGENT_JSON_VERSION,
    name: item.name,
    description: item.description,
    model_mode: isChannel ? "channel" : "inherit",
    channel_id: isChannel ? nullableString(item.channel_id) : null,
    model: isChannel ? nullableString(item.model) : null,
    reasoning_effort: isChannel ? nullableString(item.reasoning_effort) : null,
    tool_mode: isCustom ? "custom" : "all",
    tools: isCustom ? (item.tools ?? []) : [],
    system_prompt: item.system_prompt,
    inject_agents_md: item.inject_agents_md !== false,
    scope: isWorkspaces ? "workspaces" : "all",
    workspace_ids: isWorkspaces ? (item.workspace_ids ?? []) : [],
    permission_mode: nullableString(item.permission_mode),
    disallowed_tools: item.disallowed_tools ?? [],
  };

  return JSON.stringify(payload, null, 2);
}

export function serializeSubagentDraftsJson(drafts: SubagentImportDraft[]): string {
  const items = drafts.map((draft) => ({
    kind: SUBAGENT_JSON_KIND,
    version: SUBAGENT_JSON_VERSION,
    name: draft.name,
    description: draft.description,
    model_mode: draft.model_mode,
    channel_id: draft.model_mode === "channel" ? draft.channel_id : null,
    model: draft.model_mode === "channel" ? draft.model : null,
    reasoning_effort: draft.model_mode === "channel" ? draft.reasoning_effort : null,
    tool_mode: draft.tool_mode,
    tools: draft.tool_mode === "custom" ? draft.tools : [],
    system_prompt: draft.system_prompt,
    inject_agents_md: draft.inject_agents_md,
    scope: draft.scope,
    workspace_ids: draft.scope === "workspaces" ? draft.workspace_ids : [],
    permission_mode: draft.permission_mode,
    disallowed_tools: draft.disallowed_tools,
  }));
  return JSON.stringify(items.length === 1 ? items[0] : items, null, 2);
}

function draftFromRecord(record: Record<string, unknown>): SubagentImportDraft {
  if (record.kind !== undefined && record.kind !== SUBAGENT_JSON_KIND) {
    throw new Error("不是 noxcode 子智能体 JSON");
  }
  if (record.version !== undefined && record.version !== SUBAGENT_JSON_VERSION) {
    throw new Error("不支持的子智能体 JSON 版本");
  }

  const name = asString(record.name).trim();
  const description = asString(record.description).trim();
  if (name.length === 0 || description.length === 0) {
    throw new Error("子智能体缺少名称或描述");
  }

  return {
    name,
    description,
    model_mode: record.model_mode === "channel" ? "channel" : "inherit",
    channel_id: nullableString(record.channel_id),
    model: nullableString(record.model),
    reasoning_effort: nullableString(record.reasoning_effort),
    tool_mode: record.tool_mode === "custom" ? "custom" : "all",
    tools: asStringArray(record.tools),
    system_prompt: asString(record.system_prompt),
    inject_agents_md: record.inject_agents_md !== false,
    scope: record.scope === "workspaces" ? "workspaces" : "all",
    workspace_ids: asStringArray(record.workspace_ids),
    permission_mode: nullableString(record.permission_mode),
    disallowed_tools: asStringArray(record.disallowed_tools),
  };
}

export function parseSubagentImportJson(text: string): SubagentImportDraft[] {
  const trimmed = text.trim();
  if (trimmed.length === 0) {
    throw new Error("请粘贴 JSON");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    throw new Error("JSON 格式无效");
  }

  let items: unknown[];
  if (Array.isArray(parsed)) {
    items = parsed;
  } else if (isRecord(parsed) && Array.isArray(parsed.subagents)) {
    if (parsed.kind !== undefined && parsed.kind !== SUBAGENT_JSON_KIND) {
      throw new Error("不是 noxcode 子智能体 JSON");
    }
    if (parsed.version !== undefined && parsed.version !== SUBAGENT_JSON_VERSION) {
      throw new Error("不支持的子智能体 JSON 版本");
    }
    if (parsed.subagents.length === 0) {
      throw new Error("没有可导入的子智能体");
    }
    items = parsed.subagents;
  } else if (isRecord(parsed)) {
    items = [parsed];
  } else {
    throw new Error("JSON 不是子智能体对象");
  }

  if (items.length === 0) {
    throw new Error("没有可导入的子智能体");
  }

  return items.map((item) => {
    if (!isRecord(item)) {
      throw new Error("JSON 不是子智能体对象");
    }
    return draftFromRecord(item);
  });
}

export function toImportedSubagentPayload(
  draft: SubagentImportDraft,
  ctx: { channels: AiChannel[]; workspaces: Workspace[] },
): { payload: CreateNativeSubagentInput; warnings: SubagentImportWarning[] } {
  const warnings: SubagentImportWarning[] = [];

  let modelMode: "inherit" | "channel" = draft.model_mode;
  let channelId = draft.channel_id;
  let model = draft.model;
  let reasoningEffort = draft.reasoning_effort;

  if (modelMode === "channel") {
    const matched = ctx.channels.some(
      (channel) =>
        channel.enabled === true &&
        channel.id === channelId &&
        channel.models.some((entry) => entry.id === model),
    );
    if (!matched) {
      warnings.push({
        code: "channelMissing",
        channelId: channelId ?? undefined,
        model: model ?? undefined,
      });
      modelMode = "inherit";
      channelId = null;
      model = null;
      reasoningEffort = null;
    }
  }

  let scope: "all" | "workspaces" = draft.scope;
  let workspaceIds = draft.workspace_ids;
  if (scope === "workspaces") {
    const liveIds = new Set(ctx.workspaces.map((workspace) => workspace.id));
    const kept = workspaceIds.filter((id) => liveIds.has(id));
    if (kept.length === 0) {
      warnings.push({ code: "workspacesMissing" });
      scope = "all";
      workspaceIds = [];
    } else {
      if (kept.length < workspaceIds.length) {
        warnings.push({ code: "workspacesPartial", dropped: workspaceIds.length - kept.length });
      }
      workspaceIds = kept;
    }
  }

  const payload: CreateNativeSubagentInput = {
    name: draft.name,
    description: draft.description,
    model_mode: modelMode,
    channel_id: modelMode === "channel" ? channelId : null,
    model: modelMode === "channel" ? model : null,
    reasoning_effort: modelMode === "channel" ? reasoningEffort?.trim() || null : null,
    tool_mode: draft.tool_mode,
    tools: draft.tool_mode === "custom" ? draft.tools : [],
    system_prompt: draft.system_prompt,
    inject_agents_md: draft.inject_agents_md,
    scope,
    workspace_ids: scope === "workspaces" ? workspaceIds : [],
    permission_mode: draft.permission_mode?.trim() || null,
    disallowed_tools: draft.disallowed_tools ?? [],
  };

  return { payload, warnings };
}
