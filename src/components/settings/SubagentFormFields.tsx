import { useTranslation } from "react-i18next";

import { NATIVE_SUBAGENT_CUSTOM_TOOLS } from "@/lib/backend";
import type { SubagentFormState } from "@/lib/subagentForm";
import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import type { AiChannel, Workspace } from "@/lib/types";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

export function SubagentFormFields({
  form,
  enabledChannels,
  workspaces,
  busy,
  onPatch,
}: {
  form: SubagentFormState;
  enabledChannels: AiChannel[];
  workspaces: Workspace[];
  busy: boolean;
  onPatch: (updates: Partial<SubagentFormState>) => void;
}) {
  const { t } = useTranslation(["settings", "sessions"]);
  const selectedChannel = enabledChannels.find((channel) => channel.id === form.channelId) ?? null;
  const selectedModel = selectedChannel?.models.find((item) => item.id === form.model) ?? null;
  const thinkingOn = composerThinkingEnabled(selectedModel);
  const effortLevels = composerThinkingLevels(selectedModel);
  const toggleTool = (tool: string, checked: boolean) => {
    onPatch({
      tools: checked
        ? [...form.tools.filter((item) => item !== tool), tool]
        : form.tools.filter((item) => item !== tool),
    });
  };
  const toggleWorkspace = (workspaceId: string, checked: boolean) => {
    onPatch({
      workspaceIds: checked
        ? [...form.workspaceIds.filter((item) => item !== workspaceId), workspaceId]
        : form.workspaceIds.filter((item) => item !== workspaceId),
    });
  };
  // 切到某个渠道的模型后，把思考等级重置为该模型支持时的默认等级；不支持思考则清空。
  const resetReasoningEffortForModel = (
    channel: AiChannel | null | undefined,
    modelId: string,
  ): string => {
    const model = channel?.models.find((item) => item.id === modelId) ?? null;
    if (!composerThinkingEnabled(model)) return "";
    return resolveComposerThinkingLevel(composerThinkingLevels(model), null, model?.thinking_level);
  };

  return (
    <div className="space-y-3">
      <Input
        value={form.name}
        disabled={busy}
        onChange={(event) => onPatch({ name: event.target.value })}
        placeholder={t("settings:subagents.fields.name")}
      />
      <Textarea
        value={form.description}
        disabled={busy}
        onChange={(event) => onPatch({ description: event.target.value })}
        placeholder={t("settings:subagents.fields.description")}
        rows={3}
      />
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("settings:subagents.fields.scope")}
        </label>
        <Select
          value={form.scope}
          disabled={busy}
          onValueChange={(value) => {
            if (value === "all" || value === "workspaces") {
              onPatch({ scope: value });
            }
          }}
        >
          <SelectTrigger className="mt-1 bg-background">
            <SelectValue>
              {(value) =>
                value === "workspaces"
                  ? t("settings:subagents.fields.scopeWorkspaces")
                  : t("settings:subagents.fields.scopeAll")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("settings:subagents.fields.scopeAll")}</SelectItem>
            <SelectItem value="workspaces">
              {t("settings:subagents.fields.scopeWorkspaces")}
            </SelectItem>
          </SelectContent>
        </Select>
        <p className="mt-1 text-xs text-muted-foreground">
          {t("settings:subagents.fields.scopeHint")}
        </p>
      </div>
      {form.scope === "workspaces" ? (
        <div>
          <label className="text-xs font-medium text-muted-foreground">
            {t("settings:subagents.fields.scopePickWorkspaces")}
          </label>
          <div className="mt-1 max-h-48 space-y-2 overflow-y-auto rounded-md border border-border p-3">
            {workspaces.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {t("settings:subagents.fields.scopeWorkspacesEmpty")}
              </p>
            ) : (
              workspaces.map((workspace) => (
                <label key={workspace.id} className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    className="h-4 w-4 rounded border-input"
                    checked={form.workspaceIds.includes(workspace.id)}
                    disabled={busy}
                    onChange={(event) => toggleWorkspace(workspace.id, event.target.checked)}
                  />
                  {workspace.name}
                </label>
              ))
            )}
          </div>
        </div>
      ) : null}
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("settings:subagents.fields.modelMode")}
        </label>
        <Select
          value={form.modelMode}
          disabled={busy}
          onValueChange={(value) => {
            if (value === "inherit" || value === "channel") {
              // 切回继承默认时清空思考等级。
              onPatch({
                modelMode: value,
                reasoningEffort: value === "inherit" ? "" : form.reasoningEffort,
              });
            }
          }}
        >
          <SelectTrigger className="mt-1 bg-background">
            <SelectValue>
              {(value) =>
                value === "channel"
                  ? t("settings:subagents.fields.modelChannel")
                  : t("settings:subagents.fields.modelInherit")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="inherit">{t("settings:subagents.fields.modelInherit")}</SelectItem>
            <SelectItem value="channel">{t("settings:subagents.fields.modelChannel")}</SelectItem>
          </SelectContent>
        </Select>
        <p className="mt-1 text-xs text-muted-foreground">
          {t("settings:subagents.fields.modelHint")}
        </p>
      </div>
      {form.modelMode === "channel" ? (
        <>
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("settings:subagents.fields.channel")}
            </label>
            <Select
              value={form.channelId || undefined}
              disabled={busy}
              onValueChange={(value) => {
                if (typeof value === "string") {
                  const channel = enabledChannels.find((item) => item.id === value);
                  const nextModel = channel?.models[0]?.id ?? "";
                  onPatch({
                    channelId: value,
                    model: nextModel,
                    reasoningEffort: resetReasoningEffortForModel(channel, nextModel),
                  });
                }
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>
                  {(value) =>
                    typeof value === "string"
                      ? (enabledChannels.find((item) => item.id === value)?.name ?? value)
                      : t("settings:subagents.fields.channel")
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {enabledChannels.map((channel) => (
                  <SelectItem key={channel.id} value={channel.id}>
                    {channel.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("settings:subagents.fields.model")}
            </label>
            <Select
              value={form.model || undefined}
              disabled={busy}
              onValueChange={(value) => {
                if (typeof value === "string") {
                  onPatch({
                    model: value,
                    reasoningEffort: resetReasoningEffortForModel(selectedChannel, value),
                  });
                }
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>
                  {(value) =>
                    typeof value === "string" ? value : t("settings:subagents.fields.model")
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
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("settings:subagents.fields.effort")}
            </label>
            <Select
              value={thinkingOn ? form.reasoningEffort || undefined : undefined}
              disabled={busy || !thinkingOn}
              onValueChange={(value) => {
                if (typeof value === "string") {
                  onPatch({ reasoningEffort: value });
                }
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>
                  {(selected) =>
                    typeof selected === "string"
                      ? t(`sessions:effortLevels.${selected}.title`, { defaultValue: selected })
                      : t("settings:subagents.fields.effortPlaceholder")
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
            <p className="mt-1 text-xs text-muted-foreground">
              {t("settings:subagents.fields.effortHint")}
            </p>
          </div>
        </>
      ) : null}
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("settings:subagents.fields.toolMode")}
        </label>
        <Select
          value={form.toolMode}
          disabled={busy}
          onValueChange={(value) => {
            if (value === "all" || value === "custom") {
              onPatch({ toolMode: value });
            }
          }}
        >
          <SelectTrigger className="mt-1 bg-background">
            <SelectValue>
              {(value) =>
                value === "custom"
                  ? t("settings:subagents.fields.toolCustom")
                  : t("settings:subagents.fields.toolAll")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("settings:subagents.fields.toolAll")}</SelectItem>
            <SelectItem value="custom">{t("settings:subagents.fields.toolCustom")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {form.toolMode === "custom" ? (
        <div className="grid grid-cols-2 gap-2 rounded-md border border-border p-3 sm:grid-cols-3">
          {NATIVE_SUBAGENT_CUSTOM_TOOLS.map((tool) => (
            <label key={tool} className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="h-4 w-4 rounded border-input"
                checked={form.tools.includes(tool)}
                disabled={busy}
                onChange={(event) => toggleTool(tool, event.target.checked)}
              />
              {tool}
            </label>
          ))}
        </div>
      ) : null}
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("settings:subagents.fields.systemPrompt")}
        </label>
        <Textarea
          className="mt-1 min-h-40"
          value={form.systemPrompt}
          disabled={busy}
          onChange={(event) => onPatch({ systemPrompt: event.target.value })}
          placeholder={t("settings:subagents.fields.systemPromptPlaceholder")}
          rows={10}
        />
      </div>
      <label className="flex items-start gap-3 rounded-md border border-border px-3 py-2">
        <input
          type="checkbox"
          className="mt-0.5 h-4 w-4 rounded border-input"
          checked={form.injectAgentsMd}
          disabled={busy}
          onChange={(event) => onPatch({ injectAgentsMd: event.target.checked })}
        />
        <div className="space-y-1">
          <p className="text-sm font-medium">{t("settings:subagents.fields.injectAgentsMd")}</p>
          <p className="text-xs text-muted-foreground">
            {t("settings:subagents.fields.injectAgentsMdHint")}
          </p>
        </div>
      </label>
    </div>
  );
}
