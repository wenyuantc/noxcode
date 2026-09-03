import { describe, expect, it, vi } from "vitest";

import { sessionSubmissionPayload, submitSessionPrompt } from "./sessionSubmission";

function api() {
  return {
    startNativeSession: vi.fn().mockResolvedValue(undefined),
    resumeNativeSession: vi.fn().mockResolvedValue(undefined),
  };
}

const base = {
  workspaceId: "ws-1",
  channelId: "ch-1",
  prompt: "你好",
  model: "grok-4.6",
  reasoningEffort: "medium",
  planMode: false,
};

describe("submitSessionPrompt", () => {
  it("starts a new session when no session is selected", async () => {
    const backend = api();
    await submitSessionPrompt(base, backend);
    expect(backend.startNativeSession).toHaveBeenCalledTimes(1);
    expect(backend.startNativeSession).toHaveBeenCalledWith(sessionSubmissionPayload(base));
    expect(backend.resumeNativeSession).not.toHaveBeenCalled();
  });

  it("resumes the selected session instead of starting a new one", async () => {
    const backend = api();
    await submitSessionPrompt({ ...base, sessionId: "sess-1" }, backend);
    expect(backend.resumeNativeSession).toHaveBeenCalledTimes(1);
    expect(backend.resumeNativeSession).toHaveBeenCalledWith(
      sessionSubmissionPayload({ ...base, sessionId: "sess-1" }),
      "sess-1",
    );
    expect(backend.startNativeSession).not.toHaveBeenCalled();
  });

  it("treats a blank session id as a new session", async () => {
    const backend = api();
    await submitSessionPrompt({ ...base, sessionId: "   " }, backend);
    expect(backend.startNativeSession).toHaveBeenCalledTimes(1);
    expect(backend.resumeNativeSession).not.toHaveBeenCalled();
  });
});
