import { useEffect, useMemo, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { Eye, EyeOff, Loader2, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  createAiChannel,
  deleteAiChannel,
  listAiChannelModels,
  listAiChannels,
  listModelCatalog,
  testAiChannel,
  updateAiChannel,
} from "@/lib/backend";
import {
  applyCatalogToModel,
  canSaveChannelModels,
  emptyChannelModel,
  materializeThinkingLevels,
} from "@/lib/modelCatalog";
import { formatDate } from "@/lib/utils";
import type { AiChannel, AiChannelModel, AiChannelProtocol, ModelCatalogEntry } from "@/lib/types";
import { ChannelModelsEditor } from "@/components/settings/ChannelModelsEditor";
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
import { useChannelStore } from "@/stores/channelStore";

interface ChannelFormState {
  name: string;
  protocol: AiChannelProtocol;
  baseUrl: string;
  apiKey: string;
  models: AiChannelModel[];
  extraHeaders: string;
  liteModel: string;
  enabled: boolean;
}

const EMPTY_FORM: ChannelFormState = {
  name: "",
  protocol: "openai",
  baseUrl: "",
  apiKey: "",
  models: [],
  extraHeaders: "",
  liteModel: "",
  enabled: true,
};

function channelToForm(channel: AiChannel): ChannelFormState {
  return {
    name: channel.name,
    protocol: channel.protocol,
    baseUrl: channel.base_url,
    apiKey: channel.api_key?.trim() ?? "",
    models: channel.models.length > 0 ? channel.models : [],
    extraHeaders: channel.extra_headers_json ?? "",
    liteModel: channel.lite_model ?? "",
    enabled: channel.enabled,
  };
}

export function AiChannelsSettingsTab() {
  const { t } = useTranslation("settings");
  const [channels, setChannels] = useState<AiChannel[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<"save" | "delete" | "test" | "models" | null>(null);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [form, setForm] = useState<ChannelFormState>(EMPTY_FORM);
  const [showApiKey, setShowApiKey] = useState(false);
  const [catalog, setCatalog] = useState<ModelCatalogEntry[]>([]);
  const [dialogOpen, setDialogOpen] = useState(false);

  const selected = useMemo(
    () => channels.find((channel) => channel.id === selectedId) ?? null,
    [channels, selectedId],
  );
  const isCreate = selectedId === null;

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const [items, catalogItems] = await Promise.all([listAiChannels(), listModelCatalog()]);
      setChannels(items);
      setCatalog(catalogItems);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const refreshChannelStore = () => {
    void useChannelStore.getState().load();
  };

  const openCreate = () => {
    setSelectedId(null);
    setForm(EMPTY_FORM);
    setShowApiKey(false);
    setMessage(null);
    setError(null);
    setDialogOpen(true);
  };

  const openEdit = (channel: AiChannel) => {
    setSelectedId(channel.id);
    setForm(channelToForm(channel));
    setShowApiKey(false);
    setMessage(null);
    setError(null);
    setDialogOpen(true);
  };

  const closeDialog = () => {
    if (saving !== null || deleteConfirming) {
      return;
    }
    setDialogOpen(false);
    setMessage(null);
    setError(null);
  };

  const patchForm = (updates: Partial<ChannelFormState>) => {
    setForm((current) => ({ ...current, ...updates }));
  };

  const handleSave = async () => {
    setError(null);
    setMessage(null);
    const models = form.models
      .map((item) => materializeThinkingLevels(catalog, item))
      .filter((item) => item.id.trim().length > 0);
    if (!canSaveChannelModels(models, catalog)) {
      setError(t("channels.fields.thinkingLevelsEmpty"));
      return;
    }
    setSaving("save");
    const extraHeaders = form.extraHeaders.trim() || null;
    try {
      if (selectedId) {
        const updated = await updateAiChannel(selectedId, {
          name: form.name,
          protocol: form.protocol,
          base_url: form.baseUrl,
          api_key: form.apiKey.trim() || undefined,
          extra_headers_json: extraHeaders,
          models,
          lite_model: form.liteModel.trim() || null,
          enabled: form.enabled,
        });
        setChannels((current) =>
          current.map((channel) => (channel.id === updated.id ? updated : channel)),
        );
        setDialogOpen(false);
        setMessage(t("channels.messages.updated"));
      } else {
        const created = await createAiChannel({
          name: form.name,
          protocol: form.protocol,
          base_url: form.baseUrl,
          api_key: form.apiKey.trim() || null,
          extra_headers_json: extraHeaders,
          models,
          lite_model: form.liteModel.trim() || null,
          enabled: form.enabled,
        });
        setChannels((current) => [created, ...current]);
        setSelectedId(created.id);
        setDialogOpen(false);
        setMessage(t("channels.messages.created"));
      }
      refreshChannelStore();
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
      const confirmed = await confirm(t("channels.dialogs.deleteConfirm", { name: targetName }), {
        title: t("channels.dialogs.deleteTitle"),
        kind: "warning",
      });
      if (!confirmed) return;
      setSaving("delete");
      await deleteAiChannel(targetId);
      setChannels((current) => current.filter((channel) => channel.id !== targetId));
      setSelectedId(null);
      setForm(EMPTY_FORM);
      setDialogOpen(false);
      setMessage(t("channels.messages.deleted"));
      refreshChannelStore();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeleteConfirming(false);
      setSaving(null);
    }
  };

  const channelRequestPayload = () => ({
    id: selectedId,
    protocol: form.protocol,
    base_url: form.baseUrl,
    api_key: form.apiKey.trim() || null,
    extra_headers_json: form.extraHeaders.trim() || null,
  });

  const handleFetchModels = async () => {
    setSaving("models");
    setError(null);
    setMessage(null);
    try {
      const result = await listAiChannelModels(channelRequestPayload());
      if (result.models.length > 0) {
        patchForm({
          models: result.models.map((id) => applyCatalogToModel(catalog, emptyChannelModel(id))),
        });
      }
      if (result.models.length === 0) {
        setError(result.message);
      } else {
        setMessage(
          result.truncated
            ? t("channels.messages.modelsFetchedTruncated", { count: result.models.length })
            : t("channels.messages.modelsFetched", { count: result.models.length }),
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const handleTest = async () => {
    setSaving("test");
    setError(null);
    setMessage(null);
    try {
      const result = await testAiChannel({
        ...channelRequestPayload(),
        model: form.models.find((item) => item.id.trim())?.id ?? null,
      });
      if (result.ok) {
        setMessage(result.message);
      } else {
        setError(result.message);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(null);
    }
  };

  const protocolOptions: Array<{ value: AiChannelProtocol; label: string }> = [
    { value: "openai", label: t("channels.protocols.openai") },
    { value: "anthropic", label: t("channels.protocols.anthropic") },
    { value: "codex", label: t("channels.protocols.codex") },
  ];
  const busy = saving !== null;
  const formLocked = busy || deleteConfirming;

  return (
    <div className="space-y-6">
      <div className="space-y-4 rounded-lg border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-sm font-medium">{t("channels.title")}</h3>
            <p className="text-xs text-muted-foreground">{t("channels.description")}</p>
          </div>
          <Button variant="outline" onClick={openCreate}>
            <Plus className="mr-1 h-4 w-4" />
            {t("channels.actions.new")}
          </Button>
        </div>

        {!dialogOpen && message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
        {!dialogOpen && error ? <p className="text-sm text-destructive">{error}</p> : null}

        <div className="rounded-md border border-border">
          {loading ? (
            <div className="flex h-28 items-center justify-center text-sm text-muted-foreground">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              {t("channels.list.loading")}
            </div>
          ) : channels.length === 0 ? (
            <div className="px-3 py-6 text-sm text-muted-foreground">
              {t("channels.list.empty")}
            </div>
          ) : (
            channels.map((channel) => (
              <button
                key={channel.id}
                type="button"
                onClick={() => openEdit(channel)}
                className={`w-full border-b border-border px-3 py-3 text-left last:border-b-0 ${
                  selectedId === channel.id ? "bg-primary/5" : "hover:bg-muted/40"
                }`}
              >
                <div className="text-sm font-medium">{channel.name}</div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {protocolOptions.find((option) => option.value === channel.protocol)?.label ??
                    channel.protocol}{" "}
                  · {channel.enabled ? t("channels.status.enabled") : t("channels.status.disabled")}
                </div>
              </button>
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
              {isCreate ? t("channels.dialogs.createTitle") : t("channels.dialogs.editTitle")}
            </DialogTitle>
            <DialogDescription>{t("channels.description")}</DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto pr-1">
            <div className="space-y-3">
              <Input
                value={form.name}
                onChange={(event) => patchForm({ name: event.target.value })}
                placeholder={t("channels.fields.name")}
                disabled={formLocked}
              />
              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("channels.fields.protocol")}
                </label>
                <Select
                  value={form.protocol}
                  disabled={formLocked}
                  onValueChange={(value) => {
                    if (value === "openai" || value === "anthropic" || value === "codex") {
                      patchForm({ protocol: value });
                    }
                  }}
                >
                  <SelectTrigger className="mt-1 bg-background">
                    <SelectValue>
                      {(value) =>
                        typeof value === "string"
                          ? (protocolOptions.find((option) => option.value === value)?.label ??
                            value)
                          : t("channels.fields.protocol")
                      }
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {protocolOptions.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <Input
                value={form.baseUrl}
                onChange={(event) => patchForm({ baseUrl: event.target.value })}
                placeholder={t("channels.fields.baseUrl")}
                disabled={formLocked}
              />
              <div className="relative">
                <Input
                  type={showApiKey ? "text" : "password"}
                  className="pr-9"
                  value={form.apiKey}
                  onChange={(event) => patchForm({ apiKey: event.target.value })}
                  placeholder={
                    selected?.api_key_configured && !form.apiKey.trim()
                      ? t("channels.fields.apiKeyConfigured")
                      : t("channels.fields.apiKey")
                  }
                  autoComplete="off"
                  disabled={formLocked}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="absolute top-1/2 right-1 -translate-y-1/2 text-muted-foreground"
                  onClick={() => setShowApiKey((current) => !current)}
                  disabled={formLocked}
                  aria-label={
                    showApiKey ? t("channels.fields.hideApiKey") : t("channels.fields.showApiKey")
                  }
                >
                  {showApiKey ? <EyeOff /> : <Eye />}
                </Button>
              </div>
              <ChannelModelsEditor
                models={form.models}
                catalog={catalog}
                disabled={formLocked}
                onChange={(models) => patchForm({ models })}
              />
              <Textarea
                value={form.extraHeaders}
                onChange={(event) => patchForm({ extraHeaders: event.target.value })}
                placeholder={t("channels.fields.extraHeaders")}
                rows={3}
                disabled={formLocked}
              />
              <div>
                <label
                  className="text-xs font-medium text-muted-foreground"
                  htmlFor="channel-lite-model"
                >
                  {t("channels.fields.liteModel")}
                </label>
                <select
                  id="channel-lite-model"
                  className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-sm"
                  value={form.liteModel}
                  disabled={formLocked}
                  onChange={(event) => patchForm({ liteModel: event.target.value })}
                >
                  <option value="">{t("channels.fields.liteModelNone")}</option>
                  {form.models
                    .filter((item) => item.id.trim().length > 0)
                    .map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.id}
                      </option>
                    ))}
                </select>
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("channels.fields.liteModelHint")}
                </p>
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("channels.fields.enabled")}
                </label>
                <Select
                  value={form.enabled ? "enabled" : "disabled"}
                  disabled={formLocked}
                  onValueChange={(value) => {
                    if (value === "enabled" || value === "disabled") {
                      patchForm({ enabled: value === "enabled" });
                    }
                  }}
                >
                  <SelectTrigger className="mt-1 bg-background">
                    <SelectValue>
                      {(value) =>
                        value === "enabled"
                          ? t("channels.status.enabled")
                          : value === "disabled"
                            ? t("channels.status.disabled")
                            : t("channels.fields.enabled")
                      }
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="enabled">{t("channels.status.enabled")}</SelectItem>
                    <SelectItem value="disabled">{t("channels.status.disabled")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {selected ? (
                <p className="text-xs text-muted-foreground">
                  {t("channels.updatedAt", { date: formatDate(selected.updated_at) })}
                </p>
              ) : null}
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  onClick={() => void handleFetchModels()}
                  disabled={formLocked}
                >
                  {saving === "models" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                  {t("channels.actions.fetchModels")}
                </Button>
                <Button variant="outline" onClick={() => void handleTest()} disabled={formLocked}>
                  {saving === "test" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                  {t("channels.actions.test")}
                </Button>
              </div>
              {error ? <p className="text-sm text-destructive">{error}</p> : null}
              {message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
            </div>
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
                {t("channels.actions.delete")}
              </Button>
            ) : null}
            <Button variant="outline" onClick={closeDialog} disabled={formLocked}>
              {t("channels.actions.cancel")}
            </Button>
            <Button onClick={() => void handleSave()} disabled={formLocked}>
              {saving === "save" ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("channels.actions.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
