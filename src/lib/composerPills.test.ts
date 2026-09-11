import { describe, expect, it } from "vitest";
import {
  addFilePill,
  clearAllPills,
  clearTargetPill,
  hasPills,
  initialComposerPills,
  popLastPill,
  removeFilePill,
  removeTrailingTrigger,
  setTargetPill,
} from "./composerPills";

describe("composerPills", () => {
  it("initializes with null target and empty files", () => {
    const s = initialComposerPills();
    expect(s.target).toBeNull();
    expect(s.files).toEqual([]);
    expect(hasPills(s)).toBe(false);
  });

  it("sets and clears single target pill (mutually exclusive)", () => {
    let s = initialComposerPills();
    s = setTargetPill(s, {
      kind: "skill",
      name: "test-gen",
      description: "Generate unit tests",
      token: "$test-gen",
    });
    expect(s.target?.kind).toBe("skill");
    expect(s.target?.name).toBe("test-gen");
    expect(hasPills(s)).toBe(true);

    // Switching to subagent target replaces the skill
    s = setTargetPill(s, {
      kind: "subagent",
      id: "agent-1",
      name: "reviewer",
      description: "Code reviewer",
      token: "subagent:agent-1",
    });
    expect(s.target?.kind).toBe("subagent");
    expect(s.target?.name).toBe("reviewer");

    // Clear target
    s = clearTargetPill(s);
    expect(s.target).toBeNull();
    expect(hasPills(s)).toBe(false);
  });

  it("manages multi-select files without duplicates", () => {
    let s = initialComposerPills();
    s = addFilePill(s, "src/main.rs");
    s = addFilePill(s, "src/lib.rs");
    s = addFilePill(s, "src/main.rs"); // duplicate
    expect(s.files).toEqual(["src/main.rs", "src/lib.rs"]);

    s = removeFilePill(s, "src/main.rs");
    expect(s.files).toEqual(["src/lib.rs"]);

    s = clearAllPills(s);
    expect(s.files).toEqual([]);
  });

  it("pops last pill in reverse order (files first, then target)", () => {
    let s = initialComposerPills();
    s = setTargetPill(s, { kind: "command", name: "init", token: "/init" });
    s = addFilePill(s, "src/a.ts");
    s = addFilePill(s, "src/b.ts");

    s = popLastPill(s);
    expect(s.files).toEqual(["src/a.ts"]);
    expect(s.target?.name).toBe("init");

    s = popLastPill(s);
    expect(s.files).toEqual([]);
    expect(s.target?.name).toBe("init");

    s = popLastPill(s);
    expect(s.files).toEqual([]);
    expect(s.target).toBeNull();

    s = popLastPill(s);
    expect(s.files).toEqual([]);
    expect(s.target).toBeNull();
  });

  it("removes trailing trigger words from draft text", () => {
    expect(removeTrailingTrigger("hello @src")).toBe("hello");
    expect(removeTrailingTrigger("/init")).toBe("");
    expect(removeTrailingTrigger("use $rev")).toBe("use");
    expect(removeTrailingTrigger("plain text")).toBe("plain text");
    expect(removeTrailingTrigger("")).toBe("");
  });
});
