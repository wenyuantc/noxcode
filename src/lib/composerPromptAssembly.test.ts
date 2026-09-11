import { describe, expect, it } from "vitest";
import {
  assembleComposerPrompt,
  buildFilesContextPrefix,
  combineFilesWithUserPrompt,
} from "./composerPromptAssembly";
import { initialComposerPills } from "./composerPills";

describe("composerPromptAssembly", () => {
  it("formats file context prefixes correctly", () => {
    expect(buildFilesContextPrefix([])).toBe("");
    expect(buildFilesContextPrefix(["src/main.rs"])).toBe("@src/main.rs");
    expect(buildFilesContextPrefix(["src/main.rs", "src/lib.rs"])).toBe("@src/main.rs @src/lib.rs");
  });

  it("combines files with user text", () => {
    expect(combineFilesWithUserPrompt([], "hello world")).toBe("hello world");
    expect(combineFilesWithUserPrompt(["src/a.ts"], "")).toBe("@src/a.ts");
    expect(combineFilesWithUserPrompt(["src/a.ts"], "find bugs")).toBe("@src/a.ts\n\nfind bugs");
  });

  it("assembles subagent prompt with files and user input", () => {
    const pills = {
      target: {
        kind: "subagent" as const,
        id: "sub-123",
        name: "SecurityReviewer",
        token: "subagent:sub-123",
      },
      files: ["src/auth.ts"],
    };
    const result = assembleComposerPrompt(pills, "检查是否有越权漏洞");
    expect(result.intent.type).toBe("plain");
    expect(result.prompt).toContain("SecurityReviewer");
    expect(result.prompt).toContain("sub-123");
    expect(result.prompt).toContain("@src/auth.ts");
    expect(result.prompt).toContain("检查是否有越权漏洞");
  });

  it("assembles skill prompt with files and user input", () => {
    const pills = {
      target: {
        kind: "skill" as const,
        name: "unit-test",
        token: "$unit-test",
      },
      files: ["src/calc.ts"],
    };
    const result = assembleComposerPrompt(pills, "补充边界用例");
    expect(result.intent.type).toBe("skill");
    expect(result.intent).toHaveProperty("name", "unit-test");
    expect(result.prompt).toContain("unit-test");
    expect(result.prompt).toContain("@src/calc.ts");
    expect(result.prompt).toContain("补充边界用例");
  });

  it("assembles slash command pill (e.g. init)", () => {
    const pills = {
      target: {
        kind: "command" as const,
        name: "init",
        token: "/init",
      },
      files: [],
    };
    const result = assembleComposerPrompt(pills, "侧重后端架构");
    expect(result.intent.type).toBe("expand");
    expect(result.prompt).toBe("/init 侧重后端架构");
  });

  it("handles plain prompt with only files", () => {
    const pills = {
      target: null,
      files: ["src/index.ts", "package.json"],
    };
    const result = assembleComposerPrompt(pills, "总结依赖");
    expect(result.intent.type).toBe("plain");
    expect(result.prompt).toBe("@src/index.ts @package.json\n\n总结依赖");
  });

  it("handles raw prompt with no pills", () => {
    const pills = initialComposerPills();
    const result = assembleComposerPrompt(pills, "你好");
    expect(result.intent.type).toBe("plain");
    expect(result.prompt).toBe("你好");
  });
});
