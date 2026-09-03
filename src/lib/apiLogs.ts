import type { NativeApiCallLogListItem, NativeApiCallLogStats } from "@/lib/types";

export const API_CALL_LOG_PAGE_SIZE = 20;

export type ApiCallLogStatus = "success" | "failed" | "cancelled";

export const API_CALL_LOG_STATUSES: ApiCallLogStatus[] = ["success", "failed", "cancelled"];

export function isApiCallLogStatus(value: string | null | undefined): value is ApiCallLogStatus {
  return value === "success" || value === "failed" || value === "cancelled";
}

function formatCompactTokenCount(value: number): string {
  if (!Number.isFinite(value)) {
    return "0";
  }
  const abs = Math.abs(value);
  if (abs >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(2)}M`;
  }
  if (abs >= 1_000) {
    const kText = (value / 1_000).toFixed(2);
    if (Math.abs(Number(kText)) >= 1000) {
      return `${(value / 1_000_000).toFixed(2)}M`;
    }
    return `${kText}K`;
  }
  return `${value}`;
}

export function formatApiCallLogTokenCount(
  value: number | null | undefined,
  unknownLabel: string,
): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return unknownLabel;
  }
  return formatCompactTokenCount(value);
}

export function formatApiCallLogCacheRate(
  inputTokens: number | null | undefined,
  cachedTokens: number | null | undefined,
  labels: { unknown: string; empty: string },
): string {
  if (
    inputTokens === null ||
    inputTokens === undefined ||
    cachedTokens === null ||
    cachedTokens === undefined ||
    !Number.isFinite(inputTokens) ||
    !Number.isFinite(cachedTokens)
  ) {
    return labels.unknown;
  }

  const cached = Math.max(0, cachedTokens);
  const prompt = Math.max(0, inputTokens);
  const denominator = cached > prompt ? prompt + cached : prompt;
  if (denominator <= 0) {
    return labels.empty;
  }
  return `${((cached / denominator) * 100).toFixed(1)}%`;
}

export function formatApiCallLogDurationMs(
  value: number | null | undefined,
  unknownLabel: string,
  lessThanOneSecondLabel = "<1s",
): string {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return unknownLabel;
  }

  const safeMs = Math.max(0, value);
  const seconds = Math.floor(safeMs / 1000);
  if (seconds < 1) {
    return lessThanOneSecondLabel;
  }
  return `${seconds}s`;
}

/**
 * Whole-call output throughput: `t/s = output_tokens * 1000 / duration_ms`.
 *
 * Usage `output_tokens` includes thinking tokens. Thinking streams often
 * record `first_token_ms` near the end of the request, so subtracting TTFT
 * would treat the last few hundred milliseconds as the entire generation
 * window and inflate the rate into thousands of t/s.
 */
export function formatApiCallLogThroughput(
  outputTokens: number | null | undefined,
  durationMs: number | null | undefined,
  unknownLabel: string,
): string {
  if (
    outputTokens === null ||
    outputTokens === undefined ||
    durationMs === null ||
    durationMs === undefined ||
    !Number.isFinite(outputTokens) ||
    !Number.isFinite(durationMs) ||
    outputTokens < 0 ||
    durationMs <= 0
  ) {
    return unknownLabel;
  }

  const tokensPerSecond = (outputTokens * 1000) / durationMs;
  if (!Number.isFinite(tokensPerSecond)) {
    return unknownLabel;
  }

  return `${tokensPerSecond.toFixed(2)} t/s`;
}

export function formatApiCallLogThinking(
  item: Pick<NativeApiCallLogListItem, "thinking_enabled" | "thinking_level">,
  unknownLabel: string,
  offLabel: string,
): string {
  if (!item.thinking_enabled) {
    return offLabel;
  }
  const level = item.thinking_level?.trim();
  return level ? level : unknownLabel;
}

export function isTruncatedFlag(value: number | null | undefined): boolean {
  return (value ?? 0) > 0;
}

export function prettyPrintJsonBody(value: string | null | undefined): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) {
    return "";
  }

  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return value ?? "";
  }
}

export function emptyApiCallLogStats(): NativeApiCallLogStats {
  return {
    total: 0,
    success: 0,
    failed: 0,
    cancelled: 0,
    input_tokens: 0,
    output_tokens: 0,
    cached_tokens_sum: null,
    total_tokens_sum: null,
    avg_first_token_ms: null,
    avg_duration_ms: null,
  };
}

export function nextApiCallLogRequestPage(queryChanged: boolean, currentPage: number): number {
  if (queryChanged) {
    return 1;
  }
  return Number.isFinite(currentPage) && currentPage > 1 ? Math.floor(currentPage) : 1;
}

export function resolveApiCallLogListPage<T>(
  requestedPage: number,
  result: { items: T[]; total: number },
  pageSize: number = API_CALL_LOG_PAGE_SIZE,
): {
  page: number;
  items: T[];
  total: number;
  needsRefetch: boolean;
} {
  const total = Number.isFinite(result.total) ? Math.max(0, result.total) : 0;
  const totalPages = total > 0 ? Math.ceil(total / pageSize) : 0;
  const safePage = Number.isFinite(requestedPage) ? Math.floor(requestedPage) : 1;

  if (totalPages === 0) {
    return { page: 1, items: [], total: 0, needsRefetch: false };
  }

  if (safePage < 1) {
    return { page: 1, items: [], total, needsRefetch: true };
  }

  if (safePage > totalPages) {
    return { page: totalPages, items: [], total, needsRefetch: true };
  }

  return {
    page: safePage,
    items: result.items,
    total,
    needsRefetch: false,
  };
}
