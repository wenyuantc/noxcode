import { describe, expect, it } from "vitest";

import type { AiChannel, NativeSubagent, Workspace } from "@/lib/types";
import type { SubagentImportDraft } from "@/lib/subagentJson";
import {
  SUBAGENT_JSON_KIND,
  SUBAGENT_JSON_VERSION,
  parseSubagentImportJson,
  serializeSubagentDraftsJson,
  serializeSubagentJson,
  toImportedSubagentPayload,
} from "@/lib/subagentJson";

function subagent(overrides: Partial<NativeSubagent> = {}): NativeSubagent {
  return {
    id: "sa1",
    name: "code-reviewer",
    description: "审查 diff",
    model_mode: "inherit",
    channel_id: null,
    model: null,
    reasoning_effort: null,
    tool_mode: "custom",
    tools: ["Read", "Grep"],
    system_prompt: "你是审查员",
    inject_agents_md: true,
    scope: "all",
    workspace_ids: [],
    ...overrides,
  };
}

function channel(overrides: Partial<AiChannel> = {}): AiChannel {
  return {
    id: "ch1",
    name: "主渠道",
    protocol: "openai",
    base_url: "https://example.com",
    extra_headers_json: null,
    models: [
      {
        id: "model-1",
        context_tokens: null,
        max_output_tokens: null,
        thinking_enabled: null,
        thinking_level: null,
        thinking_levels: null,
        input_types: null,
      },
    ],
    responses_continuation: "auto",
    enabled: true,
    api_key: null,
    api_key_configured: false,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function workspace(id: string): Workspace {
  return {
    id,
    name: id,
    workspace_type: "local",
    repo_path: null,
    ssh_config_id: null,
    remote_repo_path: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };
}

function draft(overrides: Partial<SubagentImportDraft> = {}): SubagentImportDraft {
  return {
    name: "code-reviewer",
    description: "审查 diff",
    model_mode: "inherit",
    channel_id: null,
    model: null,
    reasoning_effort: null,
    tool_mode: "custom",
    tools: ["Read", "Grep"],
    system_prompt: "你是审查员",
    inject_agents_md: true,
    scope: "all",
    workspace_ids: [],
    permission_mode: null,
    disallowed_tools: [],
    ...overrides,
  };
}

describe("serializeSubagentJson", () => {
  it("round-trips every exported field through parse", () => {
    const parsed = parseSubagentImportJson(
      serializeSubagentJson(
        subagent({ permission_mode: "acceptEdits", disallowed_tools: ["Bash"] }),
      ),
    );

    expect(parsed).toHaveLength(1);
    expect(parsed[0]).toEqual(
      draft({ permission_mode: "acceptEdits", disallowed_tools: ["Bash"] }),
    );
  });

  it("omits id, source, path, max_turns and skills", () => {
    const json = serializeSubagentJson(
      subagent({ source: "file", path: "/tmp/a.md", max_turns: 5, skills: ["x"] }),
    );

    expect(json).toContain(`"kind": "${SUBAGENT_JSON_KIND}"`);
    expect(json).toContain(`"version": ${SUBAGENT_JSON_VERSION}`);
    expect(json).not.toContain('"id"');
    expect(json).not.toContain('"source"');
    expect(json).not.toContain('"path"');
    expect(json).not.toContain('"max_turns"');
    expect(json).not.toContain('"skills"');
  });

  it("forces inherit to clear channel fields", () => {
    const parsed = parseSubagentImportJson(
      serializeSubagentJson(
        subagent({
          model_mode: "inherit",
          channel_id: "ch1",
          model: "model-1",
          reasoning_effort: "high",
        }),
      ),
    );

    expect(parsed[0].model_mode).toBe("inherit");
    expect(parsed[0].channel_id).toBeNull();
    expect(parsed[0].model).toBeNull();
    expect(parsed[0].reasoning_effort).toBeNull();
  });

  it("forces all tool and scope modes to empty lists", () => {
    const parsed = parseSubagentImportJson(
      serializeSubagentJson(
        subagent({ tool_mode: "all", tools: ["Read"], scope: "all", workspace_ids: ["w1"] }),
      ),
    );

    expect(parsed[0].tool_mode).toBe("all");
    expect(parsed[0].tools).toEqual([]);
    expect(parsed[0].scope).toBe("all");
    expect(parsed[0].workspace_ids).toEqual([]);
  });
});

describe("parseSubagentImportJson", () => {
  it("parses an array of objects", () => {
    const parsed = parseSubagentImportJson(
      JSON.stringify([
        { name: "a", description: "A" },
        { name: "b", description: "B", tool_mode: "all" },
      ]),
    );

    expect(parsed).toHaveLength(2);
    expect(parsed[0].name).toBe("a");
    expect(parsed[0].model_mode).toBe("inherit");
    expect(parsed[0].tool_mode).toBe("all");
    expect(parsed[1].name).toBe("b");
  });

  it("parses a { subagents } wrapper without requiring kind on entries", () => {
    const parsed = parseSubagentImportJson(
      JSON.stringify({
        subagents: [{ name: "a", description: "A" }],
      }),
    );

    expect(parsed).toHaveLength(1);
    expect(parsed[0].name).toBe("a");
  });

  it("rejects a wrapper with an unsupported version", () => {
    expect(() =>
      parseSubagentImportJson(
        JSON.stringify({
          version: 2,
          subagents: [{ name: "a", description: "A" }],
        }),
      ),
    ).toThrow("不支持的子智能体 JSON 版本");
  });

  it("rejects a wrapper with a foreign kind", () => {
    expect(() =>
      parseSubagentImportJson(
        JSON.stringify({
          kind: "other",
          subagents: [{ name: "a", description: "A" }],
        }),
      ),
    ).toThrow("不是 noxcode 子智能体 JSON");
  });

  it("treats a non-array subagents field as a single object", () => {
    const parsed = parseSubagentImportJson(
      JSON.stringify({ name: "a", description: "A", subagents: "nope" }),
    );
    expect(parsed).toHaveLength(1);
    expect(parsed[0].name).toBe("a");
  });

  it("round-trips inject_agents_md false", () => {
    const parsed = parseSubagentImportJson(
      serializeSubagentJson(subagent({ inject_agents_md: false })),
    );
    expect(parsed[0].inject_agents_md).toBe(false);
  });

  it("rewrites remaining drafts after a partial import", () => {
    const json = serializeSubagentDraftsJson([draft({ name: "second", description: "还没导入" })]);
    const parsed = parseSubagentImportJson(json);
    expect(parsed).toHaveLength(1);
    expect(parsed[0].name).toBe("second");
  });

  it("trims string arrays and drops blanks", () => {
    const parsed = parseSubagentImportJson(
      JSON.stringify({
        name: "a",
        description: "A",
        tool_mode: "custom",
        tools: [" Read ", "", "Grep", 5],
        workspace_ids: "nope",
      }),
    );

    expect(parsed[0].tools).toEqual(["Read", "Grep"]);
    expect(parsed[0].workspace_ids).toEqual([]);
    expect(parsed[0].inject_agents_md).toBe(true);
  });
});

describe("parseSubagentImportJson errors", () => {
  it("rejects blank text", () => {
    expect(() => parseSubagentImportJson("   ")).toThrow("请粘贴 JSON");
  });

  it("rejects malformed json", () => {
    expect(() => parseSubagentImportJson("{")).toThrow("JSON 格式无效");
  });

  it("rejects non-object json", () => {
    expect(() => parseSubagentImportJson("42")).toThrow("JSON 不是子智能体对象");
  });

  it("rejects empty arrays and empty wrappers", () => {
    expect(() => parseSubagentImportJson("[]")).toThrow("没有可导入的子智能体");
    expect(() => parseSubagentImportJson(JSON.stringify({ subagents: [] }))).toThrow(
      "没有可导入的子智能体",
    );
  });

  it("rejects a missing name or description", () => {
    expect(() => parseSubagentImportJson(JSON.stringify({ name: "a", description: " " }))).toThrow(
      "子智能体缺少名称或描述",
    );
  });

  it("rejects a foreign kind", () => {
    expect(() =>
      parseSubagentImportJson(JSON.stringify({ kind: "other", name: "a", description: "A" })),
    ).toThrow("不是 noxcode 子智能体 JSON");
  });

  it("rejects an unsupported version", () => {
    expect(() =>
      parseSubagentImportJson(
        JSON.stringify({ kind: SUBAGENT_JSON_KIND, version: 2, name: "a", description: "A" }),
      ),
    ).toThrow("不支持的子智能体 JSON 版本");
  });
});

describe("toImportedSubagentPayload channels", () => {
  it("keeps channel mode when the enabled channel exposes the model", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({
        model_mode: "channel",
        channel_id: "ch1",
        model: "model-1",
        reasoning_effort: "high",
      }),
      { channels: [channel()], workspaces: [] },
    );

    expect(warnings).toEqual([]);
    expect(payload.model_mode).toBe("channel");
    expect(payload.channel_id).toBe("ch1");
    expect(payload.model).toBe("model-1");
    expect(payload.reasoning_effort).toBe("high");
  });

  it("downgrades to inherit when the channel is unknown", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ model_mode: "channel", channel_id: "missing", model: "model-1" }),
      { channels: [channel()], workspaces: [] },
    );

    expect(payload.model_mode).toBe("inherit");
    expect(payload.channel_id).toBeNull();
    expect(payload.model).toBeNull();
    expect(payload.reasoning_effort).toBeNull();
    expect(warnings).toEqual([{ code: "channelMissing", channelId: "missing", model: "model-1" }]);
  });

  it("downgrades to inherit when the channel is disabled", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ model_mode: "channel", channel_id: "ch1", model: "model-1" }),
      { channels: [channel({ enabled: false })], workspaces: [] },
    );

    expect(payload.model_mode).toBe("inherit");
    expect(warnings).toEqual([{ code: "channelMissing", channelId: "ch1", model: "model-1" }]);
  });

  it("downgrades to inherit when channel fields are empty", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ model_mode: "channel", channel_id: null, model: null }),
      { channels: [channel()], workspaces: [] },
    );

    expect(payload.model_mode).toBe("inherit");
    expect(warnings).toEqual([{ code: "channelMissing" }]);
  });

  it("downgrades to inherit when the model is not offered", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ model_mode: "channel", channel_id: "ch1", model: "other" }),
      { channels: [channel()], workspaces: [] },
    );

    expect(payload.model_mode).toBe("inherit");
    expect(warnings).toEqual([{ code: "channelMissing", channelId: "ch1", model: "other" }]);
  });
});

describe("toImportedSubagentPayload workspaces", () => {
  it("falls back to all scope when no workspace exists", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ scope: "workspaces", workspace_ids: ["ghost"] }),
      { channels: [], workspaces: [workspace("w1")] },
    );

    expect(payload.scope).toBe("all");
    expect(payload.workspace_ids).toEqual([]);
    expect(warnings).toEqual([{ code: "workspacesMissing" }]);
  });

  it("keeps the surviving workspaces and reports dropped ones", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ scope: "workspaces", workspace_ids: ["w1", "ghost", "w2"] }),
      { channels: [], workspaces: [workspace("w1"), workspace("w2")] },
    );

    expect(payload.scope).toBe("workspaces");
    expect(payload.workspace_ids).toEqual(["w1", "w2"]);
    expect(warnings).toEqual([{ code: "workspacesPartial", dropped: 1 }]);
  });

  it("leaves a fully valid workspace scope untouched", () => {
    const { payload, warnings } = toImportedSubagentPayload(
      draft({ scope: "workspaces", workspace_ids: ["w1"] }),
      { channels: [], workspaces: [workspace("w1")] },
    );

    expect(payload.workspace_ids).toEqual(["w1"]);
    expect(warnings).toEqual([]);
  });
});
