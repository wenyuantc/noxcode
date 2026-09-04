import { describe, expect, it } from "vitest";

import { groupGitStatus, isStaged, isUntracked } from "./gitHelpers";
import type { GitStatus, GitStatusEntry } from "./types";

function entry(
  path: string,
  xy: string,
  kind: GitStatusEntry["kind"] = "ordinary",
): GitStatusEntry {
  return {
    kind,
    xy,
    path,
    orig_path: null,
    score: null,
    mode_head: null,
    mode_index: null,
    mode_worktree: null,
  };
}

const status: GitStatus = {
  branch: { oid: "a", head: "main", upstream: null, ahead: 0, behind: 0 },
  entries: [entry("staged.ts", "M "), entry("dirty.ts", " M"), entry("new.ts", "?", "untracked")],
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

  it("treats porcelain v2 dot as unmodified", () => {
    const grouped = groupGitStatus({
      branch: status.branch,
      entries: [
        entry("staged-only.ts", "M."),
        entry("unstaged-only.ts", ".M"),
        entry("both.ts", "MM"),
      ],
    });
    expect(grouped.staged.map((item) => item.path)).toEqual(["staged-only.ts", "both.ts"]);
    expect(grouped.unstaged.map((item) => item.path)).toEqual(["unstaged-only.ts", "both.ts"]);
    expect(grouped.untracked).toEqual([]);
  });
});
