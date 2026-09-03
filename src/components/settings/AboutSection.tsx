import { useTranslation } from "react-i18next";

import { SettingCard } from "./SettingCard";

export function AboutSection() {
  const { t } = useTranslation("settings");
  return (
    <SettingCard title={t("about.title")} description={t("about.description")} badge="0.1.0">
      <p className="text-sm">{t("about.version")}: 0.1.0</p>
    </SettingCard>
  );
}
