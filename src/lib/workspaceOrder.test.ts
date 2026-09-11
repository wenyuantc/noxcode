import { describe, expect, it } from "vitest";

import {
  applyWorkspaceOrder,
  moveWorkspaceTo,
  workspaceDropIndex,
  workspaceIndicatorOffset,
  workspaceMoveTarget,
} from "./workspaceOrder";

function item(id: string) {
  return { id, name: id };
}

function ids(items: { id: string }[]): string[] {
  return items.map((entry) => entry.id);
}

describe("applyWorkspaceOrder", () => {
  it("returns the input array when no order is persisted", () => {
    const items = [item("a"), item("b")];
    expect(applyWorkspaceOrder(items, [])).toBe(items);
  });

  it("returns the input array when the persisted order already matches", () => {
    const items = [item("b"), item("a")];
    expect(applyWorkspaceOrder(items, ["b", "a"])).toBe(items);
  });

  it("reorders items to the persisted order", () => {
    const items = [item("a"), item("b"), item("c")];
    expect(ids(applyWorkspaceOrder(items, ["c", "a", "b"]))).toEqual(["c", "a", "b"]);
  });

  it("drops persisted ids that no longer exist", () => {
    const items = [item("a"), item("b")];
    expect(ids(applyWorkspaceOrder(items, ["gone", "b", "a"]))).toEqual(["b", "a"]);
  });

  it("appends unknown items after the known ones, keeping their incoming order", () => {
    const items = [item("new-1"), item("a"), item("new-2"), item("b")];
    expect(ids(applyWorkspaceOrder(items, ["b", "a"]))).toEqual(["b", "a", "new-1", "new-2"]);
  });
});

describe("moveWorkspaceTo", () => {
  it("moves an item to the front", () => {
    expect(ids(moveWorkspaceTo([item("a"), item("b"), item("c")], "c", 0))).toEqual([
      "c",
      "a",
      "b",
    ]);
  });

  it("moves an item to the back", () => {
    expect(ids(moveWorkspaceTo([item("a"), item("b"), item("c")], "a", 3))).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("inserts before and after a middle row", () => {
    expect(ids(moveWorkspaceTo([item("a"), item("b"), item("c")], "a", 2))).toEqual([
      "b",
      "a",
      "c",
    ]);
    expect(ids(moveWorkspaceTo([item("a"), item("b"), item("c")], "a", 3))).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("returns the input array for no-op drops around the item's own slot", () => {
    const items = [item("a"), item("b"), item("c")];
    expect(moveWorkspaceTo(items, "b", 1)).toBe(items);
    expect(moveWorkspaceTo(items, "b", 2)).toBe(items);
  });

  it("returns the input array for an unknown id or a non-finite index", () => {
    const items = [item("a"), item("b")];
    expect(moveWorkspaceTo(items, "missing", 0)).toBe(items);
    expect(moveWorkspaceTo(items, "a", Number.NaN)).toBe(items);
    expect(moveWorkspaceTo(items, "a", Number.POSITIVE_INFINITY)).toBe(items);
  });

  it("clamps an out-of-range index to the list bounds", () => {
    expect(ids(moveWorkspaceTo([item("a"), item("b")], "a", 99))).toEqual(["b", "a"]);
    expect(ids(moveWorkspaceTo([item("a"), item("b")], "b", -5))).toEqual(["b", "a"]);
  });
});

describe("workspaceMoveTarget", () => {
  it("targets the previous slot when moving up and the next one when moving down", () => {
    const items = [item("a"), item("b"), item("c")];
    expect(workspaceMoveTarget(items, "b", "up")).toBe(0);
    expect(workspaceMoveTarget(items, "b", "down")).toBe(3);
  });

  it("returns null at the boundaries", () => {
    const items = [item("a"), item("b")];
    expect(workspaceMoveTarget(items, "a", "up")).toBeNull();
    expect(workspaceMoveTarget(items, "b", "down")).toBeNull();
    expect(workspaceMoveTarget(items, "missing", "up")).toBeNull();
  });
});

describe("workspaceDropIndex", () => {
  const rows = [
    { id: "a", top: 0, bottom: 30 },
    { id: "b", top: 40, bottom: 70 },
  ];

  it("inserts at the top above the first row and at the bottom below the last", () => {
    expect(workspaceDropIndex(rows, -10)).toBe(0);
    expect(workspaceDropIndex(rows, 500)).toBe(2);
  });

  it("splits each row at its vertical middle", () => {
    expect(workspaceDropIndex(rows, 5)).toBe(0);
    expect(workspaceDropIndex(rows, 25)).toBe(1);
    expect(workspaceDropIndex(rows, 45)).toBe(1);
    expect(workspaceDropIndex(rows, 65)).toBe(2);
  });

  it("resolves the gap between rows to the slot before the following row", () => {
    expect(workspaceDropIndex(rows, 35)).toBe(1);
  });

  it("returns zero for an empty list", () => {
    expect(workspaceDropIndex([], 120)).toBe(0);
  });
});

describe("workspaceIndicatorOffset", () => {
  const rows = [
    { id: "a", top: 100, bottom: 130 },
    { id: "b", top: 140, bottom: 170 },
  ];

  it("anchors the line to the top edge of the row at the index", () => {
    expect(workspaceIndicatorOffset({ rows, index: 0, containerTop: 90, scrollTop: 0 })).toBe(10);
    expect(workspaceIndicatorOffset({ rows, index: 1, containerTop: 90, scrollTop: 0 })).toBe(50);
  });

  it("anchors past-the-end drops to the bottom edge of the last row", () => {
    expect(workspaceIndicatorOffset({ rows, index: 2, containerTop: 90, scrollTop: 0 })).toBe(80);
    expect(workspaceIndicatorOffset({ rows, index: 99, containerTop: 90, scrollTop: 0 })).toBe(80);
  });

  it("prefers the last block bottom for a trailing drop, so it lands after its sessions", () => {
    expect(
      workspaceIndicatorOffset({
        rows,
        index: 2,
        containerTop: 90,
        scrollTop: 0,
        tailBottom: 260,
      }),
    ).toBe(170);
    // A mid-list drop keeps using the row's own top edge.
    expect(
      workspaceIndicatorOffset({ rows, index: 1, containerTop: 90, scrollTop: 0, tailBottom: 260 }),
    ).toBe(50);
  });

  it("adds the scroll offset so a scrolled list keeps the same visual line", () => {
    expect(workspaceIndicatorOffset({ rows, index: 0, containerTop: 90, scrollTop: 240 })).toBe(
      250,
    );
  });

  it("returns null without measurable rows", () => {
    expect(
      workspaceIndicatorOffset({ rows: [], index: 0, containerTop: 0, scrollTop: 0 }),
    ).toBeNull();
  });
});
