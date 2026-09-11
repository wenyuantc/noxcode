import type { ComposerPillsState } from "./composerPills";
import { skillInvocationPrompt, subagentDelegationPrompt } from "./composerSlash";
import { resolveComposerSlash, type SlashIntent } from "./composerSlashActions";

export interface AssembledComposerPrompt {
  prompt: string;
  intent: SlashIntent;
}

export function buildFilesContextPrefix(files: string[]): string {
  if (files.length === 0) return "";
  return files.map((file) => `@${file}`).join(" ");
}

export function combineFilesWithUserPrompt(files: string[], userText: string): string {
  const filePrefix = buildFilesContextPrefix(files);
  const trimmedText = userText.trim();
  if (filePrefix && trimmedText) {
    return `${filePrefix}\n\n${trimmedText}`;
  }
  return filePrefix || trimmedText;
}

export function assembleComposerPrompt(
  pills: ComposerPillsState,
  draftText: string,
): AssembledComposerPrompt {
  const userContent = combineFilesWithUserPrompt(pills.files, draftText);

  // 1. If target is a subagent pill
  if (pills.target?.kind === "subagent") {
    const delegation = subagentDelegationPrompt(pills.target.name, pills.target.id);
    const fullPrompt = userContent ? `${delegation}\n\n${userContent}` : delegation;
    return {
      prompt: fullPrompt,
      intent: { type: "plain", prompt: fullPrompt },
    };
  }

  // 2. If target is a skill pill
  if (pills.target?.kind === "skill") {
    const fullPrompt = skillInvocationPrompt(pills.target.name, userContent);
    return {
      prompt: fullPrompt,
      intent: { type: "skill", name: pills.target.name, args: userContent },
    };
  }

  // 3. If target is a command pill
  if (pills.target?.kind === "command") {
    const commandLine = `/${pills.target.name}${userContent ? ` ${userContent}` : ""}`;
    const intent = resolveComposerSlash(commandLine);
    return {
      prompt: commandLine,
      intent,
    };
  }

  // 4. Default: No target pill, check draftText for slash/skill syntax or plain prompt
  const rawIntent = resolveComposerSlash(draftText);
  if (pills.files.length > 0) {
    const fullPrompt = combineFilesWithUserPrompt(pills.files, draftText);
    if (rawIntent.type === "plain") {
      return {
        prompt: fullPrompt,
        intent: { type: "plain", prompt: fullPrompt },
      };
    }
  }

  return {
    prompt: userContent,
    intent: rawIntent,
  };
}
