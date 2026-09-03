import { useEffect, useMemo, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { Loader2, Pencil, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  createNativeSubagent,
  deleteNativeSubagent,
  listAiChannels,
  listNativeSubagents,
  listWorkspaces,
  NATIVE_SUBAGENT_CUSTOM_TOOLS,
  updateNativeSubagent,
} from "@/lib/backend";
import type {
  AiChannel,
  CreateNativeSubagentInput,
  NativeSubagent,
  UpdateNativeSubagentInput,
  Workspace,
} from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useWorkspaceStore } from "@/stores/workspaceStore";

type NativeSubagentModelMode = "inherit" | "channel";
type NativeSubagentToolMode = "all" | "custom";
type NativeSubagentScope = "all" | "workspaces";

interface SubagentFormState {
  name: string;
  description: string;
  modelMode: NativeSubagentModelMode;
  channelId: string;
  model: string;
  toolMode: NativeSubagentToolMode;
  tools: string[];
  systemPrompt: string;
  injectAgentsMd: boolean;
  scope: NativeSubagentScope;
  workspaceIds: string[];
}

const EMPTY_FORM: SubagentFormState = {
  name: "",
  description: "",
  modelMode: "inherit",
  channelId: "",
  model: "",
  toolMode: "all",
  tools: [],
  systemPrompt: "",
  injectAgentsMd: true,
  scope: "all",
  workspaceIds: [],
};

function toForm(item: NativeSubagent, workspaces: Workspace[]): SubagentFormState {
  const liveIds = new Set(workspaces.map((workspace) => workspace.id));
  return {
    name: item.name,
    description: item.description,
    modelMode: item.model_mode === "channel" ? "channel" : "inherit",
    channelId: item.channel_id ?? "",
    model: item.model ?? "",
    toolMode: item.tool_mode === "custom" ? "custom" : "all",
    tools: item.tools,
    systemPrompt: item.system_prompt,
    injectAgentsMd: item.inject_agents_md !== false,
    scope: item.scope === "workspaces" ? "workspaces" : "all",
    workspaceIds: (item.workspace_ids ?? []).filter((id) => liveIds.has(id)),
  };
}

function payloadFrom(
  state: SubagentFormState,
): CreateNativeSubagentInput & UpdateNativeSubagentInput {
  return {
    name: state.name,
    description: state.description,
    model_mode: state.modelMode,
    channel_id: state.modelMode === "channel" ? state.channelId || null : null,
    model: state.modelMode === "channel" ? state.model || null : null,
    tool_mode: state.toolMode,
    tools: state.toolMode === "custom" ? state.tools : [],
    system_prompt: state.systemPrompt,
    inject_agents_md: state.injectAgentsMd,
    scope: state.scope,
    workspace_ids: state.scope === "workspaces" ? state.workspaceIds : [],
  };
}

