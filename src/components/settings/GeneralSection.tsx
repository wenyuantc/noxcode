import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { changeAppLocale, getCurrentAppLocale } from "@/lib/i18n";
import type { AppLocale } from "@/lib/i18n/locale";
import { updateNativeSettings, updateNetworkSettings, updateQuickPrompts } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

export function GeneralSection() {
  const { t } = useTranslation(["settings", "common"]);
  const locale = getCurrentAppLocale();
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const network = useSettingsStore((state) => state.network);
  const setNetwork = useSettingsStore((state) => state.setNetwork);
  const prompts = useSettingsStore((state) => state.quickPrompts);
  const setQuickPrompts = useSettingsStore((state) => state.setQuickPrompts);
  const [proxy, setProxy] = useState(network?.http_proxy ?? "");
  const [noProxy, setNoProxy] = useState(network?.no_proxy ?? "");
  const [ca, setCa] = useState(network?.ca_cert_path ?? "");
  const [draftPrompts, setDraftPrompts] = useState(prompts);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setProxy(network?.http_proxy ?? "");
    setNoProxy(network?.no_proxy ?? "");
    setCa(network?.ca_cert_path ?? "");
  }, [network]);

  useEffect(() => {
    setDraftPrompts(prompts);
  }, [prompts]);

  return (
    <div className="space-y-4">
      <SettingCard
        title={t("settings:general.language")}
        description={t("settings:general.languageHint")}
        badge={locale}
      >
        <select
          className="h-8 rounded-md border px-2 text-sm"
          value={locale}
          onChange={(event) => void changeAppLocale(event.target.value as AppLocale)}
        >
          <option value="zh-CN">中文简体</option>
          <option value="en">English</option>
        </select>
      </SettingCard>
      {native ? (
        <SettingCard
          title={t("settings:general.desktopNotifications")}
          description={t("settings:general.desktopNotificationsHint")}
        >
          <Switch
            checked={native.desktop_notifications}
            onCheckedChange={(desktop_notifications) => {
              void updateNativeSettings({ desktop_notifications })
                .then(setNative)
                .catch((err: unknown) => setError(String(err)));
            }}
          />
        </SettingCard>
      ) : null}
      <SettingCard
        title={t("settings:general.proxy")}
        description={t("settings:general.proxyHint")}
      >
        <div className="flex gap-2">
          <Input value={proxy} onChange={(event) => setProxy(event.target.value)} />
          <Button
            onClick={() => {
              void updateNetworkSettings({
                http_proxy: proxy || null,
                no_proxy: noProxy || null,
                ca_cert_path: ca || null,
              })
                .then(setNetwork)
                .catch((err: unknown) => setError(String(err)));
            }}
          >
            {t("common:save")}
          </Button>
        </div>
      </SettingCard>
      <SettingCard
        title={t("settings:general.noProxy")}
        description={t("settings:general.noProxyHint")}
      >
        <Input value={noProxy} onChange={(event) => setNoProxy(event.target.value)} />
      </SettingCard>
      <SettingCard
        title={t("settings:general.caCert")}
        description={t("settings:general.caCertHint")}
      >
        <div className="flex gap-2">
          <Input value={ca} onChange={(event) => setCa(event.target.value)} />
          <Button
            variant="outline"
            onClick={() => {
              void open({ multiple: false }).then((path) => {
                if (typeof path === "string") setCa(path);
              });
            }}
          >
            {t("common:open")}
          </Button>
        </div>
      </SettingCard>
      <SettingCard
        title={t("settings:general.quickPrompts")}
        description={t("settings:general.quickPromptsHint")}
      >
        <div className="space-y-3">
          {draftPrompts.map((prompt, index) => (
            <div key={prompt.id} className="space-y-1">
              <Input
                value={prompt.label}
                onChange={(event) => {
                  const next = [...draftPrompts];
                  next[index] = { ...prompt, label: event.target.value };
                  setDraftPrompts(next);
                }}
              />
              <Textarea
                value={prompt.prompt}
                onChange={(event) => {
                  const next = [...draftPrompts];
                  next[index] = { ...prompt, prompt: event.target.value };
                  setDraftPrompts(next);
                }}
              />
            </div>
          ))}
          <Button
            onClick={() => {
              void updateQuickPrompts(draftPrompts)
                .then(setQuickPrompts)
                .catch((err: unknown) => setError(String(err)));
            }}
          >
            {t("common:save")}
          </Button>
        </div>
      </SettingCard>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
    </div>
  );
}
