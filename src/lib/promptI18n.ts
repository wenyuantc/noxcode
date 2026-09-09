import sessionsEn from "@/locales/en/sessions.json";
import sessionsZh from "@/locales/zh-CN/sessions.json";
import { getLocalePreference } from "@/lib/i18n/locale";

const resources = {
  en: sessionsEn,
  "zh-CN": sessionsZh,
} as const;

function lookup(key: string): unknown {
  return key.split(".").reduce<unknown>((value, part) => {
    if (!value || typeof value !== "object") return undefined;
    return (value as Record<string, unknown>)[part];
  }, resources[getLocalePreference()].prompts);
}

export function promptT(key: string, values?: Record<string, string>): string {
  const template = lookup(key);
  if (typeof template !== "string") return key;
  return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => values?.[name] ?? "");
}
