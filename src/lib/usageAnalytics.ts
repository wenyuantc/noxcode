import { emptyApiCallLogStats } from "@/lib/apiLogs";
import type {
  NativeUsageAnalytics,
  NativeUsageDailyBucket,
  NativeUsageModelBucket,
} from "@/lib/types";

export const USAGE_CUSTOM_RANGE_MAX_DAYS = 366;
export const USAGE_MODEL_DISPLAY_LIMIT = 8;
export const USAGE_OTHER_MODEL_ID = "__other__";

export type UsageRangePreset = "7d" | "30d" | "custom";
export type UsageHeatmapLevel = 0 | 1 | 2 | 3 | 4;

export type UsageDateRangeResult =
  { ok: true; start: string; end: string } | { ok: false; reason: "order" | "span" | "incomplete" };

export interface UsageHeatmapCell {
  date: string;
  inRange: boolean;
  calls: number;
  totalTokens: number;
  level: UsageHeatmapLevel;
}

export interface UsageHeatmapWeek {
  cells: UsageHeatmapCell[];
}

const MS_PER_DAY = 86_400_000;

export function emptyUsageDailyBucket(date: string): NativeUsageDailyBucket {
  return {
    date,
    calls: 0,
    success: 0,
    failed: 0,
    cancelled: 0,
    input_tokens: 0,
    output_tokens: 0,
    cached_tokens: 0,
    total_tokens: 0,
  };
}

export function emptyUsageAnalytics(): NativeUsageAnalytics {
  return {
    stats: emptyApiCallLogStats(),
    daily: [],
    models: [],
  };
}

export function utcDateKey(date: Date): string {
  return date.toISOString().slice(0, 10);
}

export function parseUtcDateKey(date: string): Date | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    return null;
  }
  const parsed = new Date(`${date}T00:00:00.000Z`);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

export function shiftUtcDateKey(date: string, days: number): string {
  const parsed = parseUtcDateKey(date);
  if (!parsed) {
    return date;
  }
  parsed.setUTCDate(parsed.getUTCDate() + days);
  return utcDateKey(parsed);
}

export function inclusiveDayCount(start: string, end: string): number {
  const startDate = parseUtcDateKey(start);
  const endDate = parseUtcDateKey(end);
  if (!startDate || !endDate) {
    return 0;
  }
  return Math.floor((endDate.getTime() - startDate.getTime()) / MS_PER_DAY) + 1;
}

export function resolveUsageDateRange(
  preset: UsageRangePreset,
  customStart: string,
  customEnd: string,
  now = new Date(),
): UsageDateRangeResult {
  const today = utcDateKey(now);
  if (preset === "7d") {
    return { ok: true, start: shiftUtcDateKey(today, -6), end: today };
  }
  if (preset === "30d") {
    return { ok: true, start: shiftUtcDateKey(today, -29), end: today };
  }

  const start = customStart.trim();
  const end = customEnd.trim();
  if (!start || !end) {
    return { ok: false, reason: "incomplete" };
  }
  if (!parseUtcDateKey(start) || !parseUtcDateKey(end)) {
    return { ok: false, reason: "incomplete" };
  }
  if (start > end) {
    return { ok: false, reason: "order" };
  }
  if (inclusiveDayCount(start, end) > USAGE_CUSTOM_RANGE_MAX_DAYS) {
    return { ok: false, reason: "span" };
  }
  return { ok: true, start, end };
}

export function fillUsageDailyBuckets(
  start: string,
  end: string,
  daily: NativeUsageDailyBucket[],
): NativeUsageDailyBucket[] {
  if (!parseUtcDateKey(start) || !parseUtcDateKey(end) || start > end) {
    return [];
  }
  const byDate = new Map(daily.map((item) => [item.date, item]));
  const filled: NativeUsageDailyBucket[] = [];
  let cursor = start;
  while (cursor <= end) {
    filled.push(byDate.get(cursor) ?? emptyUsageDailyBucket(cursor));
    cursor = shiftUtcDateKey(cursor, 1);
  }
  return filled;
}

function compactNumber(value: number, unit: string): string {
  const text = value >= 100 ? Math.round(value).toString() : value.toFixed(1).replace(/\.0$/, "");
  return `${text}${unit}`;
}

export function formatUsageTokenCount(value: number): string {
  if (!Number.isFinite(value)) {
    return "0";
  }
  const count = Math.max(0, Math.round(value));
  if (count >= 1_000_000) {
    return compactNumber(count / 1_000_000, "M");
  }
  if (count >= 1_000) {
    return compactNumber(count / 1_000, "k");
  }
  return String(count);
}

export function formatUsageTokenExact(value: number): string {
  const count = Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0;
  return new Intl.NumberFormat("en-US").format(count);
}

export function usageTotalTokens(stats: {
  input_tokens: number;
  output_tokens: number;
  total_tokens_sum: number | null;
}): number {
  if (stats.total_tokens_sum != null && Number.isFinite(stats.total_tokens_sum)) {
    return Math.max(0, stats.total_tokens_sum);
  }
  return Math.max(0, stats.input_tokens) + Math.max(0, stats.output_tokens);
}

