import { describe, expect, it } from "vitest";

import type { NativeUsageDailyBucket, NativeUsageModelBucket } from "@/lib/types";
import {
  USAGE_CUSTOM_RANGE_MAX_DAYS,
  USAGE_OTHER_MODEL_ID,
  buildHeatmapMonthLabels,
  buildHeatmapWeeks,
  buildUsageDonutSlices,
  displayUsageModelName,
  fillUsageDailyBuckets,
  formatUsageDateWithWeekday,
  formatUsageDayLabel,
  formatUsageTokenCompact,
  formatUsageTokenCount,
  getUsageModelColor,
  heatmapLevel,
  inclusiveDayCount,
  mergeUsageModels,
  resolveHeatmapYearRange,
  resolveUsageDateRange,
  shiftUtcDateKey,
  usageAnalyticsLoadError,
  usageCacheHitRatio,
  usageTotalTokens,
  usageTrendLabelIndexes,
  utcDateKey,
} from "@/lib/usageAnalytics";

function daily(
  date: string,
  overrides: Partial<NativeUsageDailyBucket> = {},
): NativeUsageDailyBucket {
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
    ...overrides,
  };
}

function model(
  name: string,
  overrides: Partial<NativeUsageModelBucket> = {},
): NativeUsageModelBucket {
  return {
    model: name,
    calls: 1,
    input_tokens: 0,
    output_tokens: 0,
    cached_tokens: 0,
    total_tokens: 0,
    ...overrides,
  };
}

describe("resolveUsageDateRange", () => {
  const now = new Date("2026-09-04T15:30:00.000Z");

  it("uses inclusive 7-day and 30-day windows ending today UTC", () => {
    expect(resolveUsageDateRange("7d", "", "", now)).toEqual({
      ok: true,
      start: "2026-08-29",
      end: "2026-09-04",
    });
    expect(resolveUsageDateRange("30d", "", "", now)).toEqual({
      ok: true,
      start: "2026-08-06",
      end: "2026-09-04",
    });
  });

  it("validates custom ranges", () => {
    expect(resolveUsageDateRange("custom", "", "2026-09-04", now)).toEqual({
      ok: false,
      reason: "incomplete",
    });
    expect(resolveUsageDateRange("custom", "2026-09-04", "2026-09-01", now)).toEqual({
      ok: false,
      reason: "order",
    });
    expect(
      resolveUsageDateRange(
        "custom",
        "2025-09-01",
        shiftUtcDateKey("2025-09-01", USAGE_CUSTOM_RANGE_MAX_DAYS),
        now,
      ),
    ).toEqual({ ok: false, reason: "span" });
    expect(resolveUsageDateRange("custom", "2026-08-01", "2026-08-10", now)).toEqual({
      ok: true,
      start: "2026-08-01",
      end: "2026-08-10",
    });
  });
});

describe("fillUsageDailyBuckets", () => {
  it("fills missing days with zeros and keeps reported totals", () => {
    const filled = fillUsageDailyBuckets("2026-09-01", "2026-09-03", [
      daily("2026-09-01", { calls: 2, total_tokens: 40 }),
      daily("2026-09-03", { calls: 1, total_tokens: 10 }),
    ]);
    expect(filled.map((item) => item.date)).toEqual(["2026-09-01", "2026-09-02", "2026-09-03"]);
    expect(filled[1]).toMatchObject({ calls: 0, total_tokens: 0 });
    expect(filled[0].total_tokens).toBe(40);
    expect(filled[2].calls).toBe(1);
  });
});

describe("formatUsageTokenCount", () => {
  it("compacts thousands as k and millions as M", () => {
    expect(formatUsageTokenCount(0)).toBe("0");
    expect(formatUsageTokenCount(999)).toBe("999");
    expect(formatUsageTokenCount(1000)).toBe("1k");
    expect(formatUsageTokenCount(1500)).toBe("1.5k");
    expect(formatUsageTokenCount(15_000)).toBe("15k");
    expect(formatUsageTokenCount(999_499)).toBe("999k");
    expect(formatUsageTokenCount(1_000_000)).toBe("1M");
    expect(formatUsageTokenCount(1_250_000)).toBe("1.3M");
    expect(formatUsageTokenCount(12_000_000)).toBe("12M");
  });
});

describe("usageTotalTokens and cache ratio", () => {
  it("prefers total_tokens_sum and falls back to input plus output", () => {
    expect(usageTotalTokens({ input_tokens: 10, output_tokens: 4, total_tokens_sum: 20 })).toBe(20);
    expect(usageTotalTokens({ input_tokens: 10, output_tokens: 4, total_tokens_sum: null })).toBe(
      14,
    );
  });

  it("mirrors API log cache-rate math", () => {
    expect(usageCacheHitRatio(100, 25)).toBe(0.25);
    expect(usageCacheHitRatio(50, 200)).toBe(0.8);
    expect(usageCacheHitRatio(0, 0)).toBeNull();
    expect(usageCacheHitRatio(10, null)).toBeNull();
  });
});

