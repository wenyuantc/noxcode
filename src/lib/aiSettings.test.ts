import { describe, expect, it } from "vitest";

import {
  EMPTY_AI_COMMIT_MESSAGE,
  EMPTY_AI_FEATURE_OVERRIDE,
  EMPTY_AI_PROMPT_ENHANCEMENT,
  fillOverrideDefaults,
  normalizeAiSettings,
  selectOverrideChannel,
  selectOverrideModel,
  withCommitMessageDefaults,
  withEnabledOverride,
} from "./aiSettings";
import type { AiChannel, AiChannelModel } from "./types";

function model(id: string, thinking = true, levels = ["low", "medium", "high"]): AiChannelModel {
  return {
    id,
    context_tokens: 128000,
    max_output_tokens: 4096,
    thinking_enabled: thinking,
    thinking_level: thinking ? "medium" : null,
    thinking_levels: thinking ? levels : null,
    input_types: ["text"],
  };
}

function channel(id: string, models: AiChannelModel[], enabled = true): AiChannel {
  return {
    id,
    name: id,
    protocol: "openai",
    base_url: "https://example.test",
    extra_headers_json: null,
    models,
    responses_continuation: "auto",
    enabled,
    api_key: null,
    api_key_configured: true,
    created_at: "",
    updated_at: "",
  };
}

describe("aiSettings helpers", () => {
  const channels = [
    channel("disabled", [model("off")], false),
    channel("alpha", [model("gpt-a"), model("gpt-b", true, ["low", "high"])]),
    channel("beta", [model("lite", false)]),
  ];

  it("fills the first enabled channel when none is selected", () => {
    const next = fillOverrideDefaults(EMPTY_AI_FEATURE_OVERRIDE, channels);
    expect(next.channel_id).toBe("alpha");
    expect(next.model).toBe("gpt-a");
    expect(next.reasoning_effort).toBe("medium");
  });

  it("keeps a valid selection and remaps invalid effort", () => {
    const next = fillOverrideDefaults(
      {
        enabled: true,
        channel_id: "alpha",
        model: "gpt-b",
        reasoning_effort: "medium",
      },
      channels,
    );
    expect(next.model).toBe("gpt-b");
    expect(next.reasoning_effort).toBe("low");
  });

  it("clears model fields when enabling without channels", () => {
    const next = withEnabledOverride(EMPTY_AI_FEATURE_OVERRIDE, [], true);
    expect(next).toEqual({
      enabled: true,
      channel_id: null,
      model: null,
      reasoning_effort: null,
    });
  });

  it("keeps commit message style when filling defaults", () => {
    const next = fillOverrideDefaults(
      { ...EMPTY_AI_COMMIT_MESSAGE, enabled: true, style: "concise" },
      channels,
    );
    expect(next.style).toBe("concise");
    expect(next.channel_id).toBe("alpha");
  });

  it("defaults missing commit message style to detailed", () => {
    const next = withCommitMessageDefaults({
      enabled: true,
      channel_id: "alpha",
      model: "gpt-a",
      reasoning_effort: "medium",
    });
    expect(next.style).toBe("detailed");
  });

  it("defaults prompt enhancement to enabled when legacy settings omit it", () => {
    const next = normalizeAiSettings({
      commit_message: EMPTY_AI_COMMIT_MESSAGE,
      session_title: EMPTY_AI_FEATURE_OVERRIDE,
    } as never);
    expect(next.prompt_enhancement).toEqual(EMPTY_AI_PROMPT_ENHANCEMENT);
  });

  it("resets model when switching channel", () => {
    const next = selectOverrideChannel(
      {
        enabled: true,
        channel_id: "alpha",
        model: "gpt-a",
        reasoning_effort: "medium",
      },
      channels,
      "beta",
    );
    expect(next.channel_id).toBe("beta");
    expect(next.model).toBe("lite");
    expect(next.reasoning_effort).toBeNull();
  });

  it("resets effort when switching model", () => {
    const next = selectOverrideModel(
      {
        enabled: true,
        channel_id: "alpha",
        model: "gpt-a",
        reasoning_effort: "medium",
      },
      channels,
      "gpt-b",
    );
    expect(next.model).toBe("gpt-b");
    expect(next.reasoning_effort).toBe("low");
  });
});
