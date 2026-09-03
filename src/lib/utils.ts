import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

import { getDateLocale, getLocalePreference } from "@/lib/i18n/locale";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function parseDateValue(dateStr: string): Date | null {
  const trimmed = dateStr.trim();
  if (!trimmed) return null;
  const normalized = trimmed.includes("T") ? trimmed : trimmed.replace(" ", "T");
  const withTimezone = /(?:Z|[+-]\d{2}:\d{2})$/i.test(normalized) ? normalized : `${normalized}Z`;
  const parsed = new Date(withTimezone);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

export function formatDate(dateStr: string): string {
  const parsed = parseDateValue(dateStr);
  return parsed ? parsed.toLocaleString(getDateLocale(getLocalePreference())) : dateStr;
}

export function formatRelativeTime(dateStr: string, locale = "zh-CN"): string {
  const parsed = parseDateValue(dateStr);
  if (!parsed) return dateStr;
  const diffMs = Date.now() - parsed.getTime();
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (Math.abs(diffMs) < hour) {
    return rtf.format(-Math.round(diffMs / minute), "minute");
  }
  if (Math.abs(diffMs) < day) {
    return rtf.format(-Math.round(diffMs / hour), "hour");
  }
  if (Math.abs(diffMs) < 30 * day) {
    return rtf.format(-Math.round(diffMs / day), "day");
  }
  return parsed.toLocaleDateString(locale, { month: "short", day: "numeric" });
}

export function formatTokenCount(value: number): string {
  return new Intl.NumberFormat("en-US").format(Math.max(0, Math.round(value)));
}

function compactNumber(value: number, unit: string): string {
  const text = value >= 100 ? Math.round(value).toString() : value.toFixed(1).replace(/\.0$/, "");
  return `${text}${unit}`;
}

export function formatCompactTokens(value: number, locale = getCurrentAppLocale()): string {
  const count = Math.max(0, Math.round(value));
  if (locale.startsWith("zh")) {
    return count >= 10_000 ? compactNumber(count / 10_000, "万") : String(count);
  }
  if (count >= 1_000_000) return compactNumber(count / 1_000_000, "M");
  if (count >= 1_000) return compactNumber(count / 1_000, "K");
  return String(count);
}

export function greetingPeriod(now = new Date()): "morning" | "afternoon" | "evening" {
  const hour = now.getHours();
  if (hour < 12) return "morning";
  if (hour < 18) return "afternoon";
  return "evening";
}
