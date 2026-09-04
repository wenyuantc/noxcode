import { describe, expect, it } from "vitest";

import { resolveComposerPlanMode } from "./planMode";

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
