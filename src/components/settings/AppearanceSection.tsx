import { useTranslation } from "react-i18next";

import type { ThemeMode } from "@/lib/theme";
import { useUiStore } from "@/stores/uiStore";
import { SettingCard } from "./SettingCard";

export function AppearanceSection() {
  const { t } = useTranslation("settings");
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  return (
    <SettingCard
      title={t("appearance.theme")}
      description={t("appearance.themeHint")}
      badge={t(`appearance.${theme}`)}
    >
      <select
        className="h-8 rounded-md border px-2 text-sm"
        value={theme}
        onChange={(event) => setTheme(event.target.value as ThemeMode)}
      >
        <option value="system">{t("appearance.system")}</option>
        <option value="light">{t("appearance.light")}</option>
        <option value="dark">{t("appearance.dark")}</option>
      </select>
    </SettingCard>
  );
}
