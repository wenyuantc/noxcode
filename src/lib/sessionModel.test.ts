import { describe, expect, it } from "vitest";

import { emptyChannelModel } from "./modelCatalog";
import {
  mergeSessionRuntime,
  planApprovalModelArgs,
  resolvePlanApprovalThinking,
  resolveSessionSelection,
} from "./sessionModel";
import type { NativeSessionRuntime } from "./types";

const runtime: NativeSessionRuntime = {
  ai_channel_id: "live-ch",
  model: "live-model",
  reasoning_effort: "high",
  permission_mode: "edit",
  plan_mode: false,
};

describe("resolveSessionSelection", () => {
  it("uses the global default when no session is selected", () => {
    expect(
      resolveSessionSelection({
        session: { ai_channel_id: "old-ch", model: "old-model" },
        fallbackChannelId: "global-ch",
        fallbackModelId: "global-model",
      }),
    ).toEqual({ channelId: "global-ch", modelId: "global-model" });
  });

  it("prefers live runtime over transcript and global values", () => {
    expect(
      resolveSessionSelection({
        sessionId: "s1",
        runtime,
        session: { ai_channel_id: "old-ch", model: "old-model" },
        fallbackChannelId: "global-ch",
        fallbackModelId: "global-model",
      }),
    ).toEqual({ channelId: "live-ch", modelId: "live-model" });
  });

  it("uses the transcript model when the session has no runtime", () => {
    expect(
      resolveSessionSelection({
        sessionId: "s1",
        session: { ai_channel_id: "old-ch", model: "old-model" },
        fallbackChannelId: "global-ch",
        fallbackModelId: "global-model",
      }),
    ).toEqual({ channelId: "old-ch", modelId: "old-model" });
  });

  it("falls back to the global default when the session has no last model", () => {
    expect(
      resolveSessionSelection({
        sessionId: "s1",
        session: { ai_channel_id: null, model: "  " },
        fallbackChannelId: "global-ch",
        fallbackModelId: "global-model",
      }),
    ).toEqual({ channelId: "global-ch", modelId: "global-model" });
  });
});

describe("mergeSessionRuntime", () => {
  it("patches only the current runtime", () => {
    expect(
      mergeSessionRuntime(
        runtime,
        { model: "new" },
        {
          channelId: "global-ch",
          modelId: "global-model",
          permissionMode: "default",
          planMode: true,
        },
      ),
    ).toEqual({ ...runtime, model: "new" });
  });

  it("creates a session runtime from last-used values when none exists", () => {
    expect(
      mergeSessionRuntime(
        undefined,
        { model: "new" },
        {
          channelId: "old-ch",
          modelId: "old-model",
          permissionMode: "build",
          planMode: false,
        },
      ),
    ).toEqual({
      ai_channel_id: "old-ch",
      model: "new",
      reasoning_effort: null,
      permission_mode: "build",
      plan_mode: false,
    });
  });
});

describe("planApprovalModelArgs", () => {
  it("passes the selected channel and model only when approving", () => {
    expect(
      planApprovalModelArgs(true, { channelId: "ch-1", modelId: "deepseek-v4-flash" }),
    ).toEqual({ aiChannelId: "ch-1", model: "deepseek-v4-flash" });
  });

  it("includes reasoning effort when approving a thinking model", () => {
    expect(
      planApprovalModelArgs(true, { channelId: "ch-1", modelId: "deepseek-v4-flash" }, "max"),
    ).toEqual({
      aiChannelId: "ch-1",
      model: "deepseek-v4-flash",
      reasoningEffort: "max",
    });
  });

  it("omits the model when rejecting or when the selection is empty", () => {
    expect(
      planApprovalModelArgs(false, { channelId: "ch-1", modelId: "deepseek-v4-flash" }, "max"),
    ).toEqual({});
    expect(planApprovalModelArgs(true, { channelId: "  ", modelId: "m" }, "max")).toEqual({});
    expect(planApprovalModelArgs(true, { channelId: null, modelId: "m" }, "max")).toEqual({});
    expect(
      planApprovalModelArgs(true, { channelId: "ch-1", modelId: "deepseek-v4-flash" }, "  "),
    ).toEqual({ aiChannelId: "ch-1", model: "deepseek-v4-flash" });
  });
});

describe("resolvePlanApprovalThinking", () => {
  const thinkingModel = {
    ...emptyChannelModel("deepseek-v4-flash"),
    thinking_enabled: true,
    thinking_level: "high",
    thinking_levels: ["low", "high", "max"],
  };
  const silentModel = {
    ...emptyChannelModel("plain"),
    thinking_enabled: false,
  };
  const channels = [
    { id: "ch-1", models: [thinkingModel] },
    { id: "ch-2", models: [silentModel] },
  ];

  it("exposes allowed levels and keeps a valid preferred effort", () => {
    expect(
      resolvePlanApprovalThinking({
        channels,
        selection: { channelId: "ch-1", modelId: "deepseek-v4-flash" },
        preferredEffort: "max",
      }),
    ).toEqual({ enabled: true, levels: ["low", "high", "max"], effort: "max" });
  });

  it("falls back to the model default when the preferred effort is out of range", () => {
    expect(
      resolvePlanApprovalThinking({
        channels,
        selection: { channelId: "ch-1", modelId: "deepseek-v4-flash" },
        preferredEffort: "xhigh",
      }).effort,
    ).toBe("high");
  });

  it("hides the picker when the selected model has thinking off", () => {
    expect(
      resolvePlanApprovalThinking({
        channels,
        selection: { channelId: "ch-2", modelId: "plain" },
        preferredEffort: "max",
      }),
    ).toEqual({ enabled: false, levels: [], effort: "medium" });
  });
});
