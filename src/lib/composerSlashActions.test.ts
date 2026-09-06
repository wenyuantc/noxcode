import { describe, expect, it } from "vitest";

import { buildCreateSkillPrompt, buildInitPrompt } from "./composerSlash";
import {
  isExpandingSlashIntent,
  isLocalSlashIntent,
  matchComposerEffort,
  matchComposerModel,
  parseModeArg,
  resolveComposerSlash,
} from "./composerSlashActions";

describe("resolveComposerSlash", () => {
  it("expands init, goal, review and create commands", () => {
    expect(resolveComposerSlash("/init extra")).toEqual({
      type: "expand",
      prompt: buildInitPrompt("extra"),
    });
    expect(resolveComposerSlash("/create-skill code-review 审 diff")).toEqual({
      type: "expand",
      prompt: buildCreateSkillPrompt("code-review", "审 diff"),
    });
    expect(resolveComposerSlash("/create-skill")).toEqual({
      type: "open-dialog",
      dialog: "skill",
    });
    expect(resolveComposerSlash("/create-subagent")).toEqual({
      type: "open-dialog",
      dialog: "subagent",
    });
    expect(resolveComposerSlash("/goal clear").type).toBe("expand");
    expect(resolveComposerSlash("/review src").type).toBe("expand");
  });

  it("maps local session commands", () => {
    expect(resolveComposerSlash("/new")).toEqual({ type: "new-session" });
    expect(resolveComposerSlash("/clear")).toEqual({ type: "new-session" });
    expect(resolveComposerSlash("/mode yolo")).toEqual({ type: "set-mode", mode: "yolo" });
    expect(resolveComposerSlash("/mode")).toEqual({ type: "mode-help" });
    expect(resolveComposerSlash("/model gpt")).toEqual({ type: "set-model", query: "gpt" });
    expect(resolveComposerSlash("/model")).toEqual({ type: "open-models" });
    expect(resolveComposerSlash("/effort high")).toEqual({ type: "set-effort", level: "high" });
    expect(resolveComposerSlash("/plan 修登录")).toEqual({ type: "plan", task: "修登录" });
    expect(resolveComposerSlash("/permissions")).toEqual({
      type: "navigate",
      path: "/settings/permissions",
    });
    expect(resolveComposerSlash("/help")).toEqual({ type: "help" });
    expect(resolveComposerSlash("/fork ckpt-1")).toEqual({ type: "fork", checkpointId: "ckpt-1" });
    expect(resolveComposerSlash("/compact keep stacks")).toEqual({
      type: "compact",
      instructions: "keep stacks",
    });
  });

  it("keeps custom commands and skill invocations", () => {
    expect(resolveComposerSlash("/frontend:component Button")).toEqual({
      type: "custom",
      name: "frontend:component",
      args: "Button",
    });
    expect(resolveComposerSlash("/skill review --strict")).toEqual({
      type: "skill",
      name: "review",
      args: "--strict",
    });
    expect(resolveComposerSlash("$review")).toEqual({ type: "skill", name: "review", args: "" });
    expect(resolveComposerSlash("hello")).toEqual({ type: "plain", prompt: "hello" });
  });

  it("classifies local vs expanding intents", () => {
    expect(isLocalSlashIntent({ type: "help" })).toBe(true);
    expect(isLocalSlashIntent({ type: "plan" })).toBe(true);
    expect(isLocalSlashIntent({ type: "plan", task: "x" })).toBe(false);
    expect(isExpandingSlashIntent({ type: "expand", prompt: "x" })).toBe(true);
    expect(isExpandingSlashIntent({ type: "custom", name: "x", args: "" })).toBe(true);
  });
});

describe("matchers", () => {
  it("parses mode aliases", () => {
    expect(parseModeArg("plan")).toBe("plan");
    expect(parseModeArg("full")).toBe("yolo");
    expect(parseModeArg("nope")).toBeNull();
  });

  it("matches effort and model", () => {
    expect(matchComposerEffort(["low", "high", "max"], "hi")).toBe("high");
    expect(matchComposerEffort(["low", "high"], "x")).toBeNull();
    expect(
      matchComposerModel(
        [
          { id: "c1", name: "OpenAI", models: [{ id: "gpt-4.1" }, { id: "o4-mini" }] },
          { id: "c2", name: "Claude", enabled: false, models: [{ id: "sonnet" }] },
        ],
        "4.1",
      ),
    ).toEqual({ channelId: "c1", modelId: "gpt-4.1" });
  });
});
