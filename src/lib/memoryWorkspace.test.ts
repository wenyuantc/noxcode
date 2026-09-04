import { describe, expect, it } from "vitest";

import { resolveMemoryWorkspaceId } from "./memoryWorkspace";

describe("resolveMemoryWorkspaceId", () => {
  it("keeps the local selection when it is still in the list", () => {
    expect(resolveMemoryWorkspaceId("b", "a", ["a", "b", "c"])).toBe("b");
  });

  it("falls back to the active workspace when the local id is gone", () => {
    expect(resolveMemoryWorkspaceId("gone", "a", ["a", "b"])).toBe("a");
  });

  it("falls back to the first workspace when neither current nor active is valid", () => {
    expect(resolveMemoryWorkspaceId("gone", "also-gone", ["a", "b"])).toBe("a");
  });

  it("returns null when there are no workspaces", () => {
    expect(resolveMemoryWorkspaceId("a", "a", [])).toBeNull();
  });
});
