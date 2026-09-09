import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import type {
  AiChannel,
  AiCommitMessageSettings,
  AiFeatureOverride,
  AiSettings,
  CommitMessageStyle,
} from "@/lib/types";

export const EMPTY_AI_FEATURE_OVERRIDE: AiFeatureOverride = {
  enabled: false,
  channel_id: null,
  model: null,
  reasoning_effort: null,
};

export const EMPTY_AI_PROMPT_ENHANCEMENT: AiFeatureOverride = {
  ...EMPTY_AI_FEATURE_OVERRIDE,
  enabled: true,
};

export const DEFAULT_COMMIT_MESSAGE_STYLE: CommitMessageStyle = "detailed";

export const EMPTY_AI_COMMIT_MESSAGE: AiCommitMessageSettings = {
  ...EMPTY_AI_FEATURE_OVERRIDE,
  style: DEFAULT_COMMIT_MESSAGE_STYLE,
};

export function normalizeCommitMessageStyle(style: string | null | undefined): CommitMessageStyle {
  return style === "concise" ? "concise" : "detailed";
}

export function withCommitMessageDefaults(
  value: AiFeatureOverride & { style?: string | null },
): AiCommitMessageSettings {
  return {
    enabled: value.enabled,
    channel_id: value.channel_id,
    model: value.model,
    reasoning_effort: value.reasoning_effort,
    style: normalizeCommitMessageStyle(value.style),
  };
}

export function normalizeAiSettings(settings: AiSettings): AiSettings {
  return {
    ...settings,
    commit_message: withCommitMessageDefaults(settings.commit_message),
    prompt_enhancement: settings.prompt_enhancement ?? EMPTY_AI_PROMPT_ENHANCEMENT,
  };
}

export function enabledAiChannels(channels: AiChannel[]): AiChannel[] {
  return channels.filter((channel) => channel.enabled);
}

export function channelLabel(channel: AiChannel): string {
  return `${channel.name} · ${channel.protocol}`;
}

export function withEnabledOverride<T extends AiFeatureOverride>(
  current: T,
  channels: AiChannel[],
  enabled: boolean,
): T {
  if (!enabled) {
    return { ...current, enabled: false };
  }
  return fillOverrideDefaults({ ...current, enabled: true }, channels);
}

export function fillOverrideDefaults<T extends AiFeatureOverride>(
  current: T,
  channels: AiChannel[],
): T {
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

export function selectOverrideChannel<T extends AiFeatureOverride>(
  current: T,
  channels: AiChannel[],
  channelId: string,
): T {
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

export function selectOverrideModel<T extends AiFeatureOverride>(
  current: T,
  channels: AiChannel[],
  modelId: string,
): T {
  return fillOverrideDefaults(
    {
      ...current,
      model: modelId,
      reasoning_effort: null,
    },
    channels,
  );
}
