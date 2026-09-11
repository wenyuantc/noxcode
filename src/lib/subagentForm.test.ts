import { describe, expect, it } from "vitest";

import type { GeneratedNativeSubagent } from "./types";
import { formFromGeneratedSubagent } from "./subagentForm";

function draft(overrides: Partial<GeneratedNativeSubagent> = {}): GeneratedNativeSubagent {
  return {
    name: "code-reviewer",
    description: "审查 diff",
    model_mode: "inherit",
    tool_mode: "custom",
    tools: ["Read", "Grep"],
    system_prompt: "你是审查员",
    inject_agents_md: true,
    scope: "all",
    workspace_ids: [],
    ...overrides,
  };
}

describe("formFromGeneratedSubagent", () => {
  it("maps a generated draft onto the create form defaults", () => {
    const form = formFromGeneratedSubagent(draft());
    expect(form).toEqual({
      name: "code-reviewer",
      description: "审查 diff",
      modelMode: "inherit",
      channelId: "",
      model: "",
      toolMode: "custom",
      tools: ["Read", "Grep"],
      systemPrompt: "你是审查员",
      injectAgentsMd: true,
      scope: "all",
      workspaceIds: [],
    });
  });

  it("treats unknown tool modes as all permissions", () => {
    const form = formFromGeneratedSubagent(draft({ tool_mode: "all", tools: [] }));
    expect(form.toolMode).toBe("all");
    expect(form.tools).toEqual([]);
  });
});
