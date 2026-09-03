import { describe, expect, it } from "vitest";

import { groupGitStatus, isStaged, isUntracked } from "./gitHelpers";
import type { GitStatus } from "./types";

const status: GitStatus = {
  branch: { oid: "a", head: "main", upstream: null, ahead: 0, behind: 0 },
  entries: [
    {
      kind: "ordinary",
      xy: "M ",
      path: "staged.ts",
      orig_path: null,
      score: null,
      mode_head: null,
      mode_index: null,
      mode_worktree: null,
    },
    {
      kind: "ordinary",
      xy: " M",
      path: "dirty.ts",
      orig_path: null,
      score: null,
      mode_head: null,
      mode_index: null,
      mode_worktree: null,
    },
    {
      kind: "untracked",
      xy: "?",
      path: "new.ts",
      orig_path: null,
      score: null,
      mode_head: null,
      mode_index: null,
      mode_worktree: null,
    },
  ],
};

describe("gitHelpers", () => {
  it("groups staged / unstaged / untracked", () => {
    const grouped = groupGitStatus(status);
    expect(grouped.staged.map((item) => item.path)).toEqual(["staged.ts"]);
    expect(grouped.unstaged.map((item) => item.path)).toEqual(["dirty.ts"]);
    expect(grouped.untracked.map((item) => item.path)).toEqual(["new.ts"]);
    expect(isStaged(status.entries[0]!)).toBe(true);
    expect(isUntracked(status.entries[2]!)).toBe(true);
  });
});
