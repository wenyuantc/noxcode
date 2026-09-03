import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  createNativeAutomation,
  deleteNativeAutomation,
  listNativeAutomations,
  runNativeAutomationNow,
  updateNativeAutomation,
} from "@/lib/backend";
import type { NativeAutomation } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useChannelStore } from "@/stores/channelStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SettingCard } from "./SettingCard";

export function AutomationsSection() {
  const { t } = useTranslation(["settings", "common"]);
  const workspaceId = useWorkspaceStore((state) => state.activeWorkspaceId);
  const channelId = useChannelStore((state) => state.activeChannelId);
  const model = useChannelStore((state) => state.activeModelId);
  const [items, setItems] = useState<NativeAutomation[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
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

  const create = () => {
    if (!workspaceId) return;
    createNativeAutomation({
      workspace_id: workspaceId,
      name,
      cron,
      prompt,
      channel_id: channelId ?? null,
      model: model ?? null,
      enabled: true,
    })
      .then(() => {
        setName("");
        setPrompt("");
        setMessage(null);
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)));
  };

  const toggle = (item: NativeAutomation, enabled: boolean) => {
    updateNativeAutomation(item.id, { enabled })
      .then(() => reload())
      .catch((reason: unknown) => setError(String(reason)));
  };

  const remove = (item: NativeAutomation) => {
    deleteNativeAutomation(item.id)
      .then(() => reload())
      .catch((reason: unknown) => setError(String(reason)));
  };

  const runNow = (item: NativeAutomation) => {
    runNativeAutomationNow(item.id)
      .then((sessionId) => {
        setMessage(t("settings:automations.started", { session: sessionId }));
        reload();
      })
      .catch((reason: unknown) => setError(String(reason)));
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("settings:automations.hint")}</p>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
      {message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
      <SettingCard
        title={t("settings:automations.createTitle")}
        description={
          workspaceId
            ? t("settings:automations.createHint")
            : t("settings:automations.needWorkspace")
        }
      >
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="block text-sm">
            <span>{t("settings:automations.name")}</span>
            <Input
              className="mt-1"
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={!workspaceId}
            />
          </label>
          <label className="block text-sm">
            <span>{t("settings:automations.cron")}</span>
            <Input
              className="mt-1 font-mono"
              value={cron}
              onChange={(event) => setCron(event.target.value)}
              disabled={!workspaceId}
            />
          </label>
          <label className="block text-sm sm:col-span-2">
            <span>{t("settings:automations.prompt")}</span>
            <Textarea
              className="mt-1"
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              disabled={!workspaceId}
            />
          </label>
        </div>
        <Button
          className="mt-3"
          disabled={!workspaceId || !name.trim() || !cron.trim() || !prompt.trim()}
          onClick={create}
        >
          {t("settings:automations.create")}
        </Button>
      </SettingCard>
      <SettingCard
        title={t("settings:automations.listTitle")}
        description={t("settings:automations.listHint")}
      >
        {items.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("settings:automations.empty")}</p>
        ) : (
          <ul className="divide-y rounded-md border">
            {items.map((item) => (
              <li key={item.id} className="space-y-1 px-3 py-2 text-sm">
                <div className="flex items-center gap-3">
                  <span className="min-w-0 flex-1 truncate font-medium">{item.name}</span>
                  <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                    {item.cron}
                  </code>
                  <Switch
                    checked={item.enabled !== 0}
                    onCheckedChange={(checked) => toggle(item, checked)}
                  />
                  <Button variant="outline" size="sm" onClick={() => runNow(item)}>
                    {t("settings:automations.runNow")}
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => remove(item)}>
                    {t("common:delete")}
                  </Button>
                </div>
                <p className="line-clamp-2 text-xs text-muted-foreground">{item.prompt}</p>
                <p className="text-xs text-muted-foreground">
                  {t("settings:automations.next", { time: item.next_run_at ?? "-" })} ·{" "}
                  {t("settings:automations.last", { time: item.last_run_at ?? "-" })}
                  {item.last_error ? ` · ${item.last_error}` : ""}
                </p>
              </li>
            ))}
          </ul>
        )}
      </SettingCard>
    </div>
  );
}
