import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import type { AgentSession, AiChannelModel, NativeSessionRuntime } from "@/lib/types";

export interface SessionModelSelection {
  channelId: string | null;
  modelId: string | null;
}

export interface PlanApprovalModelArgs {
  aiChannelId?: string;
  model?: string;
  reasoningEffort?: string;
}

export interface PlanApprovalThinking {
  enabled: boolean;
  levels: string[];
  effort: string;
}

function nonEmpty(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

export function resolveSessionSelection(input: {
  sessionId?: string | null;
  runtime?: Pick<NativeSessionRuntime, "ai_channel_id" | "model"> | null;
  session?: Pick<AgentSession, "ai_channel_id" | "model"> | null;
  fallbackChannelId: string | null;
  fallbackModelId: string | null;
}): SessionModelSelection {
  if (!input.sessionId) {
    return {
      channelId: nonEmpty(input.fallbackChannelId),
      modelId: nonEmpty(input.fallbackModelId),
    };
  }
  return {
    channelId:
      nonEmpty(input.runtime?.ai_channel_id) ??
      nonEmpty(input.session?.ai_channel_id) ??
      nonEmpty(input.fallbackChannelId),
    modelId:
      nonEmpty(input.runtime?.model) ??
      nonEmpty(input.session?.model) ??
      nonEmpty(input.fallbackModelId),
  };
}

export function resolvePlanApprovalThinking(input: {
  channels: { id: string; models: AiChannelModel[] }[];
  selection: SessionModelSelection;
  preferredEffort?: string | null;
}): PlanApprovalThinking {
  const channel = input.channels.find((item) => item.id === input.selection.channelId);
  const model = channel?.models.find((item) => item.id === input.selection.modelId) ?? null;
  const levels = composerThinkingLevels(model);
  return {
    enabled: composerThinkingEnabled(model) && levels.length > 0,
    levels,
    effort: resolveComposerThinkingLevel(levels, input.preferredEffort, model?.thinking_level),
  };
}

export function planApprovalModelArgs(
  approved: boolean,
  selection: SessionModelSelection,
  reasoningEffort?: string | null,
): PlanApprovalModelArgs {
  if (!approved) return {};
  const aiChannelId = nonEmpty(selection.channelId) ?? undefined;
  const model = nonEmpty(selection.modelId) ?? undefined;
  if (!aiChannelId || !model) return {};
  const effort = nonEmpty(reasoningEffort) ?? undefined;
  return effort ? { aiChannelId, model, reasoningEffort: effort } : { aiChannelId, model };
}

export function mergeSessionRuntime(
  current: NativeSessionRuntime | undefined,
  changes: Partial<NativeSessionRuntime>,
  fallback: {
    channelId: string | null;
    modelId: string | null;
    permissionMode: string;
    planMode: boolean;
  },
): NativeSessionRuntime {
  if (current) return { ...current, ...changes };
  return {
    ai_channel_id: changes.ai_channel_id ?? fallback.channelId ?? "",
    model: changes.model ?? fallback.modelId ?? "",
    reasoning_effort: changes.reasoning_effort ?? null,
    permission_mode: changes.permission_mode ?? fallback.permissionMode,
    plan_mode: changes.plan_mode ?? fallback.planMode,
  };
}