describe("heatmap and model merge", () => {
  it("assigns 0-4 levels from the range maximum", () => {
    expect(heatmapLevel(0, 100)).toBe(0);
    expect(heatmapLevel(20, 100)).toBe(1);
    expect(heatmapLevel(40, 100)).toBe(2);
    expect(heatmapLevel(70, 100)).toBe(3);
    expect(heatmapLevel(100, 100)).toBe(4);
  });

  it("pads weeks from Monday and marks out-of-range cells", () => {
    const weeks = buildHeatmapWeeks(
      [daily("2026-09-02", { calls: 3, total_tokens: 80 })],
      "2026-09-01",
      "2026-09-03",
    );
    expect(weeks).toHaveLength(1);
    expect(weeks[0]?.cells.map((cell) => cell.date)).toEqual([
      "2026-08-31",
      "2026-09-01",
      "2026-09-02",
      "2026-09-03",
      "2026-09-04",
      "2026-09-05",
      "2026-09-06",
    ]);
    expect(weeks[0]?.cells[0]?.inRange).toBe(false);
    expect(weeks[0]?.cells[2]).toMatchObject({
      inRange: true,
      calls: 3,
      totalTokens: 80,
      level: 4,
    });
  });

  it("merges overflow models into a sentinel bucket", () => {
    const models = Array.from({ length: 10 }, (_, index) =>
      model(`m${index}`, { calls: 1, total_tokens: 10 - index }),
    );
    const merged = mergeUsageModels(models, 8);
    expect(merged).toHaveLength(9);
    expect(merged[8]).toMatchObject({
      model: USAGE_OTHER_MODEL_ID,
      calls: 2,
      total_tokens: 3,
    });
    expect(displayUsageModelName("", "未知", "其他")).toBe("未知");
    expect(displayUsageModelName(USAGE_OTHER_MODEL_ID, "未知", "其他")).toBe("其他");
  });
});

describe("trend helpers", () => {
  it("keeps short series labels and samples longer ones", () => {
    expect(usageTrendLabelIndexes(4)).toEqual([0, 1, 2, 3]);
    const longIndexes = usageTrendLabelIndexes(30);
    expect(longIndexes[0]).toBe(0);
    expect(longIndexes[longIndexes.length - 1]).toBe(29);
    expect(longIndexes.length).toBeLessThanOrEqual(8);
  });

  it("formats UTC day labels without shifting the calendar date", () => {
    expect(utcDateKey(new Date("2026-09-04T01:00:00.000Z"))).toBe("2026-09-04");
    expect(inclusiveDayCount("2026-09-01", "2026-09-03")).toBe(3);
    expect(formatUsageDayLabel("2026-09-04")).toBe("9/4");
  });
});

describe("usageAnalyticsLoadError", () => {
  it("hides Tauri invoke noise and keeps backend messages", () => {
    expect(
      usageAnalyticsLoadError(
        new Error("Cannot read properties of undefined (reading 'invoke')"),
        "加载失败",
      ),
    ).toBe("加载失败");
    expect(usageAnalyticsLoadError(new Error("统计使用数据失败: db locked"), "加载失败")).toBe(
      "统计使用数据失败: db locked",
    );
    expect(usageAnalyticsLoadError("nope", "加载失败")).toBe("加载失败");
  });
});

describe("formatUsageTokenCompact and formatUsageDateWithWeekday", () => {
  it("formats tokens in Chinese units (万, 亿) or English units (k, M, B)", () => {
    expect(formatUsageTokenCompact(0, "zh-CN")).toBe("0");
    expect(formatUsageTokenCompact(500, "zh-CN")).toBe("500");
    expect(formatUsageTokenCompact(27_041_000, "zh-CN")).toBe("2704.1万");
    expect(formatUsageTokenCompact(28_931_000, "zh-CN")).toBe("2893.1万");
    expect(formatUsageTokenCompact(150_000_000, "zh-CN")).toBe("1.5亿");

    expect(formatUsageTokenCompact(500, "en")).toBe("500");
    expect(formatUsageTokenCompact(1_500, "en")).toBe("1.5k");
    expect(formatUsageTokenCompact(27_041_000, "en")).toBe("27M");
    expect(formatUsageTokenCompact(1_250_000_000, "en")).toBe("1.3B");
  });

  it("formats UTC date with weekday in Chinese and English", () => {
    // 2026-09-05 is Saturday
    expect(formatUsageDateWithWeekday("2026-09-05", "zh-CN")).toBe("2026年9月5日 周六");
    expect(formatUsageDateWithWeekday("2026-09-05", "en")).toBe("Sep 5, 2026, Sat");
    expect(formatUsageDateWithWeekday("invalid-date", "zh-CN")).toBe("invalid-date");
  });
});

