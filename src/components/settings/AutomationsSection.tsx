import { useCallback, useEffect, useState } from "react";
import { Clock, Loader2, Play, Plus, Trash2, Pencil, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import {
  createNativeAutomation,
  deleteNativeAutomation,
  listNativeAutomations,
  runNativeAutomationNow,
  updateNativeAutomation,
} from "@/lib/backend";
import type { NativeAutomation } from "@/lib/types";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useChannelStore } from "@/stores/channelStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { useSessionStore } from "@/stores/sessionStore";
import { SettingCard } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

export function AutomationsSection() {
  const { t } = useTranslation(["settings", "common"]);
  const navigate = useNavigate();
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const model = useChannelStore((state) => state.activeModelId);
  const [items, setItems] = useState<NativeAutomation[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [runningId, setRunningId] = useState<string | null>(null);
  const [editing, setEditing] = useState<NativeAutomation | null>(null);
  const [resultSessionId, setResultSessionId] = useState<string | null>(null);
  const [formChannelId, setFormChannelId] = useState("");
  const [formModel, setFormModel] = useState("");
  const channels = useChannelStore((state) => state.channels);

  // Form State
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * mon-fri");
  const [prompt, setPrompt] = useState("");

  const reload = useCallback(() => {
    if (!workspaceId) {
      setItems([]);
      return;
    }
    listNativeAutomations(workspaceId)
      .then((next) => {
        setItems(next);
        setError(null);
      })
      .catch((reason: unknown) => setError(String(reason)));
  }, [workspaceId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const openCreate = () => {
    setEditing(null);
    setFormChannelId(channelId ?? "");
    setFormModel(model ?? "");
    setName("");
    setCron("0 9 * * mon-fri");
    setPrompt("");
    setError(null);
    setMessage(null);
    setDialogOpen(true);
  };

  const openEdit = (item: NativeAutomation) => {
    setEditing(item);
    setName(item.name);
    setCron(item.cron);
    setPrompt(item.prompt);
    setFormChannelId(item.channel_id ?? "");
    setFormModel(item.model ?? "");
    setError(null);
    setDialogOpen(true);
  };
  const openResult = async (sessionId: string) => {
    try {
      await useWorkspaceStore.getState().refreshSessions();
      await useSessionStore.getState().loadHistory(sessionId);
      await navigate("/");
    } catch (reason) {
      setError(String(reason));
    }
  };

  const create = async () => {
    if (!workspaceId || !name.trim() || !cron.trim() || !prompt.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const values = {
        name: name.trim(),
        cron: cron.trim(),
        prompt: prompt.trim(),
        channel_id: formChannelId || null,
        model: formModel || null,
      };
      if (editing) await updateNativeAutomation(editing.id, values);
      else await createNativeAutomation({ ...values, workspace_id: workspaceId, enabled: true });
      setName("");
      setPrompt("");
      setDialogOpen(false);
      setMessage(t("common:saved", { defaultValue: "任务创建成功" }));
      reload();
    } catch (reason: unknown) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const toggle = (item: NativeAutomation, enabled: boolean) => {
    updateNativeAutomation(item.id, { enabled })
      .then(() => reload())
      .catch((reason: unknown) => setError(String(reason)));
  };

  const remove = (item: NativeAutomation) => {
    deleteNativeAutomation(item.id)
      .then(() => {
        setMessage(t("common:deleted", { defaultValue: "已删除" }));
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)));
  };

  const runNow = (item: NativeAutomation) => {
    setRunningId(item.id);
    runNativeAutomationNow(item.id)
      .then((sessionId) => {
        setMessage(t("settings:automations.started", { session: sessionId }));
        setResultSessionId(sessionId);
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)))
      .finally(() => setRunningId(null));
  };

  return (
    <div className="space-y-6">
      {message ? (
        <SettingFeedbackCallout
          variant="success"
          message={message}
          onClose={() => setMessage(null)}
        />
      ) : null}
      {error ? (
        <SettingFeedbackCallout variant="error" message={error} onClose={() => setError(null)} />
      ) : null}
      {resultSessionId ? (
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void openResult(resultSessionId);
          }}
        >
          <ExternalLink className="size-3.5" />
          查看运行会话
        </Button>
      ) : null}

      <SettingCard
        icon={Clock}
        title={t("settings:automations.listTitle")}
        description={
          workspaceId ? t("settings:automations.listHint") : t("settings:automations.needWorkspace")
        }
        badge={`${items.length} 个任务`}
        headerAction={
          <Button
            size="sm"
            onClick={openCreate}
            disabled={!workspaceId}
            className="h-7 gap-1 text-xs"
          >
            <Plus className="size-3.5" />
            {t("settings:automations.create")}
          </Button>
        }
      >
        {items.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-center">
            <div className="flex size-10 items-center justify-center rounded-xl border border-border/70 bg-muted/30 text-muted-foreground">
              <Clock className="size-5" />
            </div>
            <p className="mt-3 text-xs font-semibold text-foreground">
              {t("settings:automations.empty")}
            </p>
            <p className="mt-1 text-[11px] text-muted-foreground max-w-sm">
              可配置定时周期任务（如工作日早上9点摸底代码库、生成周报或跑检查）。
            </p>
            {workspaceId ? (
              <Button size="sm" onClick={openCreate} className="mt-4 h-7 gap-1 text-xs">
                <Plus className="size-3.5" />
                {t("settings:automations.create")}
              </Button>
            ) : null}
          </div>
        ) : (
          <div className="grid gap-2.5">
            {items.map((item) => {
              const isRunning = runningId === item.id;
              return (
                <div
                  key={item.id}
                  className="group flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-border hover:shadow-xs"
                >
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                      <Clock className="size-4" />
                    </div>
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold tracking-tight text-foreground truncate">
                          {item.name}
                        </span>
                        <code className="rounded-md border border-border/60 bg-muted/50 px-1.5 py-0.2 font-mono text-[10px] text-muted-foreground">
                          {item.cron}
                        </code>
                      </div>
                      <p className="line-clamp-2 text-[11px] text-muted-foreground leading-relaxed">
                        {item.prompt}
                      </p>
                      <div className="flex flex-wrap items-center gap-2 text-[10px] font-mono text-muted-foreground/75">
                        <span>
                          {t("settings:automations.next", { time: item.next_run_at ?? "-" })}
                        </span>
                        <span>·</span>
                        <span>
                          {t("settings:automations.last", { time: item.last_run_at ?? "-" })}
                        </span>
                        {item.last_error ? (
                          <span className="text-destructive font-sans truncate max-w-xs">
                            · {item.last_error}
                          </span>
                        ) : null}
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center gap-2 shrink-0 self-end sm:self-center">
                    {item.last_session_id ? (
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        title="查看最近运行会话"
                        aria-label="查看最近运行会话"
                        onClick={() => {
                          void openResult(item.last_session_id!);
                        }}
                      >
                        <ExternalLink className="size-3.5" />
                      </Button>
                    ) : null}
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      title="编辑自动化"
                      aria-label="编辑自动化"
                      onClick={() => openEdit(item)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Switch
                      checked={item.enabled !== 0}
                      onCheckedChange={(checked) => toggle(item, checked)}
                      aria-label="启用自动化"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={isRunning}
                      onClick={() => runNow(item)}
                      className="h-7 text-xs gap-1"
                    >
                      {isRunning ? (
                        <Loader2 className="size-3 animate-spin" />
                      ) : (
                        <Play className="size-3" />
                      )}
                      {t("settings:automations.runNow")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      className="text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100"
                      onClick={() => remove(item)}
                      title={t("common:delete")}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </SettingCard>

      {/* 新建定时任务 Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md rounded-2xl p-0 overflow-hidden">
          <DialogHeader className="border-b border-border/50 px-6 py-4">
            <DialogTitle className="text-base font-semibold tracking-tight">
              {editing ? "编辑自动化" : t("settings:automations.createTitle")}
            </DialogTitle>
            <DialogDescription className="text-xs text-muted-foreground">
              {t("settings:automations.createHint")}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3.5 px-6 py-4">
            {error ? (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            ) : null}
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("settings:automations.name")}
              </label>
              <Input
                className="mt-1 h-8 text-xs"
                placeholder="任务名称（如：每日代码自检）"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("settings:automations.cron")}
              </label>
              <Input
                className="mt-1 h-8 text-xs font-mono"
                placeholder="0 9 * * mon-fri"
                value={cron}
                onChange={(e) => setCron(e.target.value)}
              />
              <p className="mt-1 text-[10px] text-muted-foreground">
                标准 5 字段 Cron 表达式（分 时 日 月 周），例如 "*/30 * * * *" 每30分钟执行
              </p>
            </div>
            <div>
              <label className="text-xs font-medium text-muted-foreground">
                {t("settings:automations.prompt")}
              </label>
              <Textarea
                className="mt-1 resize-none text-xs leading-relaxed"
                rows={4}
                placeholder="给 Agent 的自动执行指令…"
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
              />
            </div>
            <label className="block text-xs">
              渠道
              <select
                aria-label="自动化渠道"
                className="mt-1 h-8 w-full rounded-md border bg-background px-2"
                value={formChannelId}
                onChange={(event) => {
                  setFormChannelId(event.target.value);
                  setFormModel("");
                }}
              >
                <option value="">默认渠道</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="block text-xs">
              模型
              <select
                aria-label="自动化模型"
                className="mt-1 h-8 w-full rounded-md border bg-background px-2"
                value={formModel}
                onChange={(event) => setFormModel(event.target.value)}
              >
                <option value="">默认模型</option>
                {channels
                  .find((channel) => channel.id === formChannelId)
                  ?.models.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.id}
                    </option>
                  ))}
              </select>
            </label>
          </div>
          <DialogFooter className="m-0 shrink-0 border-t border-border/50 bg-muted/10 px-6 py-4">
            <Button
              variant="outline"
              size="sm"
              className="h-8 text-xs"
              onClick={() => setDialogOpen(false)}
            >
              {t("common:cancel")}
            </Button>
            <Button
              size="sm"
              className="h-8 text-xs"
              disabled={saving || !name.trim() || !cron.trim() || !prompt.trim()}
              onClick={() => void create()}
            >
              {saving ? <Loader2 className="mr-1 size-3.5 animate-spin" /> : null}
              {editing ? t("common:save") : t("settings:automations.create")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
