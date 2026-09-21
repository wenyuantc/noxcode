import { describe, expect, it } from "vitest";
import {
  assistantItemsText,
  buildTurnBlocks,
  groupSessionLines,
  hydrateSessionLine,
} from "./sessionLines";
import { applyTextDelta, attachLiveFragments } from "./sessionStream";

const meta = (part: number, chain_id = "chain-a") => ({ chain_id, part });
function raw(id: string, text: string, part?: number) {
  return {
    id,
    sessionId: "s",
    text,
    createdAt: id,
    assistant: part == null ? undefined : meta(part),
  };
}
function assistantText(block: ReturnType<typeof buildTurnBlocks>[number]) {
  return block.segments
    .filter((s) => s.kind === "assistant")
    .map((s) => s.items.map((i) => i.text).join("\n\n"));
}

describe("explicit assistant continuation", () => {
  it("coalesces committed fragments across reasoning and usage without modifying bytes", () => {
    for (const parts of [
      ["hel", "lo"],
      ["```ts\nconst x = ", "1;\n```"],
    ]) {
      const lines = [
        raw("0", "[USER_INPUT] go"),
        raw("1", parts[0], 0),
        raw("2", "[思考]\nchecking"),
        raw("3", "[用量] 输入 1 | 输出 1"),
        raw("4", parts[1], 1),
      ];
      const block = buildTurnBlocks(groupSessionLines(lines))[0];
      expect(assistantText(block)).toEqual([parts.join("")]);
      expect(assistantItemsText(block.assistant)).toBe(parts.join(""));
      expect(block.endedAt).toBe("4");
    }
  });

  it("coalesces a live suffix with its persisted prefix, keeping repeated text", () => {
    const base = buildTurnBlocks(groupSessionLines([raw("1", "ha", 0)]))[0];
    const deltas = applyTextDelta(
      [],
      { kind: "text", text: "ha", clear: false, assistant: meta(1) },
      "now",
    );
    const live = attachLiveFragments(base, deltas);
    expect(assistantText(live)).toEqual(["haha"]);
    expect(live.assistant.map((i) => i.text).join("\n\n")).toBe("haha");
  });

  it("retains continuation metadata after persisted envelope hydration", () => {
    const replay = hydrateSessionLine(
      raw("1", JSON.stringify({ nox: 1, line: "hel", assistant: meta(0) })),
    );
    expect(replay).toMatchObject({ text: "hel", assistant: meta(0) });
  });

  it("keeps unrelated model replies separated", () => {
    const lines = [raw("1", "one", 0), { ...raw("2", "two", 0), assistant: meta(0, "chain-b") }];
    const block = buildTurnBlocks(groupSessionLines(lines))[0];
    expect(block.assistant.map((i) => i.text).join("\n\n")).toBe("one\n\ntwo");
  });
});

it("keeps code fences continuous during reasoning deltas and commits exact suffixes", () => {
  const prefix = "```ts\nconst x = ";
  const base = buildTurnBlocks(
    groupSessionLines([raw("1", prefix, 0), raw("2", "[用量] 输入 1 | 输出 1")]),
  )[0];
  let live = applyTextDelta([], { kind: "reasoning", text: "checking", clear: false }, "3");
  live = applyTextDelta(live, { kind: "text", text: "1;", clear: false, assistant: meta(1) }, "4");
  live = applyTextDelta(live, { kind: "reasoning", text: "done", clear: false }, "5");
  live = applyTextDelta(
    live,
    { kind: "text", text: "\n```", clear: false, assistant: meta(1) },
    "6",
  );
  const attached = attachLiveFragments(base, live);
  expect(assistantText(attached)).toEqual([`${prefix}1;\n\`\`\``]);
  const replay = buildTurnBlocks(
    groupSessionLines([raw("1", prefix, 0), raw("2", "[思考]\nchecking"), raw("6", "1;\n```", 1)]),
  )[0];
  expect(assistantText(replay)).toEqual(assistantText(attached));
});

it("does not interpret explicitly identified assistant fragment contents as log prefixes or envelopes", () => {
  const text = JSON.stringify({ nox: 1, line: "must stay JSON" });
  const source = [raw("1", "[用量] ", 0), raw("2", text, 1), raw("3", " ", 2)];
  const block = buildTurnBlocks(groupSessionLines(source))[0];
  expect(assistantText(block)).toEqual([`[用量] ${text} `]);
});
