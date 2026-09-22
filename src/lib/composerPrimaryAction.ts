export type ComposerPrimaryAction = "send" | "stop";

export function composerHasSubmittable(input: {
  draft: string;
  attachmentCount: number;
  hasPills: boolean;
}): boolean {
  return input.draft.trim().length > 0 || input.attachmentCount > 0 || input.hasPills;
}

export function composerPrimaryAction(input: {
  working: boolean;
  hasSubmittable: boolean;
  sendBusy: boolean;
}): ComposerPrimaryAction {
  if (input.sendBusy) return "send";
  if (input.working && !input.hasSubmittable) return "stop";
  return "send";
}
