import { describe, expect, it } from "vitest";

import { fileRollbackChoices, summarizeRollbackPaths } from "./fileRollback";

describe("file rollback choices", () => {
  it("allows files only when the preview is available and has no conflicts", () => {
    expect(fileRollbackChoices({ available: true, conflicts: [] })).toEqual({
      conversation: true,
      files: true,
      both: true,
    });
    expect(fileRollbackChoices({ available: true, conflicts: ["a.txt"] })).toEqual({
      conversation: true,
      files: false,
      both: false,
    });
    expect(fileRollbackChoices({ available: false, conflicts: [] })).toEqual({
      conversation: true,
      files: false,
      both: false,
    });
  });

  it("summarizes a bounded path list", () => {
    expect(summarizeRollbackPaths("新增", ["a.txt", "b.txt"])).toBe("新增：a.txt、b.txt");
    expect(summarizeRollbackPaths("修改", ["1", "2", "3", "4", "5", "6", "7"], 2)).toBe(
      "修改：1、2 等 7 个",
    );
    expect(summarizeRollbackPaths("删除", [])).toBe("");
  });
});
