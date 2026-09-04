import { describe, expect, it } from "vitest";

import {
  FALLBACK_CONTEXT_WINDOW_TOKENS,
  parseContextUsageJson,
  resolveHistoricalUsage,
  resolveHistoryLimitTokens,
  usageFromUsageLines,
} from "./contextUsage";
import type { AiChannel } from "./types";

function channel(id: string, modelId: string, contextTokens: number | null): AiChannel {
  return {
    id,
    name: id,
    protocol: "openai",
    base_url: "https://example.test",
    extra_headers_json: null,
    models: [
      {
        id: modelId,
        context_tokens: contextTokens,
        max_output_tokens: null,
        thinking_enabled: null,
        thinking_level: null,
        thinking_levels: null,
        input_types: null,
      },
    ],
    enabled: true,
    api_key: null,
    api_key_configured: false,
    created_at: "t",
    updated_at: "t",
  };
}

describe("contextUsage", () => {
  it("parses a persisted snapshot", () => {
    const usage = parseContextUsageJson(
      "s1",
      JSON.stringify({
        session_record_id: "s1",
        used_tokens: 28000,
        limit_tokens: 500000,
        generation: 1,
        compactions: 0,
        cached_tokens: 100,
        prompt_tokens: 200,
      }),
    );
    expect(usage?.used_tokens).toBe(28000);
    expect(usage?.limit_tokens).toBe(500000);
    expect(usage?.cached_tokens).toBe(100);
  });

  it("rejects invalid json and missing limit", () => {
    expect(parseContextUsageJson("s1", "not-json")).toBeUndefined();
    expect(parseContextUsageJson("s1", JSON.stringify({ used_tokens: 1 }))).toBeUndefined();
    expect(
      parseContextUsageJson("s1", JSON.stringify({ used_tokens: 1, limit_tokens: 0 })),
    ).toBeUndefined();
  });

  it("builds fallback usage from the last 用量 line", () => {
    const usage = usageFromUsageLines(
      "s1",
      [
        { text: "[用量] in=10 out=1 cache=2 total=11" },
        { text: "[用量] in=28000 out=120 cache=22410 total=28120" },
      ],
      500000,
    );
    expect(usage).toMatchObject({
      session_record_id: "s1",
      used_tokens: 28000,
      limit_tokens: 500000,
      prompt_tokens: 28000,
      cached_tokens: 22410,
    });
  });

  it("prefers persisted json over 用量 fallback", () => {
    const usage = resolveHistoricalUsage({
      sessionId: "s1",
      contextUsageJson: JSON.stringify({
        session_record_id: "s1",
        used_tokens: 12,
        limit_tokens: 128000,
        generation: 0,
        compactions: 0,
      }),
      lines: [{ text: "[用量] in=28000 out=1 cache=1 total=28001" }],
      limitTokens: 500000,
    });
    expect(usage?.used_tokens).toBe(12);
    expect(usage?.limit_tokens).toBe(128000);
  });

  it("resolves the session channel model window or falls back", () => {
    const channels = [channel("ch-1", "grok-4.6", 500000)];
    expect(resolveHistoryLimitTokens({ ai_channel_id: "ch-1" }, channels, "ch-1", "grok-4.6")).toBe(
      500000,
    );
    expect(resolveHistoryLimitTokens(undefined, [], null, null)).toBe(
      FALLBACK_CONTEXT_WINDOW_TOKENS,
    );
  });
});
