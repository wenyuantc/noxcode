import { GitCommitHorizontal, MessageSquareText, Save, WandSparkles } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { updateAiSettings } from "@/lib/backend";
import {
  EMPTY_AI_COMMIT_MESSAGE,
  EMPTY_AI_FEATURE_OVERRIDE,
  EMPTY_AI_PROMPT_ENHANCEMENT,
  normalizeAiSettings,
  withEnabledOverride,
} from "@/lib/aiSettings";
import type { AiChannel, AiFeatureOverride, AiSettings, CommitMessageStyle } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { AiModelOverrideFields } from "./AiModelOverrideFields";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

const EMPTY_AI_SETTINGS: AiSettings = {
  commit_message: EMPTY_AI_COMMIT_MESSAGE,
  session_title: EMPTY_AI_FEATURE_OVERRIDE,
  prompt_enhancement: EMPTY_AI_PROMPT_ENHANCEMENT,
};

export function AiFeaturesSection() {
  const { t } = useTranslation(["settings", "common"]);
  const stored = useSettingsStore((state) => state.ai);
  const setAi = useSettingsStore((state) => state.setAi);
  const channels = useChannelStore((state) => state.channels);
  const [searchParams, setSearchParams] = useSearchParams();
  const [draft, setDraft] = useState<AiSettings>(stored ?? EMPTY_AI_SETTINGS);
  const [saving, setSaving] = useState<keyof AiSettings | null>(null);
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    if (stored) setDraft(normalizeAiSettings(stored));
  }, [stored]);

  useEffect(() => {
    if (searchParams.get("prompt-enhancement") !== "missing-model") return;
    setFeedback({ variant: "error", message: t("settings:ai.promptEnhancement.needModel") });
    setSearchParams({}, { replace: true });
  }, [searchParams, setSearchParams, t]);

  const patch = <K extends keyof AiSettings>(key: K, next: AiSettings[K]) => {
    setDraft((current) => ({ ...current, [key]: next }));
  };

  const save = async (key: keyof AiSettings) => {
    setSaving(key);
    setFeedback(null);
    try {
      const updated = await updateAiSettings(draft);
      setAi(updated);
      setDraft(updated);
      setFeedback({ variant: "success", message: t("common:saved") });
    } catch (reason) {
      setFeedback({ variant: "error", message: String(reason) });
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="space-y-6">
      {feedback ? (
        <SettingFeedbackCallout
          variant={feedback.variant}
          message={feedback.message}
          onClose={() => setFeedback(null)}
        />
      ) : null}

      <AiFeatureCard
        icon={WandSparkles}
        title={t("settings:ai.promptEnhancement.title")}
        description={t("settings:ai.promptEnhancement.description")}
        hint={t("settings:ai.promptEnhancement.hint")}
        modelLabel={t("settings:ai.promptEnhancement.model")}
        effortLabel={t("settings:ai.promptEnhancement.effort")}
        value={draft.prompt_enhancement}
        channels={channels}
        saving={saving === "prompt_enhancement"}
        disabled={saving !== null}
        onToggle={(enabled) =>
          patch(
            "prompt_enhancement",
            withEnabledOverride(draft.prompt_enhancement, channels, enabled),
          )
        }
        onChange={(next) => patch("prompt_enhancement", next)}
        onSave={() => void save("prompt_enhancement")}
      />

      <AiFeatureCard
        icon={GitCommitHorizontal}
        title={t("settings:ai.commitMessage.title")}
        description={t("settings:ai.commitMessage.description")}
        hint={t("settings:ai.commitMessage.hint")}
        modelLabel={t("settings:ai.commitMessage.model")}
        effortLabel={t("settings:ai.commitMessage.effort")}
        value={draft.commit_message}
        channels={channels}
        saving={saving === "commit_message"}
        disabled={saving !== null}
        extraFields={
          <CommitMessageStyleField
            value={draft.commit_message.style}
            disabled={saving !== null}
            onChange={(style) => patch("commit_message", { ...draft.commit_message, style })}
          />
        }
        onToggle={(enabled) =>
          patch("commit_message", withEnabledOverride(draft.commit_message, channels, enabled))
        }
        onChange={(next) => patch("commit_message", { ...draft.commit_message, ...next })}
        onSave={() => void save("commit_message")}
      />

      <AiFeatureCard
        icon={MessageSquareText}
        title={t("settings:ai.sessionTitle.title")}
        description={t("settings:ai.sessionTitle.description")}
        hint={t("settings:ai.sessionTitle.hint")}
        modelLabel={t("settings:ai.sessionTitle.model")}
        effortLabel={t("settings:ai.sessionTitle.effort")}
        value={draft.session_title}
        channels={channels}
        saving={saving === "session_title"}
        disabled={saving !== null}
        onToggle={(enabled) =>
          patch("session_title", withEnabledOverride(draft.session_title, channels, enabled))
        }
        onChange={(next) => patch("session_title", next)}
        onSave={() => void save("session_title")}
      />
    </div>
  );
}

