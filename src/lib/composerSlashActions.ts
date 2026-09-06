import {
  buildCreateSkillPrompt,
  buildCreateSubagentPrompt,
  buildGoalPrompt,
  buildInitPrompt,
  buildReviewPrompt,
  isBuiltinSlashName,
  parseLeadingSlash,
  parseNamedSlashArgs,
  parseSkillInvocation,
} from "./composerSlash";
import { isNativePermissionMode, type NativePermissionMode } from "./types";

export type ComposerPermissionChoice = NativePermissionMode | "plan";

export type SlashIntent =
  | { type: "plain"; prompt: string }
  | { type: "expand"; prompt: string }
  | { type: "skill"; name: string; args: string }
  | { type: "custom"; name: string; args: string }
  | { type: "fork"; checkpointId?: string }
  | { type: "compact"; instructions?: string }
  | { type: "new-session" }
  | { type: "set-mode"; mode: ComposerPermissionChoice }
  | { type: "mode-help" }
  | { type: "set-model"; query: string }
  | { type: "open-models" }
  | { type: "set-effort"; level: string }
  | { type: "effort-help" }
  | { type: "plan"; task?: string }
  | { type: "navigate"; path: string }
  | { type: "plugins" }
  | { type: "diff" }
  | { type: "context" }
  | { type: "help" }
  | { type: "open-dialog"; dialog: "skill" | "subagent" }
  | { type: "skill-help" };

const MODE_ALIASES: Record<string, ComposerPermissionChoice> = {
  default: "default",
  confirm: "default",
  edit: "edit",
  build: "build",
  plan: "plan",
  yolo: "yolo",
  full: "yolo",
};

export function parseModeArg(args: string): ComposerPermissionChoice | null {
  const token = args.trim().split(/\s+/)[0]?.toLowerCase();
  if (!token) return null;
  const mapped = MODE_ALIASES[token];
  if (mapped) return mapped;
  return isNativePermissionMode(token) ? token : null;
}

export function matchComposerEffort(levels: string[], query: string): string | null {
  const needle = query.trim().toLowerCase();
  if (!needle) return null;
  const exact = levels.find((level) => level.toLowerCase() === needle);
  if (exact) return exact;
  const prefixed = levels.filter((level) => level.toLowerCase().startsWith(needle));
  return prefixed.length === 1 ? (prefixed[0] ?? null) : null;
}

export function matchComposerModel(
  channels: Array<{
    id: string;
    name: string;
    enabled?: boolean;
    models: Array<{ id: string }>;
  }>,
  query: string,
): { channelId: string; modelId: string } | null {
  const needle = query.trim().toLowerCase();
  if (!needle) return null;
  const enabled = channels.filter((channel) => channel.enabled !== false);
  const pairs = enabled.flatMap((channel) =>
    channel.models.map((model) => ({
      channelId: channel.id,
      modelId: model.id,
      hay: `${channel.name}/${model.id} ${channel.id}/${model.id} ${model.id}`.toLowerCase(),
    })),
  );
  const exact = pairs.find(
    (item) =>
      item.modelId.toLowerCase() === needle ||
      `${item.channelId}/${item.modelId}`.toLowerCase() === needle ||
      item.hay.split(" ")[0] === needle,
  );
  if (exact) return { channelId: exact.channelId, modelId: exact.modelId };
  const fuzzy = pairs.filter(
    (item) => item.hay.includes(needle) || item.modelId.toLowerCase().includes(needle),
  );
  if (fuzzy.length === 1) {
    const only = fuzzy[0]!;
    return { channelId: only.channelId, modelId: only.modelId };
  }
  return null;
}

export function isLocalSlashIntent(intent: SlashIntent): boolean {
  switch (intent.type) {
    case "new-session":
    case "set-mode":
    case "mode-help":
    case "set-model":
    case "open-models":
    case "set-effort":
    case "effort-help":
    case "navigate":
    case "plugins":
    case "diff":
    case "context":
    case "help":
    case "open-dialog":
    case "skill-help":
      return true;
    case "plan":
      return !intent.task;
    default:
      return false;
  }
}

export function isExpandingSlashIntent(intent: SlashIntent): boolean {
  return intent.type === "expand" || intent.type === "skill" || intent.type === "custom";
}

export function resolveComposerSlash(prompt: string): SlashIntent {
  const trimmed = prompt.trim();
  const skill = parseSkillInvocation(trimmed);
  if (skill) return { type: "skill", name: skill.name, args: skill.args };

  const slash = parseLeadingSlash(trimmed);
  if (!slash) return { type: "plain", prompt: trimmed };

  const name = slash.name.toLowerCase();
  const args = slash.args;

  if (!isBuiltinSlashName(name)) {
    return { type: "custom", name: slash.name, args };
  }

  switch (name) {
    case "init":
      return { type: "expand", prompt: buildInitPrompt(args) };
    case "fork":
      return { type: "fork", checkpointId: args.split(/\s+/)[0] || undefined };
    case "compact":
      return { type: "compact", instructions: args || undefined };
    case "new":
    case "clear":
      return { type: "new-session" };
    case "mode": {
      const mode = parseModeArg(args);
      return mode ? { type: "set-mode", mode } : { type: "mode-help" };
    }
    case "model":
      return args
        ? { type: "set-model", query: args.split(/\s+/)[0] ?? args }
        : { type: "open-models" };
    case "effort":
      return args
        ? { type: "set-effort", level: args.split(/\s+/)[0] ?? args }
        : { type: "effort-help" };
    case "plan":
      return { type: "plan", task: args || undefined };
    case "permissions":
      return { type: "navigate", path: "/settings/permissions" };
    case "memory":
      return { type: "navigate", path: "/settings/memory" };
    case "mcp":
      return { type: "navigate", path: "/settings/mcp" };
    case "plugins":
      return { type: "plugins" };
    case "skill":
      return { type: "skill-help" };
    case "diff":
      return { type: "diff" };
    case "context":
      return { type: "context" };
    case "help":
      return { type: "help" };
    case "goal":
      return { type: "expand", prompt: buildGoalPrompt(args) };
    case "review":
      return { type: "expand", prompt: buildReviewPrompt(args) };
    case "create-skill": {
      const parsed = parseNamedSlashArgs(args);
      if (!parsed) return { type: "open-dialog", dialog: "skill" };
      return { type: "expand", prompt: buildCreateSkillPrompt(parsed.name, parsed.rest) };
    }
    case "create-subagent": {
      const parsed = parseNamedSlashArgs(args);
      if (!parsed) return { type: "open-dialog", dialog: "subagent" };
      return { type: "expand", prompt: buildCreateSubagentPrompt(parsed.name, parsed.rest) };
    }
    default:
      return { type: "custom", name: slash.name, args };
  }
}
