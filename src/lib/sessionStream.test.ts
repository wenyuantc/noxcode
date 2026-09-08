import { describe, expect, it } from "vitest";

import { thinkingDurationSeconds, type SessionTurnBlock } from "./sessionLines";
import {
  applyTextDelta,
  attachLiveFragments,
  pruneCoveredFragments,
  turnSegmentReactKey,
  type SessionStreamFragment,
} from "./sessionStream";

const NOW = "2026-01-01T00:00:00.000Z";
const LATER = "2026-01-01T00:00:10.000Z";

function block(partial?: Partial<SessionTurnBlock>): SessionTurnBlock {
  return {
    id: "turn-1",
    tools: [],
    assistant: [],
    system: [],
    segments: [],
    startedAt: NOW,
    endedAt: NOW,
    ...partial,
  };
}

function reasoning(text = "先看入口"): SessionStreamFragment {
  return { id: "reasoning-0", kind: "reasoning", text, startedAt: NOW };
}

describe("applyTextDelta", () => {
  it("appends the same kind and keeps id and startedAt", () => {
    const first = applyTextDelta([], { kind: "reasoning", text: "先看", clear: false }, NOW);
    const next = applyTextDelta(first, { kind: "reasoning", text: "入口", clear: false }, LATER);
    expect(next).toEqual([
      { id: "reasoning-0", kind: "reasoning", text: "先看入口", startedAt: NOW },
    ]);
  });

  it("keeps reasoning when text starts instead of replacing it", () => {
    const reasoningParts = applyTextDelta(
      [],
      { kind: "reasoning", text: "思考", clear: false },
      NOW,
    );
    const next = applyTextDelta(
      reasoningParts,
      { kind: "text", text: "正文", clear: false },
      LATER,
    );
    expect(next.map((part) => part.kind)).toEqual(["reasoning", "text"]);
    expect(next.map((part) => part.text)).toEqual(["思考", "正文"]);
  });

  it("clears fragments on retry reset", () => {
    const parts = applyTextDelta([], { kind: "reasoning", text: "旧", clear: false }, NOW);
    expect(applyTextDelta(parts, { kind: "text", text: "", clear: true }, LATER)).toEqual([]);
  });
});

describe("attachLiveFragments", () => {
  it("attaches reasoning then text in order", () => {
    const view = attachLiveFragments(block(), [
      reasoning("思考"),
      { id: "text-1", kind: "text", text: "正文", startedAt: LATER },
    ]);
    expect(view.segments.map((segment) => segment.kind)).toEqual(["thinking", "assistant"]);
    expect(view.segments[0]?.items[0]?.id).toBe("reasoning-0");
    expect(view.segments[0]?.items[0]?.createdAt).toBe(NOW);
  });

  it("does not duplicate a persisted thinking line", () => {
    const persisted = block({
      segments: [
        {
          kind: "thinking",
          items: [
            {
              id: "event-1",
              kind: "system",
              text: "[思考] 8秒\n先看入口",
              createdAt: NOW,
            },
          ],
        },
      ],
    });
    const view = attachLiveFragments(persisted, [reasoning("先看入口")]);
    expect(view).toBe(persisted);
    expect(turnSegmentReactKey("turn-1", persisted.segments[0]!, 0, persisted.segments)).toBe(
      "turn-1:thinking:0",
    );
  });

  it("keeps the thinking segment key after the event id replaces the live id", () => {
    const live = attachLiveFragments(block(), [reasoning("先看入口")]);
    const persisted = block({
      segments: [
        {
          kind: "thinking",
          items: [
            {
              id: "event-1",
              kind: "system",
              text: "[思考] 8秒\n先看入口",
              createdAt: NOW,
            },
          ],
        },
      ],
    });
    expect(turnSegmentReactKey(live.id, live.segments[0]!, 0, live.segments)).toBe(
      turnSegmentReactKey(persisted.id, persisted.segments[0]!, 0, persisted.segments),
    );
  });

  it("uses startedAt so thinking duration grows with nowMs", () => {
    const view = attachLiveFragments(block(), [reasoning("思考")]);
    expect(thinkingDurationSeconds(view.segments[0]!.items, Date.parse(NOW))).toBe(0);
    expect(thinkingDurationSeconds(view.segments[0]!.items, Date.parse(LATER))).toBe(10);
  });
});

describe("pruneCoveredFragments", () => {
  it("drops reasoning after a persisted thinking line and text after assistant output", () => {
    const parts = [
      reasoning("先看入口"),
      { id: "text-1", kind: "text", text: "正文", startedAt: LATER },
    ];
    const afterThink = pruneCoveredFragments(parts, "[思考] 8秒\n先看入口");
    expect(afterThink.map((part) => part.kind)).toEqual(["text"]);
    expect(pruneCoveredFragments(afterThink, "正文")).toEqual([]);
  });
});
