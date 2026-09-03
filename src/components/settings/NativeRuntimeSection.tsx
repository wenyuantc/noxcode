import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { updateNativeSettings } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

export function NativeRuntimeSection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [draft, setDraft] = useState(native);

  useEffect(() => {
    if (native) setDraft(native);
  }, [native]);

  if (!native || !draft) return null;

  const save = () => {
    void updateNativeSettings(draft).then(setNative);
  };

  return (
    <div className="space-y-4">
      <SettingCard
        title={t("settings:runtime.maxTurns")}
        description={t("settings:runtime.maxTurnsHint")}
      >
        <Input
          type="number"
          value={draft.max_turns}
          onChange={(event) => setDraft({ ...draft, max_turns: Number(event.target.value) })}
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.maxSubagentTurns")}>
        <Input
          type="number"
          value={draft.max_subagent_turns}
          onChange={(event) =>
            setDraft({ ...draft, max_subagent_turns: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.confirmHighRisk")}>
        <input
          type="checkbox"
          checked={draft.confirm_high_risk}
          onChange={(event) => {
            const next = { ...draft, confirm_high_risk: event.target.checked };
            setDraft(next);
            void updateNativeSettings({ confirm_high_risk: next.confirm_high_risk }).then(
              setNative,
            );
          }}
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.permissionTimeout")}>
        <Input
          type="number"
          value={draft.permission_timeout_secs}
          onChange={(event) =>
            setDraft({ ...draft, permission_timeout_secs: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.contextWindow")}>
        <Input
          type="number"
          value={Math.round(draft.context_window_tokens / 1000)}
          onChange={(event) =>
            setDraft({ ...draft, context_window_tokens: Number(event.target.value) * 1000 })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.rolloutBudget")}>
        <Input
          type="number"
          value={Math.round(draft.rollout_token_budget / 1000)}
          onChange={(event) =>
            setDraft({ ...draft, rollout_token_budget: Number(event.target.value) * 1000 })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.maxToolOutput")}>
        <Input
          type="number"
          value={Math.round(draft.max_tool_output_tokens / 1000)}
          onChange={(event) =>
            setDraft({ ...draft, max_tool_output_tokens: Number(event.target.value) * 1000 })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.maxConcurrent")}>
        <Input
          type="number"
          value={draft.max_concurrent_subagents}
          onChange={(event) =>
            setDraft({ ...draft, max_concurrent_subagents: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.budgetShare")}>
        <Input
          type="number"
          value={draft.subagent_budget_share_percent}
          onChange={(event) =>
            setDraft({ ...draft, subagent_budget_share_percent: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard title={t("settings:runtime.globalPrompt")}>
        <Textarea
          value={draft.global_prompt_template}
          onChange={(event) => setDraft({ ...draft, global_prompt_template: event.target.value })}
        />
      </SettingCard>
      <Button onClick={save}>{t("common:save")}</Button>
    </div>
  );
}