export function usageCacheHitRatio(
  inputTokens: number | null | undefined,
  cachedTokens: number | null | undefined,
): number | null {
  if (
    inputTokens === null ||
    inputTokens === undefined ||
    cachedTokens === null ||
    cachedTokens === undefined ||
    !Number.isFinite(inputTokens) ||
    !Number.isFinite(cachedTokens)
  ) {
    return null;
  }

  const cached = Math.max(0, cachedTokens);
  const prompt = Math.max(0, inputTokens);
  const denominator = cached > prompt ? prompt + cached : prompt;
  if (denominator <= 0) {
    return null;
  }
  return cached / denominator;
}

export function heatmapLevel(tokens: number, maxTokens: number): UsageHeatmapLevel {
  const count = Number.isFinite(tokens) ? Math.max(0, tokens) : 0;
  const max = Number.isFinite(maxTokens) ? Math.max(0, maxTokens) : 0;
  if (count <= 0 || max <= 0) {
    return 0;
  }
  const ratio = count / max;
  if (ratio <= 0.25) {
    return 1;
  }
  if (ratio <= 0.5) {
    return 2;
  }
  if (ratio <= 0.75) {
    return 3;
  }
  return 4;
}

function mondayOf(date: string): string {
  const parsed = parseUtcDateKey(date);
  if (!parsed) {
    return date;
  }
  const weekday = parsed.getUTCDay();
  const offset = weekday === 0 ? -6 : 1 - weekday;
  return shiftUtcDateKey(date, offset);
}

function sundayOf(date: string): string {
  return shiftUtcDateKey(mondayOf(date), 6);
}

export function buildHeatmapWeeks(
  days: NativeUsageDailyBucket[],
  rangeStart: string,
  rangeEnd: string,
): UsageHeatmapWeek[] {
  if (!parseUtcDateKey(rangeStart) || !parseUtcDateKey(rangeEnd) || rangeStart > rangeEnd) {
    return [];
  }

  const byDate = new Map(days.map((item) => [item.date, item]));
  const maxTokens = Math.max(0, ...days.map((item) => item.total_tokens));
  const start = mondayOf(rangeStart);
  const end = sundayOf(rangeEnd);
  const weeks: UsageHeatmapWeek[] = [];
  let cursor = start;

  while (cursor <= end) {
    const cells: UsageHeatmapCell[] = [];
    for (let index = 0; index < 7; index += 1) {
      const date = shiftUtcDateKey(cursor, index);
      const inRange = date >= rangeStart && date <= rangeEnd;
      const bucket = byDate.get(date);
      const totalTokens = bucket?.total_tokens ?? 0;
      cells.push({
        date,
        inRange,
        calls: bucket?.calls ?? 0,
        totalTokens,
        level: inRange ? heatmapLevel(totalTokens, maxTokens) : 0,
      });
    }
    weeks.push({ cells });
    cursor = shiftUtcDateKey(cursor, 7);
  }

  return weeks;
}

export function mergeUsageModels(
  models: NativeUsageModelBucket[],
  limit = USAGE_MODEL_DISPLAY_LIMIT,
): NativeUsageModelBucket[] {
  if (models.length <= limit) {
    return models;
  }

  const head = models.slice(0, limit);
  const rest = models.slice(limit);
  const other = rest.reduce(
    (acc, item) => ({
      model: USAGE_OTHER_MODEL_ID,
      calls: acc.calls + item.calls,
      input_tokens: acc.input_tokens + item.input_tokens,
      output_tokens: acc.output_tokens + item.output_tokens,
      cached_tokens: acc.cached_tokens + item.cached_tokens,
      total_tokens: acc.total_tokens + item.total_tokens,
    }),
    emptyUsageModelBucket(USAGE_OTHER_MODEL_ID),
  );

  return [...head, other];
}

export function emptyUsageModelBucket(model: string): NativeUsageModelBucket {
  return {
    model,
    calls: 0,
    input_tokens: 0,
    output_tokens: 0,
    cached_tokens: 0,
    total_tokens: 0,
  };
}

export function displayUsageModelName(
  model: string,
  unknownLabel: string,
  otherLabel: string,
): string {
  if (model === USAGE_OTHER_MODEL_ID) {
    return otherLabel;
  }
  const trimmed = model.trim();
  return trimmed || unknownLabel;
}

export function usageTrendLabelIndexes(length: number): number[] {
  if (length <= 0) {
    return [];
  }
  if (length <= 8) {
    return Array.from({ length }, (_, index) => index);
  }
  const step = Math.ceil(length / 7);
  const indexes: number[] = [];
  for (let index = 0; index < length; index += step) {
    indexes.push(index);
  }
  if (indexes[indexes.length - 1] !== length - 1) {
    indexes.push(length - 1);
  }
  return indexes;
}

export function formatUsageDayLabel(date: string): string {
  const parsed = parseUtcDateKey(date);
  if (!parsed) {
    return date;
  }
  return `${parsed.getUTCMonth() + 1}/${parsed.getUTCDate()}`;
}
