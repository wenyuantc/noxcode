import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { updateNativeSettings } from "@/lib/backend";
import type { NativePermissionMode } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

const SUBAGENT_POLICIES = ["conservative", "balanced", "aggressive"] as const;
type SubagentPolicy = (typeof SUBAGENT_POLICIES)[number];

function isSubagentPolicy(value: unknown): value is SubagentPolicy {
  return value === "conservative" || value === "balanced" || value === "aggressive";
}

function TokenKInput({
  id,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <div className="relative max-w-xs">
      <Input
        id={id}
        className="pr-8"
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-sm text-muted-foreground">
        K
      </span>
    </div>
  );
}

export function NativeRuntimeSection() {
  const { t } = useTranslation(["settings", "common", "sessions"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [draft, setDraft] = useState(native);

  useEffect(() => {
    if (native) setDraft(native);
  }, [native]);

  if (!native || !draft) return null;

  const policyLabel = (value: unknown) => {
    if (value === "conservative") return t("settings:runtime.policyConservative");
    if (value === "aggressive") return t("settings:runtime.policyAggressive");
    return t("settings:runtime.policyBalanced");
  };

  const save = () => {
    void updateNativeSettings(draft).then(setNative);
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("settings:runtime.hint")}</p>
      <SettingCard
        title={t("settings:runtime.maxTurns")}
        description={t("settings:runtime.maxTurnsHint")}
      >
        <Input
          id="native-max-turns"
          className="max-w-xs"
          type="number"
          min={0}
          max={500}
          step={1}
          value={draft.max_turns}
          onChange={(event) => setDraft({ ...draft, max_turns: Number(event.target.value) })}
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.maxSubagentTurns")}
        description={t("settings:runtime.maxSubagentTurnsHint")}
      >
        <Input
          id="native-max-subagent-turns"
          className="max-w-xs"
          type="number"
          min={0}
          max={500}
          step={1}
          value={draft.max_subagent_turns}
          onChange={(event) =>
            setDraft({ ...draft, max_subagent_turns: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.permissionMode")}
        description={t("settings:runtime.permissionModeHint")}
      >
        <select
          className="h-8 max-w-xs rounded-md border bg-background px-2 text-sm"
          value={draft.permission_mode}
          onChange={(event) => {
            const permission_mode = event.target.value as NativePermissionMode;
            const next = { ...draft, permission_mode };
            setDraft(next);
            void updateNativeSettings({ permission_mode }).then(setNative);
          }}
        >
          <option value="confirm">{t("sessions:permission.confirm.title")}</option>
          <option value="auto_edit">{t("sessions:permission.auto_edit.title")}</option>
          <option value="full">{t("sessions:permission.full.title")}</option>
        </select>
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.permissionTimeout")}
        description={t("settings:runtime.permissionTimeoutHint")}
      >
        <Input
          id="native-permission-timeout"
          className="max-w-xs"
          type="number"
          min={0}
          max={86400}
          step={1}
          value={draft.permission_timeout_secs}
          onChange={(event) =>
            setDraft({ ...draft, permission_timeout_secs: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.contextWindow")}
        description={t("settings:runtime.contextWindowHint")}
      >
        <div className="space-y-3">
          <TokenKInput
            id="native-context-window-k"
            min={8}
            max={1000}
            value={draft.context_window_tokens / 1000}
            onChange={(value) => setDraft({ ...draft, context_window_tokens: value * 1000 })}
          />
          <label
            htmlFor="native-use-custom-context-window"
            className="flex max-w-xs cursor-pointer items-center justify-between gap-3 text-sm"
          >
            <span>{t("settings:runtime.useCustomContextWindow")}</span>
            <Switch
              id="native-use-custom-context-window"
              checked={draft.use_custom_context_window}
              onCheckedChange={(checked) => {
                const next = { ...draft, use_custom_context_window: checked };
                setDraft(next);
                void updateNativeSettings({ use_custom_context_window: checked }).then(setNative);
              }}
            />
          </label>
        </div>
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.rolloutBudget")}
        description={t("settings:runtime.rolloutBudgetHint")}
      >
        <TokenKInput
          id="native-rollout-token-budget-k"
          min={0}
          max={100000}
          value={draft.rollout_token_budget / 1000}
          onChange={(value) => setDraft({ ...draft, rollout_token_budget: value * 1000 })}
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.maxToolOutput")}
        description={t("settings:runtime.maxToolOutputHint")}
      >
        <TokenKInput
          id="native-max-tool-output-k"
          min={0.256}
          max={65.536}
          step={0.001}
          value={draft.max_tool_output_tokens / 1000}
          onChange={(value) => setDraft({ ...draft, max_tool_output_tokens: value * 1000 })}
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.maxConcurrent")}
        description={t("settings:runtime.maxConcurrentHint")}
      >
        <Input
          id="native-max-concurrent-subagents"
          className="max-w-xs"
          type="number"
          min={1}
          max={16}
          step={1}
          value={draft.max_concurrent_subagents}
          onChange={(event) =>
            setDraft({ ...draft, max_concurrent_subagents: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.policy")}
        description={t("settings:runtime.policyHint")}
      >
        <Select
          value={draft.subagent_policy}
          onValueChange={(value) => {
            if (!isSubagentPolicy(value)) return;
            const next = { ...draft, subagent_policy: value };
            setDraft(next);
            void updateNativeSettings({ subagent_policy: value }).then(setNative);
          }}
        >
          <SelectTrigger id="native-subagent-policy" className="max-w-xs bg-background">
            <SelectValue>{policyLabel}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {SUBAGENT_POLICIES.map((policy) => (
              <SelectItem key={policy} value={policy}>
                {policyLabel(policy)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.budgetShare")}
        description={t("settings:runtime.budgetShareHint")}
      >
        <Input
          id="native-subagent-budget-share"
          className="max-w-xs"
          type="number"
          min={5}
          max={100}
          step={1}
          value={draft.subagent_budget_share_percent}
          onChange={(event) =>
            setDraft({ ...draft, subagent_budget_share_percent: Number(event.target.value) })
          }
        />
      </SettingCard>
      <SettingCard
        title={t("settings:runtime.globalPrompt")}
        description={t("settings:runtime.globalPromptHint")}
      >
        <Textarea
          id="native-global-prompt"
          value={draft.global_prompt_template}
          onChange={(event) => setDraft({ ...draft, global_prompt_template: event.target.value })}
        />
      </SettingCard>
      <Button onClick={save}>{t("common:save")}</Button>
    </div>
  );
}
