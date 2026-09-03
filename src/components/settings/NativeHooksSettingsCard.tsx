import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { updateNativeSettings } from "@/lib/backend";
import type { NativeHook } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useSettingsStore } from "@/stores/settingsStore";
import { SettingCard } from "./SettingCard";

export function NativeHooksSettingsCard() {
  const { t } = useTranslation(["settings", "common"]);
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const [hooks, setHooks] = useState<NativeHook[]>(native?.hooks ?? []);

  useEffect(() => {
    if (native) setHooks(native.hooks);
  }, [native]);

  return (
    <SettingCard title={t("settings:hooks.title")} description={t("settings:hooks.hint")}>
      <div className="space-y-3">
        {hooks.map((hook, index) => (
          <div key={hook.id} className="grid grid-cols-2 gap-2 rounded-md border p-3">
            <Input
              value={hook.event}
              onChange={(e) => {
                const next = [...hooks];
                next[index] = { ...hook, event: e.target.value };
                setHooks(next);
              }}
            />
            <Input
              value={hook.matcher}
              onChange={(e) => {
                const next = [...hooks];
                next[index] = { ...hook, matcher: e.target.value };
                setHooks(next);
              }}
            />
            <Input
              className="col-span-2"
              value={hook.command}
              onChange={(e) => {
                const next = [...hooks];
                next[index] = { ...hook, command: e.target.value };
                setHooks(next);
              }}
            />
          </div>
        ))}
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() =>
              setHooks([
                ...hooks,
                {
                  id: crypto.randomUUID(),
                  event: "PreToolUse",
                  matcher: "*",
                  command: "",
                  timeout_secs: 30,
                  enabled: true,
                },
              ])
            }
          >
            {t("common:create")}
          </Button>
          <Button onClick={() => void updateNativeSettings({ hooks }).then(setNative)}>
            {t("common:save")}
          </Button>
        </div>
      </div>
    </SettingCard>
  );
}
