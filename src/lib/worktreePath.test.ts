import { describe, expect, it } from "vitest";

import { isManagedWorktreePath, relativeWorktreeFilePath } from "./worktreePath";

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

describe("relativeWorktreeFilePath", () => {
  it("strips the managed worktree prefix", () => {
    expect(
      relativeWorktreeFilePath(
        "/Users/me/Library/Application Support/app/worktrees/abc-1/oms/Test.java",
        "abc-1",
      ),
    ).toBe("oms/Test.java");
    expect(relativeWorktreeFilePath("src/main.rs", "abc-1")).toBe("src/main.rs");
  });
});
