import { describe, expect, it } from "vitest";

import { isManagedWorktreePath } from "./worktreePath";

describe("isManagedWorktreePath", () => {
  it("matches the session-owned worktree path", () => {
    expect(isManagedWorktreePath("/cfg/worktrees/abc-1", "abc-1")).toBe(true);
    expect(isManagedWorktreePath("/home/u/.noxcode/worktrees/abc-1", "abc-1")).toBe(true);
  });

  it("rejects the main workspace and other sessions", () => {
    expect(isManagedWorktreePath("/repo", "abc-1")).toBe(false);
    expect(isManagedWorktreePath("/cfg/worktrees/other", "abc-1")).toBe(false);
    expect(isManagedWorktreePath("/cfg/worktrees/abc-1", "")).toBe(false);
  });
});
