import { describe, expect, it } from "vitest";

import {
  buildTurnBlocks,
  changedFilesFromItems,
  classifyLine,
  commandText,
  displaySessionTitle,
  filePathText,
  formatSessionDuration,
  groupSessionLines,
  isHiddenSessionCeremonyLine,
  latestTodos,
  parseAgentBanner,
  parseCompactBoundary,
  parseMcpStatus,
  parseReadResultLines,
  parseTodoList,
  parseUsageLine,
  permissionHint,
  stripAgentPrefix,
  stripUserPrefix,
  summarizeTools,
  parseThinkingDurationSeconds,
  parseRetryLine,
  parsePlanLine,
  summarizeRetry,
  thinkingDurationSeconds,
  thinkingText,
  workDurationSeconds,
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
    expect(classifyLine("[重试] 模型请求失败（HTTP 429）: {}, 3 秒后进行第 1/10 次重试")).toBe(
      "system",
    );
    expect(
      classifyLine(
        "[子 Agent 1(general) - 改文件] [重试] 模型请求失败（HTTP 502）: x，3 秒后进行第 1/10 次重试",
      ),
    ).toBe("system");
    expect(classifyLine("[子 Agent 1(general) - 改文件] [读取] a.ts")).toBe("tool");
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

  it("parses compact boundaries into their own segment", () => {
    const boundaryLine =
      '[COMPACT_BOUNDARY] {"trigger":"manual","source":"model","pre_tokens":120000,"post_tokens":30000,"pre_messages":80,"post_messages":12,"instructions":"keep stacks"}';
    const parsed = parseCompactBoundary(boundaryLine);
    expect(parsed?.trigger).toBe("manual");
    expect(parsed?.source).toBe("model");
    expect(parsed?.pre_tokens).toBe(120000);
    expect(parsed?.instructions).toBe("keep stacks");
    expect(parseCompactBoundary("[工具] 已压缩上下文")).toBeNull();
    expect(parseCompactBoundary("[COMPACT_BOUNDARY] not json")).toBeNull();
    expect(classifyLine(boundaryLine)).toBe("system");
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 继续"),
        line("2", "[工具] 已压缩上下文（模型摘要）：120000 → 30000 token（manual）"),
        line("3", boundaryLine),
        line("4", "继续工作"),
      ]),
    );
    const kinds = blocks[0]?.segments.map((segment) => segment.kind);
    // `[工具] 已压缩…` 仍按工具行展示，边界行独立成段。
    expect(kinds).toEqual(["tools", "compact", "assistant"]);
    expect(blocks[0]?.segments[1]?.items).toHaveLength(1);
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

  it("hides historical resume ceremony lines", () => {
    expect(isHiddenSessionCeremonyLine("[续聊] 已恢复上一会话 2 条上下文（图片附件不恢复）")).toBe(
      true,
    );
    expect(isHiddenSessionCeremonyLine("内置 Agent 会话已恢复")).toBe(true);
    expect(isHiddenSessionCeremonyLine("内置 Agent 会话已创建")).toBe(true);
    expect(isHiddenSessionCeremonyLine("[ERROR] 已取消")).toBe(true);
    expect(isHiddenSessionCeremonyLine("[ERROR] boom")).toBe(false);
    expect(isHiddenSessionCeremonyLine("[MCP] 未启用服务器")).toBe(false);
    const grouped = groupSessionLines([
      line("1", "[USER_INPUT] 你好", "2026-01-01T00:00:00Z"),
      line("2", "[续聊] 已恢复上一会话 2 条上下文（图片附件不恢复）", "2026-09-03T07:00:00Z"),
      line("3", "内置 Agent 会话已恢复", "2026-09-03T07:00:01Z"),
      line("4", "内置 Agent 会话已创建", "2026-09-03T07:00:02Z"),
      line("5", "[ERROR] 已取消", "2026-09-03T07:00:03Z"),
      line("6", "先这样", "2026-01-01T00:00:10Z"),
    ]);
    expect(grouped.map((item) => item.text)).toEqual(["你好", "先这样"]);
  });

  it("starts a new turn when resume banners follow a completed user turn", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 你好你能做什么", "2026-01-01T00:00:00Z"),
        line("2", "[思考] 用户在问好", "2026-01-01T00:00:01Z"),
        line("3", "- 写代码", "2026-01-01T00:00:20Z"),
        line("4", "内置 Agent 会话已恢复", "2026-09-03T07:00:00Z"),
        line("5", "[PERMISSION] 已在设置中关闭高风险确认", "2026-09-03T07:00:01Z"),
        line("6", "[续聊] 已恢复上一会话 4 条上下文（图片附件不恢复）", "2026-09-03T07:00:02Z"),
        line(
          "7",
          "[内置 Agent] 启动会话 渠道=Mai-grok 协议=openai model=grok-4.6 effort=medium thinking=on",
          "2026-09-03T07:00:03Z",
        ),
        line("8", "[MCP] 未启用服务器", "2026-09-03T07:00:04Z"),
        line("9", "[USER_INPUT] 你是什么模型？", "2026-09-03T07:00:05Z"),
        line("10", "我是 grok-4.6", "2026-09-03T07:00:08Z"),
      ]),
    );
    expect(blocks).toHaveLength(2);
    expect(blocks[0]?.user?.text).toBe("你好你能做什么");
    expect(blocks[0]?.endedAt).toBe("2026-01-01T00:00:20Z");
    expect(blocks[0]?.assistant.map((item) => item.text)).toEqual(["- 写代码"]);
    expect(workDurationSeconds(blocks[0]!)).toBe(20);
    expect(blocks[1]?.user?.text).toBe("你是什么模型？");
    expect(blocks[1]?.startedAt).toBe("2026-09-03T07:00:05Z");
    expect(blocks[1]?.endedAt).toBe("2026-09-03T07:00:08Z");
  });

  it("starts a new turn from live resume banners without a session_requested line", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 你好", "2026-01-01T00:00:00Z"),
        line("2", "先这样", "2026-01-01T00:00:10Z"),
        line("3", "[续聊] 已恢复上一会话 2 条上下文（图片附件不恢复）", "2026-09-03T07:00:00Z"),
        line(
          "4",
          "[内置 Agent] 启动会话 渠道=Mai-grok 协议=openai model=grok-4.6 effort=medium thinking=on",
          "2026-09-03T07:00:01Z",
        ),
        line("5", "[USER_INPUT] 你是什么模型？", "2026-09-03T07:00:02Z"),
      ]),
    );
    expect(blocks).toHaveLength(2);
    expect(blocks[0]?.endedAt).toBe("2026-01-01T00:00:10Z");
    expect(blocks[1]?.user?.text).toBe("你是什么模型？");
  });

  it("does not split a turn on mid-turn permission prompts", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 删文件", "2026-01-01T00:00:00Z"),
        line("2", "[PERMISSION] 等待确认高风险操作（local / Shell）：rm", "2026-01-01T00:00:01Z"),
        line("3", "[命令] rm x", "2026-01-01T00:00:02Z"),
        line("4", "已删除", "2026-01-01T00:00:03Z"),
      ]),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual([
      "system",
      "terminal",
      "assistant",
    ]);
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
    expect(thinkingText(groupSessionLines([line("2", "[思考] 8秒\n先看入口再改 Composer")]))).toBe(
      "先看入口再改 Composer",
    );
  });

  it("reads persisted thinking duration instead of a zero-width timestamp span", () => {
    expect(parseThinkingDurationSeconds("[思考] 8秒\n先看入口")).toBe(8);
    expect(parseThinkingDurationSeconds("[思考]\n先看入口")).toBeNull();
    const untimed = groupSessionLines([
      line("1", "[思考]\n旧记录没有秒数", "2026-01-01T00:00:00Z"),
    ]);
    expect(thinkingDurationSeconds(untimed)).toBe(0);
    const timed = groupSessionLines([
      line("1", "[思考] 11秒\n先看仓库文档", "2026-01-01T00:00:00Z"),
    ]);
    expect(thinkingDurationSeconds(timed)).toBe(11);
  });

  it("formats durations with minutes and hours after 60 seconds", () => {
    const t = (key: string, options?: Record<string, number>) => {
      if (key === "durationSeconds") return `${options?.seconds}秒`;
      if (key === "durationMinutesOnly") return `${options?.minutes}分钟`;
      if (key === "durationMinutesSeconds") return `${options?.minutes}分${options?.seconds}秒`;
      if (key === "durationHoursOnly") return `${options?.hours}小时`;
      if (key === "durationHoursMinutes") return `${options?.hours}小时${options?.minutes}分`;
      return `${options?.hours}小时${options?.minutes}分${options?.seconds}秒`;
    };
    expect(formatSessionDuration(t, 11)).toBe("11秒");
    expect(formatSessionDuration(t, 60)).toBe("1分钟");
    expect(formatSessionDuration(t, 83)).toBe("1分23秒");
    expect(formatSessionDuration(t, 3600)).toBe("1小时");
    expect(formatSessionDuration(t, 3661)).toBe("1小时1分1秒");
  });

  it("truncates display titles to 30 characters", () => {
    expect(displaySessionTitle("  hello  ")).toBe("hello");
    expect(
      displaySessionTitle("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十超出"),
    ).toBe("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十");
  });

  it("parses rust retry lines and ascii punctuation", () => {
    const rust = parseRetryLine(
      '[重试] 模型请求失败（HTTP 429）: {"error":{"message":"All available accounts are currently rate-limited. Please retry later.","type":"rate_limit_error"}}，3 秒后进行第 2/10 次重试',
    );
    expect(rust).toMatchObject({
      failed: false,
      status: 429,
      delaySeconds: 3,
      attempt: 2,
      maxRetries: 10,
      title: "模型请求失败",
      message: "All available accounts are currently rate-limited. Please retry later.",
    });
    expect(rust?.json).toContain('"type": "rate_limit_error"');

    const ascii = parseRetryLine(
      '[重试] 模型请求失败 (HTTP 502) : {"error":{"message":"bad gateway"}}, 3 秒后进行第 1/10 次重试',
    );
    expect(ascii).toMatchObject({
      status: 502,
      attempt: 1,
      maxRetries: 10,
      message: "bad gateway",
    });

    const child = parseRetryLine(
      "[子 Agent 1(general) - 改文件] [重试] 模型请求失败（HTTP 503）: gateway，3 秒后进行第 1/10 次重试",
    );
    expect(child).toMatchObject({
      agentPrefix: "[子 Agent 1(general) - 改文件]",
      status: 503,
      attempt: 1,
      title: "模型请求失败: gateway",
    });

    expect(
      parseRetryLine(
        '[ERROR] 模型请求失败（HTTP 429）: {"error":{"message":"rate limited","type":"rate_limit_error"}}',
      ),
    ).toMatchObject({
      failed: true,
      status: 429,
      message: "rate limited",
    });
    expect(parseRetryLine("[ERROR] boom")).toBeNull();
  });

  it("groups consecutive retries and folds the trailing model error", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 分析项目", "2026-01-01T00:00:00Z"),
        line(
          "2",
          '[重试] 模型请求失败（HTTP 502）: {"error":{"message":"bad gateway"}}，3 秒后进行第 1/10 次重试',
          "2026-01-01T00:00:01Z",
        ),
        line(
          "3",
          '[重试] 模型请求失败（HTTP 429）: {"error":{"message":"rate limited"}}，3 秒后进行第 2/10 次重试',
          "2026-01-01T00:00:04Z",
        ),
        line(
          "4",
          '[ERROR] 模型请求失败（HTTP 429）: {"error":{"message":"rate limited"}}',
          "2026-01-01T00:00:07Z",
        ),
      ]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["retry"]);
    expect(blocks[0]?.segments[0]?.items).toHaveLength(3);
    expect(summarizeRetry(blocks[0]!.segments[0]!.items)).toEqual({
      status: 429,
      attempt: 2,
      maxRetries: 10,
      count: 2,
      failed: true,
    });
  });

  it("parses plan documents and status lines into their own segments", () => {
    expect(parsePlanLine("[PLAN]\n## 目标\n- 改 Composer")).toEqual({
      kind: "document",
      status: null,
      title: "目标",
      body: "## 目标\n- 改 Composer",
      questionSummary: null,
    });
    expect(parsePlanLine("[计划]\n先摸底再改")).toMatchObject({
      kind: "document",
      title: "先摸底再改",
      body: "先摸底再改",
    });
    expect(parsePlanLine("[PLAN] 开始执行")).toEqual({
      kind: "status",
      status: "execute",
      title: null,
      body: "开始执行",
      questionSummary: null,
    });
    expect(parsePlanLine("[PLAN] 等待用户回答：用哪个入口")).toEqual({
      kind: "status",
      status: "waiting_question",
      title: null,
      body: "等待用户回答：用哪个入口",
      questionSummary: "用哪个入口",
    });
    expect(parsePlanLine("[思考] 不是计划")).toBeNull();

    const blocks = buildTurnBlocks(
      groupSessionLines([
        line(
          "1",
          "[PLAN] 已进入计划模式：只读摸底，本轮结束后自动开始执行",
          "2026-01-01T00:00:00Z",
        ),
        line("2", "[USER_INPUT] 做个方案", "2026-01-01T00:00:01Z"),
        line("3", "[PLAN]\n## 目标\n分两步改", "2026-01-01T00:00:02Z"),
        line("4", "[PLAN] 开始执行", "2026-01-01T00:00:03Z"),
        line("5", "开始改文件", "2026-01-01T00:00:04Z"),
      ]),
    );
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual([
      "plan",
      "plan",
      "plan",
      "assistant",
    ]);
    expect(blocks[0]?.segments[0]?.items).toHaveLength(1);
    expect(blocks[0]?.segments[1]?.items[0]?.text).toContain("## 目标");
    expect(parsePlanLine(blocks[0]!.segments[2]!.items[0]!.text)?.status).toBe("execute");
  });

  it("keeps unrelated errors out of the retry segment", () => {
    const blocks = buildTurnBlocks(
      groupSessionLines([
        line("1", "[USER_INPUT] 问好", "2026-01-01T00:00:00Z"),
        line("2", "[ERROR] boom", "2026-01-01T00:00:01Z"),
        line("3", "已取消", "2026-01-01T00:00:02Z"),
      ]),
    );
    expect(blocks[0]?.segments.map((segment) => segment.kind)).toEqual(["system", "assistant"]);
  });
});