function AiFeatureCard({
  icon,
  title,
  description,
  hint,
  modelLabel,
  effortLabel,
  value,
  channels,
  saving,
  disabled,
  extraFields,
  onToggle,
  onChange,
  onSave,
}: {
  icon: typeof GitCommitHorizontal;
  title: string;
  description: string;
  hint: string;
  modelLabel: string;
  effortLabel: string;
  value: AiFeatureOverride;
  channels: AiChannel[];
  saving: boolean;
  disabled: boolean;
  extraFields?: ReactNode;
  onToggle: (enabled: boolean) => void;
  onChange: (next: AiFeatureOverride) => void;
  onSave: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  return (
    <SettingCard
      icon={icon}
      title={title}
      description={description}
      headerAction={
        <Switch
          checked={value.enabled}
          disabled={disabled}
          onCheckedChange={onToggle}
          aria-label={title}
        />
      }
    >
      {value.enabled ? (
        <div className="space-y-4">
          <p className="text-xs text-muted-foreground">{hint}</p>
          {extraFields}
          <AiModelOverrideFields
            value={value}
            channels={channels}
            modelLabel={modelLabel}
            effortLabel={effortLabel}
            disabled={disabled}
            onChange={onChange}
          />
          <Button size="sm" className="h-7 gap-1.5 text-xs" disabled={disabled} onClick={onSave}>
            <Save className="size-3.5" />
            {saving ? t("common:loading", { defaultValue: "保存中…" }) : t("settings:ai.save")}
          </Button>
        </div>
      ) : (
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-muted-foreground">{t("settings:ai.disabledHint")}</p>
          <Button size="sm" className="h-7 gap-1.5 text-xs" disabled={disabled} onClick={onSave}>
            <Save className="size-3.5" />
            {saving ? t("common:loading", { defaultValue: "保存中…" }) : t("settings:ai.save")}
          </Button>
        </div>
      )}
    </SettingCard>
  );
}

function CommitMessageStyleField({
  value,
  disabled,
  onChange,
}: {
  value: CommitMessageStyle;
  disabled: boolean;
  onChange: (style: CommitMessageStyle) => void;
}) {
  const { t } = useTranslation("settings");
  return (
    <div className="space-y-1.5">
      <label className="text-xs font-medium text-foreground">
        {t("settings:ai.commitMessage.style")}
      </label>
      <Select
        value={value}
        disabled={disabled}
        onValueChange={(next) => {
          if (next === "concise" || next === "detailed") onChange(next);
        }}
      >
        <SelectTrigger className="w-full bg-background">
          <SelectValue>
            {(selected) =>
              selected === "concise"
                ? t("settings:ai.commitMessage.styleConcise")
                : t("settings:ai.commitMessage.styleDetailed")
            }
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="concise">{t("settings:ai.commitMessage.styleConcise")}</SelectItem>
          <SelectItem value="detailed">{t("settings:ai.commitMessage.styleDetailed")}</SelectItem>
        </SelectContent>
      </Select>
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        {t("settings:ai.commitMessage.styleHint")}
      </p>
    </div>
  );
}
