import { describe, expect, it } from "vitest";

import {
  buildTurnBlocks,
  changedFilesFromItems,
  classifyLine,
  commandText,
  displaySessionTitle,
  filePathText,
  groupSessionLines,
  latestTodos,
  parseAgentBanner,
  parseMcpStatus,
  parseReadResultLines,
  parseTodoList,
  parseUsageLine,
  permissionHint,
  stripAgentPrefix,
  stripUserPrefix,
  summarizeTools,
  thinkingText,
} from "./sessionLines";

function line(id: string, text: string, createdAt = id) {
  return { id, sessionId: "s", text, createdAt };
}

describe("sessionLines", () => {
  it("classifies prefixes", () => {
    expect(classifyLine("[USER_INPUT] hello")).toBe("user");
    expect(classifyLine("[读取] README.md")).toBe("tool");
    expect(classifyLine("[工具结果]\nbody")).toBe("tool_result");
    expect(classifyLine("[思考] 已生成 12 字")).toBe("system");
    expect(classifyLine("[思考]\n先看入口再改 Composer")).toBe("system");
    expect(classifyLine("最终汇报")).toBe("assistant");
    expect(classifyLine("[ERROR] boom")).toBe("error");
  });

  it("pairs tool results with the previous tool call", () => {
    const grouped = groupSessionLines([
      line("1", "[USER_INPUT] 读一下"),
      line("2", "[读取] README.md"),
      line("3", "[工具结果]\n# title\nmore"),
      line("4", "总结如下"),
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

  it("attaches leading status lines to the first user turn", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[PERMISSION] 已在设置中关闭高风险确认", "2026-01-01T00:00:00Z"),
        line("2", "[MCP] 未启用服务器", "2026-01-01T00:00:00Z"),
        line("3", "[USER_INPUT] 分析项目", "2026-01-01T00:00:01Z"),
        line("4", "好了", "2026-01-01T00:00:35Z"),
      ]),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.user?.text).toBe("分析项目");
    expect(blocks[0]?.startedAt).toBe("2026-01-01T00:00:01Z");
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["system", "assistant"]);
  });

  it("groups a turn around the user line", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 读一下", "2026-01-01T00:00:00Z"),
        line("2", "[读取] README.md", "2026-01-01T00:00:10Z"),
        line("3", "[工具结果]\n# title\nmore", "2026-01-01T00:00:11Z"),
        line("4", "总结如下", "2026-01-01T00:00:20Z"),
      ]),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.user?.text).toBe("读一下");
    expect(blocks[0]?.tools).toHaveLength(1);
    expect(blocks[0]?.tools[0]?.result).toBe("# title\nmore");
    expect(blocks[0]?.assistant).toHaveLength(1);
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["tools", "assistant"]);
  });

  it("keeps interleaved segment order", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 做一下", "2026-01-01T00:00:00Z"),
        line("2", "[思考] 已生成 8 字", "2026-01-01T00:00:01Z"),
        line("3", "[读取] a.ts", "2026-01-01T00:00:02Z"),
        line("4", "[命令] ls", "2026-01-01T00:00:03Z"),
        line("5", "[读取] b.ts", "2026-01-01T00:00:04Z"),
        line("6", "好了", "2026-01-01T00:00:05Z"),
      ]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual([
      "thinking",
      "tools",
      "terminal",
      "tools",
      "assistant",
    ]);
  });

  it("summarizes lookup tools only", () => {
    const items = groupSessionLines([
      line("1", "[读取] a.ts"),
      line("2", "[工具] Glob **/*.rs"),
      line("3", "[工具] Grep TODO"),
      line("4", "[命令] pwd"),
      line("5", "[写入] a.ts"),
      line("6", "[待办]\n- [pending] x (low)"),
    ]);
    expect(summarizeTools(items)).toEqual({ files: 1, lists: 1, searches: 1 });
  });

  it("splits read / command / read into separate segments", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 查"),
        line("2", "[读取] a.ts"),
        line("3", "[命令] cat a.ts"),
        line("4", "[读取] b.ts"),
      ]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual([
      "tools",
      "terminal",
      "tools",
    ]);
    expect(commandText(blocks[0]!.segments[1]!.items[0]!)).toBe("cat a.ts");
  });

  it("merges consecutive writes and edits into changes", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 改"),
        line("2", "[写入] src/a.ts"),
        line("3", "[编辑] src/b.ts"),
        line("4", "[补丁] src/c.ts"),
      ]),
    );
    expect(blocks[0]?.segments).toHaveLength(1);
    expect(blocks[0]?.segments[0]?.kind).toBe("changes");
    expect(blocks[0]?.segments[0]?.items).toHaveLength(3);
    expect(filePathText(blocks[0]!.segments[0]!.items[0]!)).toBe("src/a.ts");
  });

  it("keeps a single write as a file segment", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([line("1", "[USER_INPUT] 写"), line("2", "[写入] only.ts")]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["file"]);
  });

  it("parses a TodoWrite list and latest in-progress item", () => {
    const text = [
      "[待办]",
      "- [pending] 准备 (low)",
      "- [in_progress] 定位 TestController (high)",
      "- [pending] 实现 ok 接口 (medium)",
      "- [pending] 补测试 (low)",
      "- [pending] 收尾 (low)",
    ].join("\n");
    const parsed = parseTodoList(text);
    expect(parsed?.total).toBe(5);
    expect(parsed?.completed).toBe(0);
    expect(parsed?.current?.content).toBe("定位 TestController");
    expect(parsed?.current?.status).toBe("in_progress");
  });

  it("ignores todo placeholders and uses the last parseable list", () => {
    expect(parseTodoList("[待办] 读取任务清单")).toBeNull();
    expect(parseTodoList("[待办] (空)")).toBeNull();
    expect(parseTodoList("[待办] 更新任务清单")).toBeNull();
    const items = groupSessionLines([
      line("1", "[待办] 读取任务清单"),
      line("2", "[待办]\n- [completed] 旧任务 (low)\n- [pending] 还没做 (medium)"),
      line(
        "3",
        "[待办]\n- [completed] a (low)\n- [completed] b (low)\n- [in_progress] 当前项 (high)\n- [pending] d (low)\n- [pending] e (low)",
      ),
    ]);
    const latest = latestTodos(items);
    expect(latest?.total).toBe(5);
    expect(latest?.completed).toBe(2);
    expect(latest?.current?.content).toBe("当前项");
    expect(
      latestTodos([
        line("1", "[待办] 读取任务清单"),
        line("2", "[待办]\n- [completed] a (low)\n- [in_progress] 当前项 (high)"),
      ])?.current?.content,
    ).toBe("当前项");
  });

  it("strips permission hints", () => {
    expect(permissionHint("[PERMISSION] 已在设置中关闭高风险确认，本会话工具将直接执行")).toBe(
      "已在设置中关闭高风险确认，本会话工具将直接执行",
    );
    expect(
      permissionHint(
        "[PERMISSION] 已开启自动编辑：覆盖文件直接执行，删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍需确认",
      ),
    ).toBe("已开启自动编辑：覆盖文件直接执行，删除 / 推送 / 强制 Git / 不透明命令 / MCP 仍需确认");
    expect(permissionHint("[PERMISSION] 等待确认高风险操作（local / Shell）：rm -rf /")).toBe(
      "等待确认高风险操作（local / Shell）：rm -rf /",
    );
    expect(permissionHint("[PERMISSION] 确认超时，已按拒绝处理")).toBe("确认超时，已按拒绝处理");
    expect(permissionHint("[MCP] 未启用服务器")).toBeNull();
  });

  it("parses the agent startup banner and leaves other agent lines", () => {
    expect(
      parseAgentBanner(
        "[内置 Agent] 启动会话 渠道=Mai-grok 协议=openai model=grok-4.6 effort=medium thinking=on",
      ),
    ).toEqual({
      channel: "Mai-grok",
      protocol: "openai",
      model: "grok-4.6",
      effort: "medium",
      thinking: true,
    });
    expect(
      parseAgentBanner(
        "[内置 Agent] 启动会话 渠道=DeepSeek 协议=openai model=deepseek-v4-flash effort=默认 thinking=off",
      ),
    ).toEqual({
      channel: "DeepSeek",
      protocol: "openai",
      model: "deepseek-v4-flash",
      effort: "默认",
      thinking: false,
    });
    expect(parseAgentBanner("[内置 Agent] 已停止")).toBeNull();
    expect(stripAgentPrefix("[内置 Agent] 已停止")).toBe("已停止");
  });

  it("parses MCP status lines", () => {
    expect(parseMcpStatus("[MCP] 未启用服务器")).toEqual({ kind: "off" });
    expect(parseMcpStatus("[MCP] 已连接：a、b")).toEqual({ kind: "on", servers: ["a", "b"] });
    expect(parseMcpStatus("[MCP] 将连接 3 个已启用服务器")).toEqual({ kind: "pending", count: 3 });
    expect(parseMcpStatus("[MCP] 没有成功连接的服务器")).toEqual({
      kind: "error",
      detail: "没有成功连接的服务器",
    });
    expect(parseMcpStatus("[MCP] 无法连接 files：timeout（已跳过，不回退到其他位置）")).toEqual({
      kind: "error",
      detail: "无法连接 files：timeout（已跳过，不回退到其他位置）",
    });
    expect(parseMcpStatus("[MCP] 握手失败 git：boom（已跳过）")).toEqual({
      kind: "error",
      detail: "握手失败 git：boom（已跳过）",
    });
    expect(parseMcpStatus("[MCP] 读取配置失败：bad json")).toEqual({
      kind: "error",
      detail: "读取配置失败：bad json",
    });
    expect(parseMcpStatus("[MCP] SSH 会话将在远端拉起 MCP，失败不回退本机")).toEqual({
      kind: "info",
      detail: "SSH 会话将在远端拉起 MCP，失败不回退本机",
    });
    expect(parseMcpStatus("[PERMISSION] 确认超时，已按拒绝处理")).toBeNull();
  });

  it("parses usage lines into chips", () => {
    expect(parseUsageLine("[用量] in=4399 out=123 cache=3200 total=4522")).toEqual({
      input: 4399,
      output: 123,
      cache: 3200,
      total: 4522,
    });
    expect(parseUsageLine("[用量] in=10 out=4 reason=12 total=14")).toEqual({
      input: 10,
      output: 4,
      reasoning: 12,
      total: 14,
    });
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 问", "2026-01-01T00:00:00Z"),
        line("2", "[用量] in=10 out=4 total=14", "2026-01-01T00:00:01Z"),
        line("3", "好了", "2026-01-01T00:00:02Z"),
      ]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["usage", "assistant"]);
  });

  it("parses read result line numbers", () => {
    expect(parseReadResultLines("     1\t# title\n     2\tbody")).toEqual([
      { line: 1, text: "# title" },
      { line: 2, text: "body" },
    ]);
  });

  it("collects agent-changed files and dedupes", () => {
    const items = groupSessionLines([
      line("1", "[写入] src/a.ts"),
      line("2", "[编辑] src/a.ts"),
      line("3", "[编辑] src/b.ts"),
      line("4", "[补丁] 应用多文件补丁"),
    ]);
    items[3]!.result = "3 files\nwrote src/c.ts\ndeleted src/d.ts";
    expect(changedFilesFromItems(items)).toEqual(["src/a.ts", "src/b.ts", "src/c.ts", "src/d.ts"]);
  });

  it("keeps full thinking body after the prefix", () => {
    const items = groupSessionLines([line("1", "[思考]\n先看入口再改 Composer")]);
    expect(thinkingText(items)).toBe("先看入口再改 Composer");
  });

  it("truncates display titles to 30 characters", () => {
    expect(displaySessionTitle("  hello  ")).toBe("hello");
    expect(
      displaySessionTitle("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十超出"),
    ).toBe("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十");
  });
});
