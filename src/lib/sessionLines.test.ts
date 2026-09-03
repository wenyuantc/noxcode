import { describe, expect, it } from "vitest";

import { buildTurnBlocks, classifyLine, groupSessionLines, stripUserPrefix } from "./sessionLines";

describe("sessionLines", () => {
  it("classifies prefixes", () => {
    expect(classifyLine("[USER_INPUT] hello")).toBe("user");
    expect(classifyLine("[读取] README.md")).toBe("tool");
    expect(classifyLine("[工具结果]\nbody")).toBe("tool_result");
    expect(classifyLine("[思考] 已生成 12 字")).toBe("system");
    expect(classifyLine("最终汇报")).toBe("assistant");
    expect(classifyLine("[ERROR] boom")).toBe("error");
  });

  it("pairs tool results with the previous tool call", () => {
    const grouped = groupSessionLines([
      { id: "1", sessionId: "s", text: "[USER_INPUT] 读一下", createdAt: "1" },
      { id: "2", sessionId: "s", text: "[读取] README.md", createdAt: "2" },
      { id: "3", sessionId: "s", text: "[工具结果]\n# title\nmore", createdAt: "3" },
      { id: "4", sessionId: "s", text: "总结如下", createdAt: "4" },
    ]);
    expect(grouped).toHaveLength(3);
    expect(grouped[0]?.kind).toBe("user");
    expect(grouped[0]?.text).toBe("读一下");
    expect(grouped[1]?.kind).toBe("tool");
    expect(grouped[1]?.result).toBe("# title\nmore");
    expect(grouped[2]?.kind).toBe("assistant");
  });

  it("keeps multiline tool results", () => {
    expect(stripUserPrefix("[USER_INPUT]  a")).toBe("a");
  });

  it("groups a turn around the user line", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        { id: "1", sessionId: "s", text: "[USER_INPUT] 读一下", createdAt: "2026-01-01T00:00:00Z" },
        { id: "2", sessionId: "s", text: "[读取] README.md", createdAt: "2026-01-01T00:00:10Z" },
        {
          id: "3",
          sessionId: "s",
          text: "[工具结果]\n# title\nmore",
          createdAt: "2026-01-01T00:00:11Z",
        },
        { id: "4", sessionId: "s", text: "总结如下", createdAt: "2026-01-01T00:00:20Z" },
      ]),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.user?.text).toBe("读一下");
    expect(blocks[0]?.tools).toHaveLength(1);
    expect(blocks[0]?.tools[0]?.result).toBe("# title\nmore");
    expect(blocks[0]?.assistant).toHaveLength(1);
  });
});
