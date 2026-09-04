import { open } from "@tauri-apps/plugin-dialog";
import { Bell, FileUp, Globe, Network, Plus, Save, Sparkles, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { changeAppLocale, getCurrentAppLocale } from "@/lib/i18n";
import { updateNativeSettings, updateNetworkSettings, updateQuickPrompts } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard, SettingRow } from "./SettingCard";
import { SettingFeedbackCallout } from "./SettingFeedbackCallout";

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
  const [feedback, setFeedback] = useState<{
    variant: "success" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    setProxy(network?.http_proxy ?? "");
    setNoProxy(network?.no_proxy ?? "");
    setCa(network?.ca_cert_path ?? "");
  }, [network]);

  useEffect(() => {
    setDraftPrompts(prompts);
  }, [prompts]);

  const saveNetwork = async () => {
    try {
      const updated = await updateNetworkSettings({
        http_proxy: proxy.trim() || null,
        no_proxy: noProxy.trim() || null,
        ca_cert_path: ca.trim() || null,
      });
      setNetwork(updated);
      setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
    } catch (err) {
      setFeedback({ variant: "error", message: String(err) });
    }
  };

  const savePrompts = async () => {
    try {
      const updated = await updateQuickPrompts(draftPrompts);
      setQuickPrompts(updated);
      setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
    } catch (err) {
      setFeedback({ variant: "error", message: String(err) });
    }
  };

  const addPrompt = () => {
    setDraftPrompts([
      ...draftPrompts,
      {
        id: `prompt-${Date.now()}`,
        label: "新提示词",
        prompt: "",
      },
    ]);
  };

  const removePrompt = (index: number) => {
    setDraftPrompts(draftPrompts.filter((_, i) => i !== index));
  };

  return (
    <div className="space-y-6">
      {feedback ? (
        <SettingFeedbackCallout
          variant={feedback.variant}
          message={feedback.message}
          onClose={() => setFeedback(null)}
        />
      ) : null}

      {/* 偏好与通知 */}
      <SettingCard
        title={t("settings:sections.general")}
        description={t("settings:general.languageHint")}
        divided
      >
        {/* 界面语言 */}
        <SettingRow
          icon={Globe}
          title={t("settings:general.language")}
          description={t("settings:general.languageHint")}
        >
          <div className="flex items-center rounded-lg border border-border/80 bg-muted/40 p-0.5">
            <button
              type="button"
              onClick={() => void changeAppLocale("zh-CN")}
              className={`rounded-md px-3 py-1 text-xs font-medium transition-all ${
                locale === "zh-CN"
                  ? "bg-background text-foreground shadow-2xs"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              中文简体
            </button>
            <button
              type="button"
              onClick={() => void changeAppLocale("en")}
              className={`rounded-md px-3 py-1 text-xs font-medium transition-all ${
                locale === "en"
                  ? "bg-background text-foreground shadow-2xs"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              English
            </button>
          </div>
        </SettingRow>

        {/* 桌面通知 */}
        {native ? (
          <SettingRow
            icon={Bell}
            title={t("settings:general.desktopNotifications")}
            description={t("settings:general.desktopNotificationsHint")}
          >
            <Switch
              checked={native.desktop_notifications}
              onCheckedChange={(desktop_notifications) => {
                void updateNativeSettings({ desktop_notifications })
                  .then((res) => {
                    setNative(res);
                    setFeedback({ variant: "success", message: t("common:saved") ?? "保存成功" });
                  })
                  .catch((err: unknown) => {
                    setFeedback({ variant: "error", message: String(err) });
                  });
              }}
            />
          </SettingRow>
        ) : null}
      </SettingCard>

      {/* 网络与代理 */}
      <SettingCard
        icon={Network}
        title={t("settings:general.proxy")}
        description={t("settings:general.proxyHint")}
        headerAction={
          <Button size="sm" onClick={() => void saveNetwork()} className="h-7 gap-1 text-xs">
            <Save className="size-3.5" />
            {t("common:save")}
          </Button>
        }
        divided
      >
        <SettingRow
          title={t("settings:general.proxy")}
          description={t("settings:general.proxyHint")}
        >
          <Input
            className="h-8 max-w-xs text-xs font-mono"
            placeholder="http://127.0.0.1:7890"
            value={proxy}
            onChange={(e) => setProxy(e.target.value)}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:general.noProxy")}
          description={t("settings:general.noProxyHint")}
        >
          <Input
            className="h-8 max-w-xs text-xs font-mono"
            placeholder="localhost, 127.0.0.1, .internal"
            value={noProxy}
            onChange={(e) => setNoProxy(e.target.value)}
          />
        </SettingRow>

        <SettingRow
          title={t("settings:general.caCert")}
          description={t("settings:general.caCertHint")}
        >
          <div className="flex w-full max-w-xs items-center gap-1.5">
            <Input
              className="h-8 text-xs font-mono"
              placeholder="/path/to/ca.pem"
              value={ca}
              onChange={(e) => setCa(e.target.value)}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 px-2.5"
              onClick={() => {
                void open({ multiple: false }).then((path) => {
                  if (typeof path === "string") setCa(path);
                });
              }}
            >
              <FileUp className="size-3.5" />
            </Button>
          </div>
        </SettingRow>
      </SettingCard>

      {/* 首页快捷提示词 */}
      <SettingCard
        icon={Sparkles}
        title={t("settings:general.quickPrompts")}
        description={t("settings:general.quickPromptsHint")}
        headerAction={
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={addPrompt}
              className="h-7 gap-1 text-xs"
            >
              <Plus className="size-3.5" />
              {t("channels.actions.addModel", { defaultValue: "添加" })}
            </Button>
            <Button size="sm" onClick={() => void savePrompts()} className="h-7 gap-1 text-xs">
              <Save className="size-3.5" />
              {t("common:save")}
            </Button>
          </div>
        }
      >
        <div className="space-y-3">
          {draftPrompts.map((prompt, index) => (
            <div
              key={prompt.id}
              className="group relative rounded-xl border border-border/70 bg-card p-3 shadow-2xs transition-all hover:border-border"
            >
              <div className="mb-2 flex items-center justify-between gap-2">
                <Input
                  className="h-7 max-w-xs text-xs font-medium"
                  placeholder="标签文字（如：摸底仓库）"
                  value={prompt.label}
                  onChange={(event) => {
                    const next = [...draftPrompts];
                    next[index] = { ...prompt, label: event.target.value };
                    setDraftPrompts(next);
                  }}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="text-muted-foreground opacity-60 hover:text-destructive hover:opacity-100"
                  onClick={() => removePrompt(index)}
                  title={t("common:delete")}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
              <Textarea
                className="min-h-16 resize-none text-xs leading-relaxed"
                placeholder="提示词内容模版…"
                value={prompt.prompt}
                onChange={(event) => {
                  const next = [...draftPrompts];
                  next[index] = { ...prompt, prompt: event.target.value };
                  setDraftPrompts(next);
                }}
              />
            </div>
          ))}
          {draftPrompts.length === 0 ? (
            <p className="py-4 text-center text-xs text-muted-foreground">
              暂无快捷提示词，点击右上角添加
            </p>
          ) : null}
        </div>
      </SettingCard>
    </div>
  );
}
