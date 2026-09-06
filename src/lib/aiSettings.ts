import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import type { AiChannel, AiFeatureOverride } from "@/lib/types";

export const EMPTY_AI_FEATURE_OVERRIDE: AiFeatureOverride = {
  enabled: false,
  channel_id: null,
  model: null,
  reasoning_effort: null,
};

export function enabledAiChannels(channels: AiChannel[]): AiChannel[] {
  return channels.filter((channel) => channel.enabled);
}

export function channelLabel(channel: AiChannel): string {
  return `${channel.name} · ${channel.protocol}`;
}

export function withEnabledOverride(
  current: AiFeatureOverride,
  channels: AiChannel[],
  enabled: boolean,
): AiFeatureOverride {
  if (!enabled) {
    return { ...current, enabled: false };
  }
  const next = fillOverrideDefaults({ ...current, enabled: true }, channels);
  return next;
}

export function fillOverrideDefaults(
  current: AiFeatureOverride,
  channels: AiChannel[],
): AiFeatureOverride {
  const enabled = enabledAiChannels(channels);
  const selected =
    enabled.find((channel) => channel.id === current.channel_id) ?? enabled[0] ?? null;
  if (!selected) {
    return {
      ...current,
      channel_id: null,
      model: null,
      reasoning_effort: null,
    };
  }
  const model =
    selected.models.find((item) => item.id === current.model) ?? selected.models[0] ?? null;
  const levels = composerThinkingLevels(model);
  const effort = composerThinkingEnabled(model)
    ? resolveComposerThinkingLevel(levels, current.reasoning_effort, model?.thinking_level)
    : null;
  return {
    ...current,
    channel_id: selected.id,
    model: model?.id ?? null,
    reasoning_effort: effort,
  };
}

export function selectOverrideChannel(
  current: AiFeatureOverride,
  channels: AiChannel[],
  channelId: string,
): AiFeatureOverride {
  return fillOverrideDefaults(
    {
      ...current,
      channel_id: channelId,
      model: null,
      reasoning_effort: null,
    },
    channels,
  );
}

export function selectOverrideModel(
  current: AiFeatureOverride,
  channels: AiChannel[],
  modelId: string,
): AiFeatureOverride {
  return fillOverrideDefaults(
    {
      ...current,
      model: modelId,
      reasoning_effort: null,
    },
    channels,
  );
}
