import { describe, expect, it } from "vitest";

import { diffGutterWidth, parseDiffLineNumbers } from "./diffLineNumbers";

const PATCH = `diff --git a/src/hi.ts b/src/hi.ts
index 111..222 100644
--- a/src/hi.ts
+++ b/src/hi.ts
@@ -1,3 +1,4 @@
 keep
-removed
+added
+also
 context
`;

describe("diffLineNumbers", () => {
  it("assigns old and new numbers from hunk headers", () => {
    const lines = parseDiffLineNumbers(PATCH);
    expect(lines.map((line) => [line.type, line.oldLine, line.newLine])).toEqual([
      ["meta", null, null],
      ["meta", null, null],
      ["meta", null, null],
      ["meta", null, null],
      ["hunk", null, null],
      ["ctx", 1, 1],
      ["del", 2, null],
      ["add", null, 2],
      ["add", null, 3],
      ["ctx", 3, 4],
      ["ctx", 4, 5],
    ]);
  });

  it("sizes the gutter from the largest visible line number", () => {
    expect(diffGutterWidth(parseDiffLineNumbers(PATCH))).toBe(2);
    expect(diffGutterWidth([{ type: "ctx", text: "x", oldLine: 8, newLine: 120 }])).toBe(3);
  });
});
