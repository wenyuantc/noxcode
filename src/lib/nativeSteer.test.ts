import { describe, expect, it } from "vitest";
import { canClearSteerDraft, prepareSteerAttempt, steerError } from "./nativeSteer";

describe("steer submission identity", () => {
  it("keeps the original UUID and expected turn after uncertain transport failure", () => {
    const first = prepareSteerAttempt(null, "session", "turn-1", "draft", ["image.png"]);
    const retry = prepareSteerAttempt(first, "session", "turn-2", "draft", ["image.png"]);
    expect(retry).toBe(first);
    expect(retry.turnId).toBe("turn-1");
    expect(
      prepareSteerAttempt(first, "session", "turn-2", "changed", ["image.png"]).inputId,
    ).not.toBe(first.inputId);
    expect(
      prepareSteerAttempt(first, "session", "turn-2", "draft", ["other.png"]).inputId,
    ).not.toBe(first.inputId);
  });

  it("distinguishes definitive rejection from transport or storage uncertainty", () => {
    expect(steerError({ kind: "rejected", message: "stale turn" })).toEqual({
      message: "stale turn",
      definitive: true,
    });
    expect(steerError({ kind: "unavailable", message: "disk unavailable" }).definitive).toBe(false);
    expect(steerError("transport closed").definitive).toBe(false);
  });

  it("late acceptance cannot clear a switched session, edited text or new attachment", () => {
    const submitted = { sessionId: "s1", text: "draft", paths: ["image.png"] };
    expect(canClearSteerDraft(submitted, { ...submitted })).toBe(true);
    expect(canClearSteerDraft(submitted, { ...submitted, sessionId: "s2" })).toBe(false);
    expect(canClearSteerDraft(submitted, { ...submitted, text: "new draft" })).toBe(false);
    expect(canClearSteerDraft(submitted, { ...submitted, paths: ["new.png"] })).toBe(false);
  });
});
