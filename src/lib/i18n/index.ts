import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import {
  DEFAULT_LOCALE,
  getLocalePreference,
  persistLocalePreference,
  type AppLocale,
} from "./locale";

import commonZh from "@/locales/zh-CN/common.json";
import navZh from "@/locales/zh-CN/nav.json";
import layoutZh from "@/locales/zh-CN/layout.json";
import sessionsZh from "@/locales/zh-CN/sessions.json";
import settingsZh from "@/locales/zh-CN/settings.json";
import sshZh from "@/locales/zh-CN/ssh.json";
import gitZh from "@/locales/zh-CN/git.json";
import apiLogsZh from "@/locales/zh-CN/apiLogs.json";
import errorsZh from "@/locales/zh-CN/errors.json";

import commonEn from "@/locales/en/common.json";
import navEn from "@/locales/en/nav.json";
import layoutEn from "@/locales/en/layout.json";
import sessionsEn from "@/locales/en/sessions.json";
import settingsEn from "@/locales/en/settings.json";
import sshEn from "@/locales/en/ssh.json";
import gitEn from "@/locales/en/git.json";
import apiLogsEn from "@/locales/en/apiLogs.json";
import errorsEn from "@/locales/en/errors.json";

export const I18N_NAMESPACES = [
  "common",
  "nav",
  "layout",
  "sessions",
  "settings",
  "ssh",
  "git",
  "apiLogs",
  "errors",
] as const;

const resources = {
  "zh-CN": {
    common: commonZh,
    nav: navZh,
    layout: layoutZh,
    sessions: sessionsZh,
    settings: settingsZh,
    ssh: sshZh,
    git: gitZh,
    apiLogs: apiLogsZh,
    errors: errorsZh,
  },
  en: {
    common: commonEn,
    nav: navEn,
    layout: layoutEn,
    sessions: sessionsEn,
    settings: settingsEn,
    ssh: sshEn,
    git: gitEn,
    apiLogs: apiLogsEn,
    errors: errorsEn,
  },
};

const initialLocale = getLocalePreference();

if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources,
    lng: initialLocale,
    fallbackLng: DEFAULT_LOCALE,
    defaultNS: "common",
    ns: [...I18N_NAMESPACES],
    interpolation: { escapeValue: false },
    returnNull: false,
  });
}

async function syncWindowTitle(locale: AppLocale): Promise<void> {
  const title = i18n.t("common:appName", { lng: locale });
  if (typeof document !== "undefined") {
    document.title = title;
  }
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTitle(title);
  } catch {
    // browser preview
  }
}

export async function changeAppLocale(locale: AppLocale): Promise<AppLocale> {
  persistLocalePreference(locale);
  await i18n.changeLanguage(locale);
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale === "en" ? "en" : "zh-CN";
  }
  await syncWindowTitle(locale);
  return locale;
}

export function getCurrentAppLocale(): AppLocale {
  const lng = i18n.resolvedLanguage ?? i18n.language;
  return lng === "en" ? "en" : "zh-CN";
}

if (typeof document !== "undefined") {
  document.documentElement.lang = initialLocale === "en" ? "en" : "zh-CN";
  void syncWindowTitle(initialLocale);
}

export { i18n };
export default i18n;
