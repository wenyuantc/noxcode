import { GitCommitHorizontal, MessageSquareText, Save } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { updateAiSettings } from "@/lib/backend";
import { EMPTY_AI_FEATURE_OVERRIDE, withEnabledOverride } from "@/lib/aiSettings";
import type { AiChannel, AiFeatureOverride, AiSettings } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { AiModelOverrideFields } from "./AiModelOverrideFields";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

const EMPTY_AI_SETTINGS: AiSettings = {
  commit_message: EMPTY_AI_FEATURE_OVERRIDE,
  session_title: EMPTY_AI_FEATURE_OVERRIDE,
};

export function AiFeaturesSection() {
  const { t } = useTranslation(["settings", "common"]);
  const stored = useSettingsStore((state) => state.ai);
  const setAi = useSettingsStore((state) => state.setAi);
  const channels = useChannelStore((state) => state.channels);
  const [draft, setDraft] = useState<AiSettings>(stored ?? EMPTY_AI_SETTINGS);
  const [saving, setSaving] = useState<"commit_message" | "session_title" | null>(null);
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    if (stored) setDraft(stored);
  }, [stored]);

  const patch = (key: keyof AiSettings, next: AiFeatureOverride) => {
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
        onToggle={(enabled) =>
          patch("commit_message", withEnabledOverride(draft.commit_message, channels, enabled))
        }
        onChange={(next) => patch("commit_message", next)}
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
