import { useTranslation } from "react-i18next";

import { NATIVE_SUBAGENT_CUSTOM_TOOLS } from "@/lib/backend";
import type { SubagentFormState } from "@/lib/subagentForm";
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
  const { t } = useTranslation("settings");
  const selectedChannel = enabledChannels.find((channel) => channel.id === form.channelId) ?? null;
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

  return (
    <div className="space-y-3">
      <Input
        value={form.name}
        disabled={busy}
        onChange={(event) => onPatch({ name: event.target.value })}
        placeholder={t("subagents.fields.name")}
      />
      <Textarea
        value={form.description}
        disabled={busy}
        onChange={(event) => onPatch({ description: event.target.value })}
        placeholder={t("subagents.fields.description")}
        rows={3}
      />
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("subagents.fields.scope")}
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
                  ? t("subagents.fields.scopeWorkspaces")
                  : t("subagents.fields.scopeAll")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("subagents.fields.scopeAll")}</SelectItem>
            <SelectItem value="workspaces">{t("subagents.fields.scopeWorkspaces")}</SelectItem>
          </SelectContent>
        </Select>
        <p className="mt-1 text-xs text-muted-foreground">{t("subagents.fields.scopeHint")}</p>
      </div>
      {form.scope === "workspaces" ? (
        <div>
          <label className="text-xs font-medium text-muted-foreground">
            {t("subagents.fields.scopePickWorkspaces")}
          </label>
          <div className="mt-1 max-h-48 space-y-2 overflow-y-auto rounded-md border border-border p-3">
            {workspaces.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {t("subagents.fields.scopeWorkspacesEmpty")}
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
          {t("subagents.fields.modelMode")}
        </label>
        <Select
          value={form.modelMode}
          disabled={busy}
          onValueChange={(value) => {
            if (value === "inherit" || value === "channel") {
              onPatch({ modelMode: value });
            }
          }}
        >
          <SelectTrigger className="mt-1 bg-background">
            <SelectValue>
              {(value) =>
                value === "channel"
                  ? t("subagents.fields.modelChannel")
                  : t("subagents.fields.modelInherit")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="inherit">{t("subagents.fields.modelInherit")}</SelectItem>
            <SelectItem value="channel">{t("subagents.fields.modelChannel")}</SelectItem>
          </SelectContent>
        </Select>
        <p className="mt-1 text-xs text-muted-foreground">{t("subagents.fields.modelHint")}</p>
      </div>
      {form.modelMode === "channel" ? (
        <>
          <div>
            <label className="text-xs font-medium text-muted-foreground">
              {t("subagents.fields.channel")}
            </label>
            <Select
              value={form.channelId || undefined}
              disabled={busy}
              onValueChange={(value) => {
                if (typeof value === "string") {
                  const channel = enabledChannels.find((item) => item.id === value);
                  onPatch({
                    channelId: value,
                    model: channel?.models[0]?.id ?? "",
                  });
                }
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>
                  {(value) =>
                    typeof value === "string"
                      ? (enabledChannels.find((item) => item.id === value)?.name ?? value)
                      : t("subagents.fields.channel")
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
              {t("subagents.fields.model")}
            </label>
            <Select
              value={form.model || undefined}
              disabled={busy}
              onValueChange={(value) => {
                if (typeof value === "string") {
                  onPatch({ model: value });
                }
              }}
            >
              <SelectTrigger className="mt-1 bg-background">
                <SelectValue>
                  {(value) => (typeof value === "string" ? value : t("subagents.fields.model"))}
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
        </>
      ) : null}
      <div>
        <label className="text-xs font-medium text-muted-foreground">
          {t("subagents.fields.toolMode")}
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
                  ? t("subagents.fields.toolCustom")
                  : t("subagents.fields.toolAll")
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("subagents.fields.toolAll")}</SelectItem>
            <SelectItem value="custom">{t("subagents.fields.toolCustom")}</SelectItem>
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
          {t("subagents.fields.systemPrompt")}
        </label>
        <Textarea
          className="mt-1 min-h-40"
          value={form.systemPrompt}
          disabled={busy}
          onChange={(event) => onPatch({ systemPrompt: event.target.value })}
          placeholder={t("subagents.fields.systemPromptPlaceholder")}
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
          <p className="text-sm font-medium">{t("subagents.fields.injectAgentsMd")}</p>
          <p className="text-xs text-muted-foreground">
            {t("subagents.fields.injectAgentsMdHint")}
          </p>
        </div>
      </label>
    </div>
  );
}