function SubagentFormFields({
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

export function SubagentsSettingsTab() {
  const { t } = useTranslation("settings");
  const [items, setItems] = useState<NativeSubagent[]>([]);
  const [channels, setChannels] = useState<AiChannel[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<"save" | "delete" | "create" | null>(null);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [form, setForm] = useState<SubagentFormState>(EMPTY_FORM);
  const [dialogOpen, setDialogOpen] = useState(false);

  const selected = useMemo(
    () => items.find((item) => item.id === selectedId) ?? null,
    [items, selectedId],
  );
  const enabledChannels = useMemo(() => channels.filter((channel) => channel.enabled), [channels]);
  const isCreate = selectedId === null;

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const [subagents, channelItems, workspaceItems] = await Promise.all([
        listNativeSubagents(useWorkspaceStore.getState().activeWorkspaceId),
        listAiChannels(),
        listWorkspaces(),
      ]);
      setItems(subagents);
      setChannels(channelItems);
      setWorkspaces(workspaceItems);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const openCreate = () => {
    setSelectedId(null);
    setForm(EMPTY_FORM);
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const openEdit = (item: NativeSubagent) => {
    setSelectedId(item.id);
    setForm(toForm(item, workspaces));
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const closeDialog = () => {
    if (saving !== null || deleteConfirming) {
      return;
    }
    setDialogOpen(false);
    setError(null);
  };

  const handleCreate = async () => {
    setSaving("create");
    setError(null);
    try {
      const created = await createNativeSubagent(payloadFrom(form));
      setItems((current) => [created, ...current]);
      setSelectedId(created.id);
      setDialogOpen(false);
      setMessage(t("subagents.messages.created"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleSave = async () => {
    if (!selectedId) {
      return;
    }
    setSaving("save");
    setError(null);
    try {
      const updated = await updateNativeSubagent(selectedId, payloadFrom(form));
      setItems((current) => current.map((item) => (item.id === updated.id ? updated : item)));
      setDialogOpen(false);
      setMessage(t("subagents.messages.updated"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleDelete = async () => {
    if (!selected || saving !== null || deleteConfirming) return;
    const targetId = selected.id;
    const targetName = selected.name;
    setDeleteConfirming(true);
    setError(null);
    setMessage(null);
    try {
      const confirmed = await confirm(t("subagents.dialogs.deleteConfirm", { name: targetName }), {
        title: t("subagents.dialogs.deleteTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("delete");
      await deleteNativeSubagent(targetId);
      setItems((current) => current.filter((item) => item.id !== targetId));
      setSelectedId(null);
      setForm(EMPTY_FORM);
      setDialogOpen(false);
      setMessage(t("subagents.messages.deleted"));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleteConfirming(false);
      setSaving(null);
    }
  };

  const busy = saving !== null;
  const formLocked = busy || deleteConfirming;
  const canSubmit =
    form.name.trim().length > 0 &&
    form.description.trim().length > 0 &&
    (form.scope !== "workspaces" || form.workspaceIds.length > 0) &&
    (form.modelMode !== "channel" ||
      (form.channelId.trim().length > 0 && form.model.trim().length > 0)) &&
    (form.toolMode !== "custom" || form.tools.length > 0);
  const builtinItems = [
    { id: "general", name: "general", hintKey: "subagents.builtin.general" },
    { id: "explore", name: "explore", hintKey: "subagents.builtin.explore" },
  ] as const;

  return (
    <div className="space-y-6">
      <div className="space-y-4 rounded-lg border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-sm font-medium">{t("subagents.title")}</h3>
            <p className="text-xs text-muted-foreground">{t("subagents.description")}</p>
            <p className="mt-2 text-xs text-muted-foreground">{t("subagents.howToCall")}</p>
            <p className="mt-1 text-xs text-muted-foreground">{t("subagents.profilesHint")}</p>
          </div>
          <Button variant="outline" onClick={openCreate}>
            <Plus className="mr-1 h-4 w-4" />
            {t("subagents.actions.new")}
          </Button>
        </div>

        {!dialogOpen && message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
        {!dialogOpen && error ? <p className="text-sm text-destructive">{error}</p> : null}

        <div className="rounded-md border border-border">
          {builtinItems.map((item) => (
            <div key={item.id} className="border-b border-border px-3 py-3">
              <div className="text-sm font-medium">{item.name}</div>
              <div className="mt-1 text-xs text-muted-foreground">{t(item.hintKey)}</div>
            </div>
          ))}
          {loading ? (
            <div className="flex h-28 items-center justify-center text-sm text-muted-foreground">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("subagents.list.loading")}
            </div>
          ) : items.length === 0 ? (
            <div className="px-3 py-6 text-sm text-muted-foreground">
              {t("subagents.list.empty")}
            </div>
          ) : (
            items.map((item) => (
              <div
                key={item.id}
                className="flex items-start justify-between gap-3 border-b border-border px-3 py-3 last:border-b-0"
              >
                <div className="min-w-0">
                  <div className="text-sm font-medium">{item.name}</div>
                  <div className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                    {item.description}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {item.scope === "workspaces"
                      ? t("subagents.fields.scopeWorkspacesCount", {
                          count: (item.workspace_ids ?? []).length,
                        })
                      : t("subagents.fields.scopeAll")}
                  </div>
                </div>
                {item.source === "file" ? (
                  <span
                    className="shrink-0 rounded-md border px-2 py-1 text-xs text-muted-foreground"
                    title={item.path ?? ""}
                  >
                    {t("subagents.fields.fileProfile")}
                  </span>
                ) : (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="shrink-0"
                    onClick={() => openEdit(item)}
                  >
                    <Pencil className="mr-1 h-3.5 w-3.5" />
                    {t("subagents.actions.edit")}
                  </Button>
                )}
              </div>
            ))
          )}
        </div>
      </div>

      <Dialog
        open={dialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closeDialog();
          }
        }}
      >
        <DialogContent
          className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl"
          showCloseButton={!formLocked}
        >
          <DialogHeader className="shrink-0 pb-3">
            <DialogTitle>
              {isCreate ? t("subagents.dialogs.createTitle") : t("subagents.dialogs.editTitle")}
            </DialogTitle>
            <DialogDescription>{t("subagents.description")}</DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <SubagentFormFields
              form={form}
              enabledChannels={enabledChannels}
              workspaces={workspaces}
              busy={formLocked}
              onPatch={(updates) => setForm((current) => ({ ...current, ...updates }))}
            />
            {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}
          </div>
          <DialogFooter className="mt-4 shrink-0">
            {!isCreate ? (
              <Button
                variant="destructive"
                className="sm:mr-auto"
                onClick={() => void handleDelete()}
                disabled={formLocked}
              >
                {saving === "delete" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                <Trash2 className="mr-1 h-4 w-4" />
                {t("subagents.actions.delete")}
              </Button>
            ) : null}
            <Button variant="outline" onClick={closeDialog} disabled={formLocked}>
              {t("subagents.actions.cancel")}
            </Button>
            {isCreate ? (
              <Button onClick={() => void handleCreate()} disabled={formLocked || !canSubmit}>
                {saving === "create" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                {t("subagents.actions.create")}
              </Button>
            ) : (
              <Button onClick={() => void handleSave()} disabled={formLocked || !canSubmit}>
                {saving === "save" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                {t("subagents.actions.save")}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
