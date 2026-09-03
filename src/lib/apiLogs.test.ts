import { describe, expect, it } from "vitest";

import {
  API_CALL_LOG_PAGE_SIZE,
  emptyApiCallLogStats,
  formatApiCallLogCacheRate,
  formatApiCallLogDurationMs,
  formatApiCallLogThinking,
  formatApiCallLogThroughput,
  formatApiCallLogTokenCount,
  isApiCallLogStatus,
  isTruncatedFlag,
  nextApiCallLogRequestPage,
  prettyPrintJsonBody,
  resolveApiCallLogListPage,
} from "@/lib/apiLogs";

const CACHE_RATE_LABELS = { unknown: "未知", empty: "—" };

describe("formatApiCallLogTokenCount", () => {
  it("shows the unknown label instead of 0 when tokens are missing", () => {
    expect(formatApiCallLogTokenCount(null, "未知")).toBe("未知");
    expect(formatApiCallLogTokenCount(undefined, "Unknown")).toBe("Unknown");
  });

  it("keeps an explicit zero", () => {
    expect(formatApiCallLogTokenCount(0, "未知")).toBe("0");
  });

  it("compacts thousands as K and millions as M, matching conversation usage", () => {
    expect(formatApiCallLogTokenCount(950, "未知")).toBe("950");
    expect(formatApiCallLogTokenCount(1_250, "未知")).toBe("1.25K");
    expect(formatApiCallLogTokenCount(12_340, "未知")).toBe("12.34K");
    expect(formatApiCallLogTokenCount(999_499, "未知")).toBe("999.50K");
    expect(formatApiCallLogTokenCount(999_995, "未知")).toBe("1.00M");
    expect(formatApiCallLogTokenCount(1_000_000, "未知")).toBe("1.00M");
    expect(formatApiCallLogTokenCount(4_560_000, "未知")).toBe("4.56M");
    expect(formatApiCallLogTokenCount(12_000_000, "未知")).toBe("12.00M");
  });
});

describe("formatApiCallLogCacheRate", () => {
  it("keeps unknown when either side is missing", () => {
    expect(formatApiCallLogCacheRate(null, 25, CACHE_RATE_LABELS)).toBe("未知");
    expect(formatApiCallLogCacheRate(100, null, CACHE_RATE_LABELS)).toBe("未知");
    expect(formatApiCallLogCacheRate(undefined, undefined, CACHE_RATE_LABELS)).toBe("未知");
  });

  it("formats zero, normal, and overflow rates", () => {
    expect(formatApiCallLogCacheRate(100, 0, CACHE_RATE_LABELS)).toBe("0.0%");
    expect(formatApiCallLogCacheRate(100, 25, CACHE_RATE_LABELS)).toBe("25.0%");
    expect(formatApiCallLogCacheRate(10, 3, CACHE_RATE_LABELS)).toBe("30.0%");
    expect(formatApiCallLogCacheRate(50, 200, CACHE_RATE_LABELS)).toBe("80.0%");
  });

  it("shows the empty placeholder when cache is known but input is zero", () => {
    expect(formatApiCallLogCacheRate(0, 0, CACHE_RATE_LABELS)).toBe("—");
  });
});

describe("formatApiCallLogDurationMs", () => {
  it("renders whole seconds without rounding up", () => {
    expect(formatApiCallLogDurationMs(320, "未知")).toBe("<1s");
    expect(formatApiCallLogDurationMs(999, "未知")).toBe("<1s");
    expect(formatApiCallLogDurationMs(1000, "未知")).toBe("1s");
    expect(formatApiCallLogDurationMs(1200, "未知")).toBe("1s");
    expect(formatApiCallLogDurationMs(1999, "未知")).toBe("1s");
    expect(formatApiCallLogDurationMs(10000, "未知")).toBe("10s");
    expect(formatApiCallLogDurationMs(10500, "未知")).toBe("10s");
  });

  it("shows the unknown label when duration is missing", () => {
    expect(formatApiCallLogDurationMs(null, "未知")).toBe("未知");
    expect(formatApiCallLogDurationMs(Number.NaN, "未知")).toBe("未知");
  });

  it("clamps negative durations to <1s", () => {
    expect(formatApiCallLogDurationMs(-12, "未知")).toBe("<1s");
  });
});

