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

function compactChineseNumber(value: number, unit: string): string {
  const text = value.toFixed(1).replace(/\.0$/, "");
  return `${text}${unit}`;
}

export function formatUsageTokenCompact(value: number, locale = "zh-CN"): string {
  if (!Number.isFinite(value)) {
    return "0";
  }
  const count = Math.max(0, Math.round(value));
  if (locale.startsWith("zh")) {
    if (count >= 100_000_000) {
      return compactChineseNumber(count / 100_000_000, "亿");
    }
    if (count >= 10_000) {
      return compactChineseNumber(count / 10_000, "万");
    }
    return String(count);
  }
  if (count >= 1_000_000_000) {
    return compactNumber(count / 1_000_000_000, "B");
  }
  if (count >= 1_000_000) {
    return compactNumber(count / 1_000_000, "M");
  }
  if (count >= 1_000) {
    return compactNumber(count / 1_000, "k");
  }
  return String(count);
}

export function formatUsageDateWithWeekday(date: string, locale = "zh-CN"): string {
  const parsed = parseUtcDateKey(date);
  if (!parsed) {
    return date;
  }
  const year = parsed.getUTCFullYear();
  const month = parsed.getUTCMonth() + 1;
  const day = parsed.getUTCDate();
  const weekday = parsed.getUTCDay();

  if (locale.startsWith("zh")) {
    const weekdaysZh = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
    return `${year}年${month}月${day}日 ${weekdaysZh[weekday]}`;
  }

  const monthsEn = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  const weekdaysEn = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  return `${monthsEn[month - 1]} ${day}, ${year}, ${weekdaysEn[weekday]}`;
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

export const USAGE_MODEL_PALETTE = [
  "#3b82f6", // Sky/Blue
  "#10b981", // Emerald
  "#8b5cf6", // Violet
  "#f43f5e", // Rose
  "#f97316", // Orange
  "#eab308", // Yellow
  "#06b6d4", // Cyan
  "#6366f1", // Indigo
  "#64748b", // Slate (for other / overflow)
];

export function getUsageModelColor(index: number, isOther = false): string {
  if (isOther) {
    return "#64748b";
  }
  const mainPalette = USAGE_MODEL_PALETTE.slice(0, USAGE_MODEL_PALETTE.length - 1);
  return mainPalette[index % mainPalette.length];
}

export interface UsageDonutSlice {
  model: string;
  name: string;
  tokens: number;
  calls: number;
  percentage: number;
  color: string;
  path: string;
}

export function buildUsageDonutSlices(
  models: NativeUsageModelBucket[],
  unknownLabel: string,
  otherLabel: string,
  cx = 80,
  cy = 80,
  rOuter = 68,
  rInner = 46,
): { slices: UsageDonutSlice[]; allTokens: number } {
  const rows = mergeUsageModels(models);
  const allTokens = rows.reduce((sum, item) => sum + item.total_tokens, 0);

  if (allTokens <= 0) {
    return { slices: [], allTokens: 0 };
  }

  const activeRows = rows.filter((item) => item.total_tokens > 0);

  if (activeRows.length === 1) {
    const single = activeRows[0];
    const isOther = single.model === USAGE_OTHER_MODEL_ID;
    const name = displayUsageModelName(single.model, unknownLabel, otherLabel);
    const color = getUsageModelColor(0, isOther);
    const fullRingPath = [
      `M ${cx} ${cy - rOuter}`,
      `A ${rOuter} ${rOuter} 0 1 0 ${cx} ${cy + rOuter}`,
      `A ${rOuter} ${rOuter} 0 1 0 ${cx} ${cy - rOuter}`,
      `M ${cx} ${cy - rInner}`,
      `A ${rInner} ${rInner} 0 1 1 ${cx} ${cy + rInner}`,
      `A ${rInner} ${rInner} 0 1 1 ${cx} ${cy - rInner}`,
      "Z",
    ].join(" ");

    return {
      slices: [
        {
          model: single.model,
          name,
          tokens: single.total_tokens,
          calls: single.calls,
          percentage: 100,
          color,
          path: fullRingPath,
        },
      ],
      allTokens,
    };
  }

  let currentAngle = -Math.PI / 2;
  const slices: UsageDonutSlice[] = [];

  for (let i = 0; i < rows.length; i++) {
    const item = rows[i];
    if (item.total_tokens <= 0) {
      continue;
    }
    const isOther = item.model === USAGE_OTHER_MODEL_ID;
    const name = displayUsageModelName(item.model, unknownLabel, otherLabel);
    const color = getUsageModelColor(i, isOther);
    const fraction = item.total_tokens / allTokens;
    const sweepAngle = fraction * 2 * Math.PI;
    const startAngle = currentAngle;
    const endAngle = currentAngle + sweepAngle;
    currentAngle = endAngle;

    const x1 = cx + rOuter * Math.cos(startAngle);
    const y1 = cy + rOuter * Math.sin(startAngle);
    const x2 = cx + rOuter * Math.cos(endAngle);
    const y2 = cy + rOuter * Math.sin(endAngle);

    const x3 = cx + rInner * Math.cos(endAngle);
    const y3 = cy + rInner * Math.sin(endAngle);
    const x4 = cx + rInner * Math.cos(startAngle);
    const y4 = cy + rInner * Math.sin(startAngle);

    const largeArc = sweepAngle > Math.PI ? 1 : 0;
    const path = `M ${x1.toFixed(3)} ${y1.toFixed(3)} A ${rOuter} ${rOuter} 0 ${largeArc} 1 ${x2.toFixed(3)} ${y2.toFixed(3)} L ${x3.toFixed(3)} ${y3.toFixed(3)} A ${rInner} ${rInner} 0 ${largeArc} 0 ${x4.toFixed(3)} ${y4.toFixed(3)} Z`;

    slices.push({
      model: item.model,
      name,
      tokens: item.total_tokens,
      calls: item.calls,
      percentage: fraction * 100,
      color,
      path,
    });
  }

  return { slices, allTokens };
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

export function usageAnalyticsLoadError(error: unknown, fallback: string): string {
  if (!(error instanceof Error)) {
    return fallback;
  }
  const message = error.message.trim();
  if (!message || /invoke|tauri|undefined|not allowed|ipc/i.test(message)) {
    return fallback;
  }
  return message;
}
