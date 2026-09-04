import { describe, expect, it } from "vitest";

import { clampMentionIndex, resolveComposerMentionKey } from "./composerMention";

const base = {
  shiftKey: false,
  isComposing: false,
  mentionVisible: true,
  itemCount: 5,
  activeIndex: 2,
};

describe("clampMentionIndex", () => {
  it("clamps to a valid row", () => {
    expect(clampMentionIndex(0, 0)).toBe(0);
    expect(clampMentionIndex(4, 0)).toBe(0);
    expect(clampMentionIndex(-1, 3)).toBe(0);
    expect(clampMentionIndex(0, 3)).toBe(0);
    expect(clampMentionIndex(2, 3)).toBe(2);
    expect(clampMentionIndex(3, 3)).toBe(2);
  });
});

describe("resolveComposerMentionKey", () => {
  it("ignores keys while IME is composing", () => {
    expect(resolveComposerMentionKey({ ...base, key: "Enter", isComposing: true })).toEqual({
      type: "none",
    });
    expect(resolveComposerMentionKey({ ...base, key: "ArrowDown", isComposing: true })).toEqual({
      type: "none",
    });
    expect(resolveComposerMentionKey({ ...base, key: "Tab", isComposing: true })).toEqual({
      type: "none",
    });
  });

  it("moves the highlight and clamps at the ends", () => {
    expect(resolveComposerMentionKey({ ...base, key: "ArrowDown" })).toEqual({
      type: "move",
      nextIndex: 3,
    });
    expect(resolveComposerMentionKey({ ...base, key: "ArrowDown", activeIndex: 4 })).toEqual({
      type: "move",
      nextIndex: 4,
    });
    expect(resolveComposerMentionKey({ ...base, key: "ArrowUp" })).toEqual({
      type: "move",
      nextIndex: 1,
    });
    expect(resolveComposerMentionKey({ ...base, key: "ArrowUp", activeIndex: 0 })).toEqual({
      type: "move",
      nextIndex: 0,
    });
    expect(resolveComposerMentionKey({ ...base, key: "Down" })).toEqual({
      type: "move",
      nextIndex: 3,
    });
    expect(resolveComposerMentionKey({ ...base, key: "Up" })).toEqual({
      type: "move",
      nextIndex: 1,
    });
  });

  it("confirms the highlighted item with Enter or Tab", () => {
    expect(resolveComposerMentionKey({ ...base, key: "Enter" })).toEqual({ type: "confirm" });
    expect(resolveComposerMentionKey({ ...base, key: "Tab" })).toEqual({ type: "confirm" });
  });

  it("lets Shift+Enter insert a newline while the list is open", () => {
    expect(resolveComposerMentionKey({ ...base, key: "Enter", shiftKey: true })).toEqual({
      type: "none",
    });
  });

  it("dismisses the list with Escape", () => {
    expect(resolveComposerMentionKey({ ...base, key: "Escape" })).toEqual({ type: "dismiss" });
  });

  it("toggles plan mode with Shift+Tab even while the list is open", () => {
    expect(resolveComposerMentionKey({ ...base, key: "Tab", shiftKey: true })).toEqual({
      type: "togglePlanMode",
    });
    expect(
      resolveComposerMentionKey({
        ...base,
        key: "Tab",
        shiftKey: true,
        mentionVisible: false,
        itemCount: 0,
      }),
    ).toEqual({ type: "togglePlanMode" });
  });

  it("sends when the list is closed or empty", () => {
    expect(
      resolveComposerMentionKey({
        ...base,
        key: "Enter",
        mentionVisible: false,
        itemCount: 0,
      }),
    ).toEqual({ type: "send" });
    expect(
      resolveComposerMentionKey({
        ...base,
        key: "Enter",
        mentionVisible: true,
        itemCount: 0,
      }),
    ).toEqual({ type: "send" });
  });

  it("does not steal Tab when the list is closed", () => {
    expect(
      resolveComposerMentionKey({
        ...base,
        key: "Tab",
        mentionVisible: false,
        itemCount: 0,
      }),
    ).toEqual({ type: "none" });
  });
});