describe("formatApiCallLogThroughput", () => {
  it("formats whole-call tokens per second to two decimals", () => {
    expect(formatApiCallLogThroughput(120, 4500, "未知")).toBe("26.67 t/s");
    expect(formatApiCallLogThroughput(1, 3, "未知")).toBe("333.33 t/s");
  });

  it("uses total duration so thinking dumps are not inflated into thousands of t/s", () => {
    expect(formatApiCallLogThroughput(3388, 64238, "未知")).toBe("52.74 t/s");
  });

  it("keeps an explicit zero when duration is valid", () => {
    expect(formatApiCallLogThroughput(0, 4500, "未知")).toBe("0.00 t/s");
  });

  it("shows unknown when output tokens or duration is missing", () => {
    expect(formatApiCallLogThroughput(null, 4500, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(undefined, 4500, "Unknown")).toBe("Unknown");
    expect(formatApiCallLogThroughput(120, null, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(undefined, undefined, "未知")).toBe("未知");
  });

  it("shows unknown for non-finite or negative telemetry", () => {
    expect(formatApiCallLogThroughput(Number.NaN, 4500, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(120, Number.POSITIVE_INFINITY, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(120, Number.NaN, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(-1, 4500, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(120, -1, "未知")).toBe("未知");
  });

  it("shows unknown when duration is not positive", () => {
    expect(formatApiCallLogThroughput(120, 0, "未知")).toBe("未知");
    expect(formatApiCallLogThroughput(0, 0, "未知")).toBe("未知");
  });
});

describe("formatApiCallLogThinking", () => {
  it("uses the off label when thinking is disabled", () => {
    expect(
      formatApiCallLogThinking({ thinking_enabled: 0, thinking_level: "high" }, "未知", "关闭"),
    ).toBe("关闭");
  });

  it("falls back to unknown when enabled but the level is empty", () => {
    expect(
      formatApiCallLogThinking({ thinking_enabled: 1, thinking_level: null }, "未知", "关闭"),
    ).toBe("未知");
  });

  it("returns the stored thinking level when enabled", () => {
    expect(
      formatApiCallLogThinking({ thinking_enabled: 1, thinking_level: "high" }, "未知", "关闭"),
    ).toBe("high");
  });
});

describe("prettyPrintJsonBody", () => {
  it("pretty-prints valid JSON and leaves invalid text unchanged", () => {
    expect(prettyPrintJsonBody('{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(prettyPrintJsonBody("not-json")).toBe("not-json");
    expect(prettyPrintJsonBody(null)).toBe("");
  });
});

describe("api log helpers", () => {
  it("recognizes known statuses and truncation flags", () => {
    expect(isApiCallLogStatus("success")).toBe(true);
    expect(isApiCallLogStatus("running")).toBe(false);
    expect(isTruncatedFlag(1)).toBe(true);
    expect(isTruncatedFlag(0)).toBe(false);
    expect(emptyApiCallLogStats().total).toBe(0);
  });
});

describe("nextApiCallLogRequestPage", () => {
  it("forces page 1 when filters or scope change, even from later pages", () => {
    expect(nextApiCallLogRequestPage(true, 4)).toBe(1);
    expect(nextApiCallLogRequestPage(true, 1)).toBe(1);
  });

  it("keeps the current page when only paginating", () => {
    expect(nextApiCallLogRequestPage(false, 3)).toBe(3);
    expect(nextApiCallLogRequestPage(false, 0)).toBe(1);
  });
});

describe("resolveApiCallLogListPage", () => {
  it("keeps a valid page and its items", () => {
    const items = [{ id: "p2" }];
    expect(resolveApiCallLogListPage(2, { items, total: API_CALL_LOG_PAGE_SIZE * 2 + 1 })).toEqual({
      page: 2,
      items,
      total: API_CALL_LOG_PAGE_SIZE * 2 + 1,
      needsRefetch: false,
    });
  });

  it("clears items immediately when the query has no rows", () => {
    expect(resolveApiCallLogListPage(3, { items: [{ id: "stale" }], total: 0 })).toEqual({
      page: 1,
      items: [],
      total: 0,
      needsRefetch: false,
    });
  });

  it("clears stale rows and signals a refetch when the page is past the last page", () => {
    expect(
      resolveApiCallLogListPage(5, {
        items: [{ id: "stale-page" }],
        total: API_CALL_LOG_PAGE_SIZE + 2,
      }),
    ).toEqual({
      page: 2,
      items: [],
      total: API_CALL_LOG_PAGE_SIZE + 2,
      needsRefetch: true,
    });
  });
});
