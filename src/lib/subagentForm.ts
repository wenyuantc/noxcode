import type {
  CreateNativeSubagentInput,
  GeneratedNativeSubagent,
  NativeSubagent,
  UpdateNativeSubagentInput,
  Workspace,
} from "./types";

export type NativeSubagentModelMode = "inherit" | "channel";
export type NativeSubagentToolMode = "all" | "custom";
export type NativeSubagentScope = "all" | "workspaces";

export interface SubagentFormState {
  name: string;
  description: string;
  modelMode: NativeSubagentModelMode;
  channelId: string;
  model: string;
  toolMode: NativeSubagentToolMode;
  tools: string[];
  systemPrompt: string;
  injectAgentsMd: boolean;
  scope: NativeSubagentScope;
  workspaceIds: string[];
}

export const EMPTY_SUBAGENT_FORM: SubagentFormState = {
  name: "",
  description: "",
  modelMode: "inherit",
  channelId: "",
  model: "",
  toolMode: "all",
  tools: [],
  systemPrompt: "",
  injectAgentsMd: true,
  scope: "all",
  workspaceIds: [],
};

export function formFromGeneratedSubagent(draft: GeneratedNativeSubagent): SubagentFormState {
  return {
    name: draft.name,
    description: draft.description,
    modelMode: "inherit",
    channelId: "",
    model: "",
    toolMode: draft.tool_mode === "custom" ? "custom" : "all",
    tools: draft.tools,
    systemPrompt: draft.system_prompt,
    injectAgentsMd: draft.inject_agents_md !== false,
    scope: "all",
    workspaceIds: [],
  };
}

export function toSubagentForm(item: NativeSubagent, workspaces: Workspace[]): SubagentFormState {
  const liveIds = new Set(workspaces.map((workspace) => workspace.id));
  return {
    name: item.name,
    description: item.description,
    modelMode: item.model_mode === "channel" ? "channel" : "inherit",
    channelId: item.channel_id ?? "",
    model: item.model ?? "",
    toolMode: item.tool_mode === "custom" ? "custom" : "all",
    tools: item.tools,
    systemPrompt: item.system_prompt,
    injectAgentsMd: item.inject_agents_md !== false,
    scope: item.scope === "workspaces" ? "workspaces" : "all",
    workspaceIds: (item.workspace_ids ?? []).filter((id) => liveIds.has(id)),
  };
}

export function subagentPayloadFrom(
  state: SubagentFormState,
): CreateNativeSubagentInput & UpdateNativeSubagentInput {
  return {
    name: state.name,
    description: state.description,
    model_mode: state.modelMode,
    channel_id: state.modelMode === "channel" ? state.channelId || null : null,
    model: state.modelMode === "channel" ? state.model || null : null,
    tool_mode: state.toolMode,
    tools: state.toolMode === "custom" ? state.tools : [],
    system_prompt: state.systemPrompt,
    inject_agents_md: state.injectAgentsMd,
    scope: state.scope,
    workspace_ids: state.scope === "workspaces" ? state.workspaceIds : [],
  };
}

export function canSubmitSubagentForm(form: SubagentFormState): boolean {
  return (
    form.name.trim().length > 0 &&
    form.description.trim().length > 0 &&
    (form.scope !== "workspaces" || form.workspaceIds.length > 0) &&
    (form.modelMode !== "channel" ||
      (form.channelId.trim().length > 0 && form.model.trim().length > 0)) &&
    (form.toolMode !== "custom" || form.tools.length > 0)
  );
}
