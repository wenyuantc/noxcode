import { describe, expect, it, vi } from "vitest";

import { applyComposerPlanMode, resolveComposerPlanMode } from "./planMode";

describe("resolveComposerPlanMode", () => {
  it("uses the selected session mode, including false", () => {
    expect(resolveComposerPlanMode("s1", { s1: false }, true)).toBe(false);
    expect(resolveComposerPlanMode("s1", { s1: true }, false)).toBe(true);
  });

  it("falls back to the default for an unknown or empty session", () => {
    expect(resolveComposerPlanMode("s2", { s1: true }, true)).toBe(true);
    expect(resolveComposerPlanMode(null, { s1: false }, true)).toBe(true);
    expect(resolveComposerPlanMode(undefined, {}, false)).toBe(false);
  });
});

describe("applyComposerPlanMode", () => {
  it("writes the default and the selected session", () => {
    const setDefault = vi.fn();
    const setSession = vi.fn();
    applyComposerPlanMode({
      enabled: true,
      sessionId: "s1",
      setDefault,
      setSession,
    });
    expect(setDefault).toHaveBeenCalledWith(true);
    expect(setSession).toHaveBeenCalledWith("s1", true);
  });

  it("skips the session write when no session is selected", () => {
    const setDefault = vi.fn();
    const setSession = vi.fn();
    applyComposerPlanMode({
      enabled: false,
      sessionId: null,
      setDefault,
      setSession,
    });
    expect(setDefault).toHaveBeenCalledWith(false);
    expect(setSession).not.toHaveBeenCalled();
  });
});
