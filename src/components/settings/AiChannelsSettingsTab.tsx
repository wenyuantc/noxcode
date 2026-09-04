import { useEffect, useMemo, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  Bot,
  Cpu,
  Eye,
  EyeOff,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Sparkles,
  Trash2,
  Zap,
} from "lucide-react";
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
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

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
  const [testingId, setTestingId] = useState<string | null>(null);
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
    try {
      const [data, catalogData] = await Promise.all([listAiChannels(), listModelCatalog()]);
      setChannels(data);
      setCatalog(catalogData);
      setError(null);
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
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const openEdit = (channel: AiChannel) => {
    setSelectedId(channel.id);
    setForm(channelToForm(channel));
    setShowApiKey(false);
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const closeDialog = () => {
    if (saving !== null || deleteConfirming) return;
    setDialogOpen(false);
    setSelectedId(null);
    setForm(EMPTY_FORM);
    setShowApiKey(false);
  };

  const patchForm = (patch: Partial<ChannelFormState>) => {
    setForm((current) => ({ ...current, ...patch }));
  };

  const handleSave = async () => {
    if (!form.name.trim()) {
      setError(t("channels.validation.nameRequired"));
      return;
    }
    if (!form.baseUrl.trim()) {
      setError(t("channels.validation.baseUrlRequired"));
      return;
    }
    if (form.models.length > 0 && !canSaveChannelModels(form.models)) {
      setError(t("channels.validation.modelIdsRequired"));
      return;
    }
    if (form.extraHeaders.trim().length > 0) {
      try {
        const parsed: unknown = JSON.parse(form.extraHeaders);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
          setError(t("channels.validation.extraHeadersJsonObject"));
          return;
        }
      } catch {
        setError(t("channels.validation.extraHeadersJsonInvalid"));
        return;
      }
    }

    setSaving("save");
    setError(null);
    setMessage(null);

    const materializedModels = form.models.map((m) => materializeThinkingLevels(catalog, m));

    try {
      if (selected) {
        const updated = await updateAiChannel(selected.id, {
          name: form.name,
          protocol: form.protocol,
          base_url: form.baseUrl,
          api_key: form.apiKey.trim() ? form.apiKey.trim() : null,
          models: materializedModels,
          extra_headers_json: form.extraHeaders.trim() || null,
          lite_model: form.liteModel.trim() || null,
          enabled: form.enabled,
        });
        setChannels((current) =>
          current.map((channel) => (channel.id === updated.id ? updated : channel)),
        );
        setSelectedId(updated.id);
        setDialogOpen(false);
        setMessage(t("channels.messages.saved"));
      } else {
        const created = await createAiChannel({
          name: form.name,
          protocol: form.protocol,
          base_url: form.baseUrl,
          api_key: form.apiKey.trim() || null,
          models: materializedModels,
          extra_headers_json: form.extraHeaders.trim() || null,
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
        setMessage(t("channels.messages.modelsFetched", { count: result.models.length }));
      } else {
        setMessage(t("channels.messages.noModelsFetched"));
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

  const handleQuickTest = async (channel: AiChannel) => {
    setTestingId(channel.id);
    setError(null);
    setMessage(null);
    try {
      const result = await testAiChannel({
        id: channel.id,
        protocol: channel.protocol,
        base_url: channel.base_url,
        api_key: channel.api_key?.trim() || null,
        extra_headers_json: channel.extra_headers_json?.trim() || null,
        model: channel.models.find((item) => item.id.trim())?.id ?? null,
      });
      if (result.ok) {
        setMessage(`[${channel.name}] 测通成功: ${result.message}`);
      } else {
        setError(`[${channel.name}] 测通失败: ${result.message}`);
      }
    } catch (err) {
      setError(`[${channel.name}] 测通异常: ${String(err)}`);
    } finally {
      setTestingId(null);
    }
  };

  const protocolOptions: Array<{ value: AiChannelProtocol; label: string }> = [
    { value: "openai", label: t("channels.protocols.openai") },
    { value: "anthropic", label: t("channels.protocols.anthropic") },
    { value: "codex", label: t("channels.protocols.codex") },
  ];
  const busy = saving !== null;
  const formLocked = busy || deleteConfirming;

  const getProtocolIcon = (proto: AiChannelProtocol) => {
    if (proto === "anthropic") return Bot;
    if (proto === "codex") return Cpu;
    return Sparkles;
  };

  return (
    <div className="space-y-6">
      {/* 外部反馈提示 */}
      {!dialogOpen && message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {!dialogOpen && error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}

      <SettingCard
        icon={Sparkles}
        title={t("channels.title")}
        description={t("channels.description")}
        badge={`${channels.length} 个渠道`}
        headerAction={
          <Button size="sm" onClick={openCreate} className="h-7 gap-1 text-xs">
            <Plus className="size-3.5" />
            {t("channels.actions.new")}
          </Button>
        }
      >
        {loading ? (
          <div className="flex h-36 items-center justify-center text-xs text-muted-foreground">
            <Loader2 className="mr-2 size-4 animate-spin text-primary" />
            {t("channels.list.loading")}
          </div>
        ) : channels.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <div className="flex size-12 items-center justify-center rounded-2xl border border-border/70 bg-muted/30 text-muted-foreground shadow-2xs">
              <Sparkles className="size-6" />
            </div>
            <p className="mt-3 text-xs font-semibold text-foreground">{t("channels.list.empty")}</p>
            <p className="mt-1 text-[11px] text-muted-foreground max-w-sm">
              添加 OpenAI、Anthropic 或自定义网关，即可开始使用多模型能力。
            </p>
            <Button size="sm" onClick={openCreate} className="mt-4 h-7 gap-1 text-xs">
              <Plus className="size-3.5" />
              {t("channels.actions.new")}
            </Button>
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-1">
            {channels.map((channel) => {
              const ProtoIcon = getProtocolIcon(channel.protocol);
              const isTesting = testingId === channel.id;

              return (
                <div
                  key={channel.id}
                  className="group relative flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all duration-150 hover:border-border hover:shadow-xs"
                >
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                      <ProtoIcon className="size-4" />
                    </div>
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold tracking-tight text-foreground truncate">
                          {channel.name}
                        </span>
                        {/* 状态指示灯 */}
                        <div className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 text-[10px] font-mono">
                          <span
                            className={`size-1.5 rounded-full ${
                              channel.enabled
                                ? "bg-emerald-500 shadow-2xs shadow-emerald-500/50"
                                : "bg-muted-foreground/40"
                            }`}
                          />
                          <span className="text-muted-foreground">
                            {channel.enabled
                              ? t("channels.status.enabled")
                              : t("channels.status.disabled")}
                          </span>
                        </div>
                        {/* 协议 Badge */}
                        <span className="rounded-md border border-border/50 bg-background px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground uppercase">
                          {channel.protocol}
                        </span>
                        {/* 模型数 Badge */}
                        <span className="rounded-md bg-muted px-1.5 py-0.2 text-[10px] font-mono text-muted-foreground">
                          {channel.models.length} 模型
                        </span>
                        {channel.lite_model ? (
                          <span className="rounded-md bg-primary/10 px-1.5 py-0.2 text-[10px] font-mono text-primary truncate max-w-36">
                            ⚡ {channel.lite_model}
                          </span>
                        ) : null}
                      </div>
                      <p className="text-[11px] font-mono text-muted-foreground/80 truncate max-w-md">
                        {channel.base_url}
                      </p>
                    </div>
                  </div>

                  {/* 快捷操作 */}
                  <div className="flex items-center gap-1.5 shrink-0 self-end sm:self-center">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={isTesting}
                      onClick={() => void handleQuickTest(channel)}
                      className="h-7 text-xs gap-1"
                    >
                      {isTesting ? (
                        <Loader2 className="size-3 animate-spin" />
                      ) : (
                        <Zap className="size-3" />
                      )}
                      {t("channels.actions.test")}
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => openEdit(channel)}
                      className="h-7 text-xs gap-1"
                    >
                      <Pencil className="size-3" />
                      {t("channels.actions.edit", { defaultValue: "编辑" })}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </SettingCard>

      {/* 编辑/新建渠道 Dialog */}
      <Dialog
        open={dialogOpen}
        onOpenChange={(open) => {
          if (!open) closeDialog();
        }}
      >
        <DialogContent
          className="flex max-h-[90vh] w-full flex-col gap-0 overflow-hidden sm:max-w-2xl rounded-2xl p-0"
          showCloseButton={!formLocked}
        >
          <DialogHeader className="shrink-0 border-b border-border/50 px-6 py-4">
            <DialogTitle className="text-base font-semibold tracking-tight">
              {isCreate ? t("channels.dialogs.createTitle") : t("channels.dialogs.editTitle")}
            </DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground">
              {t("channels.description")}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
            <div className="space-y-4">
              {error ? <SettingFeedbackCallout variant="error" message={error} /> : null}
              {message ? <SettingFeedbackCallout variant="success" message={message} /> : null}

              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("channels.fields.name")}
                  </label>
                  <Input
                    className="mt-1 h-8 text-xs"
                    value={form.name}
                    onChange={(event) => patchForm({ name: event.target.value })}
                    placeholder="如：OpenAI Official"
                    disabled={formLocked}
                  />
                </div>
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
                    <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {protocolOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value} className="text-xs">
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("channels.fields.baseUrl")}
                </label>
                <Input
                  className="mt-1 h-8 text-xs font-mono"
                  value={form.baseUrl}
                  onChange={(event) => patchForm({ baseUrl: event.target.value })}
                  placeholder="https://api.openai.com/v1"
                  disabled={formLocked}
                />
              </div>

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("channels.fields.apiKey")}
                </label>
                <div className="relative mt-1">
                  <Input
                    type={showApiKey ? "text" : "password"}
                    className="h-8 pr-9 text-xs font-mono"
                    value={form.apiKey}
                    onChange={(event) => patchForm({ apiKey: event.target.value })}
                    placeholder={
                      selected?.api_key_configured && !form.apiKey.trim()
                        ? t("channels.fields.apiKeyConfigured")
                        : "sk-..."
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
                  >
                    {showApiKey ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                  </Button>
                </div>
              </div>

              <div className="rounded-xl border border-border/70 bg-muted/20 p-3">
                <ChannelModelsEditor
                  models={form.models}
                  catalog={catalog}
                  disabled={formLocked}
                  onChange={(models) => patchForm({ models })}
                />
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <label
                    className="text-xs font-medium text-muted-foreground"
                    htmlFor="channel-lite-model"
                  >
                    {t("channels.fields.liteModel")}
                  </label>
                  <Select
                    value={form.liteModel || "none"}
                    disabled={formLocked}
                    onValueChange={(val) =>
                      patchForm({ liteModel: !val || val === "none" ? "" : val })
                    }
                  >
                    <SelectTrigger
                      id="channel-lite-model"
                      className="mt-1 h-8 text-xs bg-background"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none" className="text-xs">
                        {t("channels.fields.liteModelNone")}
                      </SelectItem>
                      {form.models
                        .filter((item) => item.id.trim().length > 0)
                        .map((item) => (
                          <SelectItem key={item.id} value={item.id} className="text-xs font-mono">
                            {item.id}
                          </SelectItem>
                        ))}
                    </SelectContent>
                  </Select>
                </div>

                <div>
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("channels.fields.enabled")}
                  </label>
                  <Select
                    value={form.enabled ? "enabled" : "disabled"}
                    disabled={formLocked}
                    onValueChange={(value) => patchForm({ enabled: value === "enabled" })}
                  >
                    <SelectTrigger className="mt-1 h-8 text-xs bg-background">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="enabled" className="text-xs">
                        {t("channels.status.enabled")}
                      </SelectItem>
                      <SelectItem value="disabled" className="text-xs">
                        {t("channels.status.disabled")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              <div>
                <label className="text-xs font-medium text-muted-foreground">
                  {t("channels.fields.extraHeaders")}
                </label>
                <Textarea
                  className="mt-1 resize-none font-mono text-xs leading-relaxed"
                  value={form.extraHeaders}
                  onChange={(event) => patchForm({ extraHeaders: event.target.value })}
                  placeholder='{"HTTP-Referer": "https://..."}'
                  rows={2}
                  disabled={formLocked}
                />
              </div>

              {selected ? (
                <p className="text-[11px] font-mono text-muted-foreground">
                  {t("channels.updatedAt", { date: formatDate(selected.updated_at) })}
                </p>
              ) : null}

              <div className="flex flex-wrap gap-2 pt-1">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 text-xs gap-1"
                  onClick={() => void handleFetchModels()}
                  disabled={formLocked}
                >
                  {saving === "models" ? (
                    <Loader2 className="size-3 animate-spin" />
                  ) : (
                    <RefreshCw className="size-3" />
                  )}
                  {t("channels.actions.fetchModels")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 text-xs gap-1"
                  onClick={() => void handleTest()}
                  disabled={formLocked}
                >
                  {saving === "test" ? (
                    <Loader2 className="size-3 animate-spin" />
                  ) : (
                    <Zap className="size-3" />
                  )}
                  {t("channels.actions.test")}
                </Button>
              </div>
            </div>
          </div>

          <DialogFooter className="shrink-0 border-t border-border/50 px-6 py-3 bg-muted/10">
            {!isCreate ? (
              <Button
                variant="destructive"
                size="sm"
                className="sm:mr-auto h-8 text-xs"
                onClick={() => void handleDelete()}
                disabled={formLocked}
              >
                {saving === "delete" ? (
                  <Loader2 className="mr-1 size-3.5 animate-spin" />
                ) : (
                  <Trash2 className="mr-1 size-3.5" />
                )}
                {t("channels.actions.delete")}
              </Button>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs"
              onClick={closeDialog}
              disabled={formLocked}
            >
              {t("channels.actions.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-8 text-xs"
              onClick={() => void handleSave()}
              disabled={formLocked}
            >
              {saving === "save" ? <Loader2 className="mr-1 size-3.5 animate-spin" /> : null}
              {t("channels.actions.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
