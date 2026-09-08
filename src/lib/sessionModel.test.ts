import { describe, expect, it } from "vitest";

import {
  mergeSessionRuntime,
  planApprovalModelArgs,
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

  it("omits the model when rejecting or when the selection is empty", () => {
    expect(
      planApprovalModelArgs(false, { channelId: "ch-1", modelId: "deepseek-v4-flash" }),
    ).toEqual({});
    expect(planApprovalModelArgs(true, { channelId: "  ", modelId: "m" })).toEqual({});
    expect(planApprovalModelArgs(true, { channelId: null, modelId: "m" })).toEqual({});
  });
});
