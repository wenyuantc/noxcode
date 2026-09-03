import { useTranslation } from "react-i18next";

import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";

export function QuickPromptChips() {
  const { t } = useTranslation("settings");
  const prompts = useSettingsStore((state) => state.quickPrompts);
  const setDraft = useUiStore((state) => state.setComposerDraft);

  if (prompts.length === 0) {
    return <p className="text-center text-xs text-muted-foreground">{t("general.quickPrompts")}</p>;
  }

  return (
    <div className="flex flex-wrap justify-center gap-2">
      {prompts.map((prompt) => (
        <button
          key={prompt.id}
          type="button"
          className="rounded-full border px-3 py-1 text-xs hover:bg-accent"
          onClick={() => setDraft(prompt.prompt)}
        >
          {prompt.label}
        </button>
      ))}
    </div>
  );
}
