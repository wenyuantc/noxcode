import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  channelLabel,
  enabledAiChannels,
  selectOverrideChannel,
  selectOverrideModel,
} from "@/lib/aiSettings";
import { composerThinkingEnabled, composerThinkingLevels } from "@/lib/modelCatalog";
import type { AiChannel, AiFeatureOverride } from "@/lib/types";

export function AiModelOverrideFields({
  value,
  channels,
  modelLabel,
  effortLabel,
  onChange,
  disabled = false,
}: {
  value: AiFeatureOverride;
  channels: AiChannel[];
  modelLabel: string;
  effortLabel: string;
  onChange: (next: AiFeatureOverride) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation(["settings", "sessions"]);
  const enabled = enabledAiChannels(channels);
  const selectedChannel = enabled.find((channel) => channel.id === value.channel_id) ?? null;
  const selectedModel = selectedChannel?.models.find((model) => model.id === value.model) ?? null;
  const effortLevels = composerThinkingLevels(selectedModel);
  const thinkingOn = composerThinkingEnabled(selectedModel);

  if (enabled.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        {t("settings:ai.needChannel")}{" "}
        <Link to="/settings/channels" className="text-primary underline-offset-2 hover:underline">
          {t("settings:sections.channels")}
        </Link>
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-foreground">{t("settings:ai.channel")}</label>
        <Select
          value={value.channel_id ?? undefined}
          disabled={disabled}
          onValueChange={(next) => {
            if (typeof next === "string") {
              onChange(selectOverrideChannel(value, enabled, next));
            }
          }}
        >
          <SelectTrigger className="w-full bg-background">
            <SelectValue>
              {(selected) => {
                if (typeof selected !== "string") return t("settings:ai.channelPlaceholder");
                const channel = enabled.find((item) => item.id === selected);
                return channel ? channelLabel(channel) : selected;
              }}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            {enabled.map((channel) => (
              <SelectItem key={channel.id} value={channel.id}>
                {channelLabel(channel)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-foreground">{modelLabel}</label>
          <Select
            value={value.model ?? undefined}
            disabled={disabled || !selectedChannel}
            onValueChange={(next) => {
              if (typeof next === "string") {
                onChange(selectOverrideModel(value, enabled, next));
              }
            }}
          >
            <SelectTrigger className="w-full bg-background">
              <SelectValue>
                {(selected) =>
                  typeof selected === "string" ? selected : t("settings:ai.modelPlaceholder")
                }
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {(selectedChannel?.models ?? []).map((model) => (
                <SelectItem key={model.id} value={model.id}>
                  {model.id}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <label className="text-xs font-medium text-foreground">{effortLabel}</label>
          <Select
            value={thinkingOn ? (value.reasoning_effort ?? undefined) : undefined}
            disabled={disabled || !thinkingOn}
            onValueChange={(next) => {
              if (typeof next === "string") {
                onChange({ ...value, reasoning_effort: next });
              }
            }}
          >
            <SelectTrigger className="w-full bg-background">
              <SelectValue>
                {(selected) =>
                  typeof selected === "string"
                    ? t(`sessions:effortLevels.${selected}.title`, { defaultValue: selected })
                    : t("settings:ai.effortPlaceholder")
                }
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {effortLevels.map((level) => (
                <SelectItem key={level} value={level}>
                  {t(`sessions:effortLevels.${level}.title`, { defaultValue: level })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {t("settings:ai.effortHint")}
          </p>
        </div>
      </div>
    </div>
  );
}
