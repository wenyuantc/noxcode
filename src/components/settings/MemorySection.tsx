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
import { SettingCard } from "./SettingCard";

export function MemorySection() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const [view, setView] = useState<NativeMemoryView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [interval, setInterval] = useState<number>(native?.memory_dream_interval ?? 10);

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
        setError(null);
      })
      .catch((reason: unknown) => setError(String(reason)));
  }, [workspaceId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const remove = (entry: NativeMemoryEntry) => {
    if (!workspaceId) return;
    deleteNativeMemory(workspaceId, entry.file_name)
      .then(() => reload())
      .catch((reason: unknown) => setError(String(reason)));
  };

  const dream = () => {
    if (!workspaceId || !channelId) {
      setError(t("settings:memory.needChannel"));
      return;
    }
    setBusy(true);
    setMessage(null);
    dreamNativeMemory(workspaceId, channelId)
      .then((summary) => {
        setMessage(summary);
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)))
      .finally(() => setBusy(false));
  };

  if (!native) return null;

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("settings:memory.hint")}</p>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      {message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
      <SettingCard
        title={t("settings:memory.settingsTitle")}
        description={t("settings:memory.settingsHint")}
      >
        <div className="space-y-3">
          <label
            htmlFor="memory-enabled"
            className="flex max-w-xs cursor-pointer items-center justify-between gap-3 text-sm"
          >
            <span>{t("settings:memory.enabled")}</span>
            <Switch
              id="memory-enabled"
              checked={native.memory_enabled}
              onCheckedChange={(checked) => {
                void updateNativeSettings({ memory_enabled: checked }).then(setNative);
              }}
            />
          </label>
          <label htmlFor="memory-dream-interval" className="block max-w-xs text-sm">
            <span>{t("settings:memory.dreamInterval")}</span>
            <div className="mt-1 flex gap-2">
              <Input
                id="memory-dream-interval"
                type="number"
                min={0}
                max={1000}
                step={1}
                value={interval}
                onChange={(event) => setInterval(Number(event.target.value))}
              />
              <Button
                variant="outline"
                onClick={() =>
                  void updateNativeSettings({ memory_dream_interval: interval }).then(setNative)
                }
              >
                {t("common:save")}
              </Button>
            </div>
          </label>
        </div>
      </SettingCard>
      <SettingCard
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
      >
        {view ? (
          <div className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={reload}>
                {t("settings:memory.refresh")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => workspaceId && void openNativeMemoryDir(workspaceId)}
              >
                {t("settings:memory.openDir")}
              </Button>
              <Button size="sm" disabled={busy || view.entries.length === 0} onClick={dream}>
                {busy ? t("settings:memory.dreaming") : t("settings:memory.dream")}
              </Button>
            </div>
            {view.entries.length === 0 ? (
              <p className="text-xs text-muted-foreground">{t("settings:memory.empty")}</p>
            ) : (
              <ul className="divide-y rounded-md border">
                {view.entries.map((entry) => (
                  <li key={entry.file_name} className="px-3 py-2 text-sm">
                    <div className="flex items-center gap-3">
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate text-left"
                        onClick={() =>
                          setOpen((current) =>
                            current === entry.file_name ? null : entry.file_name,
                          )
                        }
                      >
                        <span className="font-medium">{entry.name}</span>
                        <span className="ml-2 text-xs text-muted-foreground">
                          {t(`settings:memory.kind.${entry.kind}`, { defaultValue: entry.kind })}
                        </span>
                        {entry.description ? (
                          <span className="ml-2 text-xs text-muted-foreground">
                            {entry.description}
                          </span>
                        ) : null}
                      </button>
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {entry.updated_at}
                      </span>
                      <Button variant="ghost" size="sm" onClick={() => remove(entry)}>
                        {t("common:delete")}
                      </Button>
                    </div>
                    {open === entry.file_name ? (
                      <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-md border bg-muted/30 px-3 py-2 text-xs leading-5">
                        {entry.body}
                      </pre>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
        ) : null}
      </SettingCard>
    </div>
  );
}
