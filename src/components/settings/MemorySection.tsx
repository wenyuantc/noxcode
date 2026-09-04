import {
  Brain,
  ChevronDown,
  ChevronRight,
  FolderOpen,
  Loader2,
  RefreshCw,
  Sparkles,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  deleteNativeMemory,
  dreamNativeMemory,
  listNativeMemories,
  openNativeMemoryDir,
  updateNativeSettings,
} from "@/lib/backend";
import type { NativeMemoryEntry, NativeMemoryView } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard, SettingRow } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

export function MemorySection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const [view, setView] = useState<NativeMemoryView | null>(null);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [interval, setInterval] = useState<number>(native?.memory_dream_interval ?? 10);
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    if (native) setInterval(native.memory_dream_interval);
  }, [native]);

  const reload = useCallback(() => {
    if (!workspaceId) {
      setView(null);
      return;
    }
    listNativeMemories(workspaceId)
      .then((next) => {
        setView(next);
      })
      .catch((reason: unknown) => {
        setFeedback({ variant: "error", message: String(reason) });
      });
  }, [workspaceId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const remove = (entry: NativeMemoryEntry) => {
    if (!workspaceId) return;
    deleteNativeMemory(workspaceId, entry.file_name)
      .then(() => {
        setFeedback({ variant: "success", message: t("common:deleted") ?? "已删除" });
        reload();
      })
      .catch((reason: unknown) => {
        setFeedback({ variant: "error", message: String(reason) });
      });
  };

  const dream = () => {
    if (!workspaceId || !channelId) {
      setFeedback({ variant: "error", message: t("settings:memory.needChannel") });
      return;
    }
    setBusy(true);
    setFeedback(null);
    dreamNativeMemory(workspaceId, channelId)
      .then((summary) => {
        setFeedback({ variant: "success", message: summary });
        reload();
      })
      .catch((reason: unknown) => {
        setFeedback({ variant: "error", message: String(reason) });
      })
      .finally(() => setBusy(false));
  };

  if (!native) return null;

  return (
    <div className="space-y-6">
      {feedback ? (
        <SettingFeedbackCallout
          variant={feedback.variant}
          message={feedback.message}
          onClose={() => setFeedback(null)}
        />
      ) : null}

      {/* 记忆配置卡片 */}
      <SettingCard
        icon={Brain}
        title={t("settings:memory.settingsTitle")}
        description={t("settings:memory.settingsHint")}
        divided
      >
        <SettingRow
          title={t("settings:memory.enabled")}
          description="启用后，Agent 将自动提取与学习对话中的长期记忆。"
        >
          <Switch
            id="memory-enabled"
            checked={native.memory_enabled}
            onCheckedChange={(checked) => {
              void updateNativeSettings({ memory_enabled: checked }).then((res) => {
                setNative(res);
                setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
              });
            }}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:memory.dreamInterval")}
          description="每隔指定会话轮次自动触发一次记忆提炼与深度巩固（Dream）。"
        >
          <div className="flex items-center gap-2">
            <div className="relative w-28">
              <Input
                id="memory-dream-interval"
                className="h-8 pr-7 text-xs font-mono text-right"
                type="number"
                min={0}
                max={1000}
                step={1}
                value={interval}
                onChange={(e) => setInterval(Number(e.target.value))}
              />
              <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground font-mono">
                轮
              </span>
            </div>
            <Button
              size="sm"
              variant="outline"
              className="h-8 text-xs"
              onClick={() =>
                void updateNativeSettings({ memory_dream_interval: interval }).then((res) => {
                  setNative(res);
                  setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
                })
              }
            >
              {t("common:save")}
            </Button>
          </div>
        </SettingRow>
      </SettingCard>

      {/* 记忆实体条目管理 */}
      <SettingCard
        icon={Sparkles}
        title={t("settings:memory.entriesTitle")}
        description={
          view
            ? t("settings:memory.entriesHint", {
                dir: view.dir,
                extractions: view.extractions,
                dreams: view.dreams,
              })
            : t("settings:memory.needWorkspace")
        }
        badge={view ? `${view.entries.length} 条记忆` : undefined}
        headerAction={
          view ? (
            <div className="flex items-center gap-1.5">
              <Button variant="outline" size="sm" className="h-7 text-xs gap-1" onClick={reload}>
                <RefreshCw className="size-3" />
                {t("settings:memory.refresh")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="h-7 text-xs gap-1"
                onClick={() => workspaceId && void openNativeMemoryDir(workspaceId)}
              >
                <FolderOpen className="size-3" />
                {t("settings:memory.openDir")}
              </Button>
              <Button
                size="sm"
                className="h-7 text-xs gap-1"
                disabled={busy || view.entries.length === 0}
                onClick={dream}
              >
                {busy ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : (
                  <Sparkles className="size-3" />
                )}
                {busy ? t("settings:memory.dreaming") : t("settings:memory.dream")}
              </Button>
            </div>
          ) : null
        }
      >
        {view ? (
          view.entries.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-10 text-center">
              <div className="flex size-10 items-center justify-center rounded-xl border border-border/70 bg-muted/30 text-muted-foreground">
                <Brain className="size-5" />
              </div>
              <p className="mt-3 text-xs font-medium text-foreground">
                {t("settings:memory.empty")}
              </p>
              <p className="mt-1 text-[11px] text-muted-foreground">
                Agent 在会话过程中会自动沉淀记忆文件，并展示在此处。
              </p>
            </div>
          ) : (
            <div className="space-y-2">
              {view.entries.map((entry) => {
                const isOpen = open === entry.file_name;
                return (
                  <div
                    key={entry.file_name}
                    className="group rounded-xl border border-border/70 bg-card transition-all hover:border-border hover:shadow-2xs overflow-hidden"
                  >
                    <div className="flex items-center gap-3 p-3 text-xs">
                      <button
                        type="button"
                        className="flex min-w-0 flex-1 items-center gap-2 text-left"
                        onClick={() => setOpen(isOpen ? null : entry.file_name)}
                      >
                        {isOpen ? (
                          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
                        ) : (
                          <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
                        )}
                        <span className="font-medium text-foreground truncate">{entry.name}</span>
                        <span className="inline-flex items-center rounded-md border border-border/60 bg-muted/50 px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground">
                          {t(`settings:memory.kind.${entry.kind}`, { defaultValue: entry.kind })}
                        </span>
                        {entry.description ? (
                          <span className="truncate text-muted-foreground hidden sm:inline">
                            {entry.description}
                          </span>
                        ) : null}
                      </button>
                      <span className="shrink-0 text-[11px] text-muted-foreground font-mono">
                        {entry.updated_at}
                      </span>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        className="text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100"
                        onClick={() => remove(entry)}
                        title={t("common:delete")}
                      >
                        <Trash2 className="size-3.5" />
                      </Button>
                    </div>
                    {isOpen ? (
                      <div className="border-t border-border/50 bg-muted/20 p-3">
                        <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-foreground">
                          {entry.body}
                        </pre>
                      </div>
                    ) : null}
                  </div>
                );
              })}
            </div>
          )
        ) : null}
      </SettingCard>
    </div>
  );
}