describe("buildUsageDonutSlices and getUsageModelColor", () => {
  it("returns assigned palette colors and slate for other", () => {
    expect(getUsageModelColor(0)).toBe("#3b82f6");
    expect(getUsageModelColor(1)).toBe("#10b981");
    expect(getUsageModelColor(0, true)).toBe("#64748b");
  });

  it("handles empty models or zero tokens", () => {
    const emptyResult = buildUsageDonutSlices([], "未知", "其他");
    expect(emptyResult.slices).toHaveLength(0);
    expect(emptyResult.allTokens).toBe(0);

    const zeroResult = buildUsageDonutSlices([model("gpt-4", { total_tokens: 0 })], "未知", "其他");
    expect(zeroResult.slices).toHaveLength(0);
    expect(zeroResult.allTokens).toBe(0);
  });

  it("generates a full donut ring when only 1 model has positive tokens", () => {
    const singleResult = buildUsageDonutSlices(
      [model("gpt-4", { total_tokens: 1000, calls: 5 })],
      "未知",
      "其他",
    );
    expect(singleResult.slices).toHaveLength(1);
    expect(singleResult.slices[0].percentage).toBe(100);
    expect(singleResult.slices[0].path).toContain("A 68 68");
  });

  it("generates slices with correct percentages and paths for multiple models", () => {
    const multiResult = buildUsageDonutSlices(
      [
        model("gpt-4", { total_tokens: 600, calls: 10 }),
        model("claude-3", { total_tokens: 400, calls: 8 }),
      ],
      "未知",
      "其他",
    );
    expect(multiResult.slices).toHaveLength(2);
    expect(multiResult.allTokens).toBe(1000);
    expect(multiResult.slices[0].percentage).toBe(60);
    expect(multiResult.slices[1].percentage).toBe(40);
    expect(multiResult.slices[0].color).toBe("#3b82f6");
    expect(multiResult.slices[1].color).toBe("#10b981");
  });
});

describe("resolveHeatmapYearRange and buildHeatmapMonthLabels", () => {
  const now = new Date("2026-09-09T12:00:00.000Z");

  it("resolves lastYear to 52 full weeks ending on current week", () => {
    const range = resolveHeatmapYearRange("lastYear", now);
    expect(range.end).toBe("2026-09-09");
    expect(range.maxDate).toBe("2026-09-09");
    // 2026-09-09 is Wednesday, monday is 2026-09-07. 51 weeks before is 2025-09-15.
    expect(range.start).toBe("2025-09-15");

    const weeks = buildHeatmapWeeks([], range.start, range.end);
    expect(weeks).toHaveLength(52);
    // First week Monday
    expect(weeks[0]?.cells[0]?.date).toBe("2025-09-15");
    // Last week Monday is 2026-09-07, Sunday is 2026-09-13
    expect(weeks[51]?.cells[0]?.date).toBe("2026-09-07");
    expect(weeks[51]?.cells[6]?.date).toBe("2026-09-13");
    // Today (Wednesday) is in range, Thursday is not in range
    expect(weeks[51]?.cells[2]?.date).toBe("2026-09-09");
    expect(weeks[51]?.cells[2]?.inRange).toBe(true);
    expect(weeks[51]?.cells[3]?.date).toBe("2026-09-10");
    expect(weeks[51]?.cells[3]?.inRange).toBe(false);
  });

  it("resolves calendar year range correctly", () => {
    const pastYearRange = resolveHeatmapYearRange("2025", now);
    expect(pastYearRange.start).toBe("2025-01-01");
    expect(pastYearRange.end).toBe("2025-12-31");
    expect(pastYearRange.maxDate).toBe("2025-12-31");

    const currentYearRange = resolveHeatmapYearRange("2026", now);
    expect(currentYearRange.start).toBe("2026-01-01");
    expect(currentYearRange.end).toBe("2026-12-31");
    expect(currentYearRange.maxDate).toBe("2026-09-09");

    const weeks2026 = buildHeatmapWeeks([], currentYearRange.start, currentYearRange.end);
    expect(weeks2026).toHaveLength(53);
    const labels2026Zh = buildHeatmapMonthLabels(weeks2026, "zh-CN");
    expect(labels2026Zh[0]?.label).toBe("1月");
  });

  it("builds non-overlapping month labels across weeks", () => {
    const range = resolveHeatmapYearRange("lastYear", now);
    const weeks = buildHeatmapWeeks([], range.start, range.end);
    const labelsZh = buildHeatmapMonthLabels(weeks, "zh-CN");
    const labelsEn = buildHeatmapMonthLabels(weeks, "en");

    expect(labelsZh.length).toBeGreaterThanOrEqual(10);
    expect(labelsZh[0]?.label).toBe("9月");
    expect(labelsEn[0]?.label).toBe("Sep");

    // Ensure all labels are at least 2 columns apart
    for (let i = 1; i < labelsZh.length; i += 1) {
      expect(labelsZh[i].colIndex - labelsZh[i - 1].colIndex).toBeGreaterThanOrEqual(2);
    }
  });
});
