export type AppLocale = "zh-CN" | "en";

export const DEFAULT_LOCALE: AppLocale = "zh-CN";
export const SUPPORTED_LOCALES: AppLocale[] = ["zh-CN", "en"];

const LOCALE_STORAGE_KEY = "noxcode:locale";

export const LOCALE_CHANGE_EVENT = "noxcode:locale-change";

export function isAppLocale(value: string | null | undefined): value is AppLocale {
  return value === "zh-CN" || value === "en";
}

export function getLocalePreference(): AppLocale {
  if (typeof window === "undefined") return DEFAULT_LOCALE;
  const stored = window.localStorage.getItem(LOCALE_STORAGE_KEY);
  return isAppLocale(stored) ? stored : DEFAULT_LOCALE;
}

export function persistLocalePreference(locale: AppLocale) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  window.dispatchEvent(
    new CustomEvent(LOCALE_CHANGE_EVENT, {
      detail: { locale },
    }),
  );
}

export function getCurrentAppLocale(): AppLocale {
  return getLocalePreference();
}

export function getDateLocale(locale: AppLocale = getLocalePreference()): string {
  return locale === "en" ? "en-US" : "zh-CN";
}
