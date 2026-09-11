export type ComposerTargetKind = "command" | "skill" | "subagent";

export interface ComposerCommandTarget {
  kind: "command";
  name: string;
  description?: string;
  argumentHint?: string;
  token: string;
}

export interface ComposerSkillTarget {
  kind: "skill";
  name: string;
  description?: string;
  sourceLabel?: string;
  token: string;
}

export interface ComposerSubagentTarget {
  kind: "subagent";
  id: string;
  name: string;
  description?: string;
  token: string;
}

export type ComposerTargetPill =
  ComposerCommandTarget | ComposerSkillTarget | ComposerSubagentTarget;

export interface ComposerPillsState {
  target: ComposerTargetPill | null;
  files: string[];
}

export function initialComposerPills(): ComposerPillsState {
  return {
    target: null,
    files: [],
  };
}

export function setTargetPill(
  state: ComposerPillsState,
  target: ComposerTargetPill | null,
): ComposerPillsState {
  return {
    ...state,
    target,
  };
}

export function addFilePill(state: ComposerPillsState, filePath: string): ComposerPillsState {
  const trimmed = filePath.trim();
  if (!trimmed || state.files.includes(trimmed)) return state;
  return {
    ...state,
    files: [...state.files, trimmed],
  };
}

export function removeFilePill(state: ComposerPillsState, filePath: string): ComposerPillsState {
  return {
    ...state,
    files: state.files.filter((f) => f !== filePath),
  };
}

export function clearTargetPill(state: ComposerPillsState): ComposerPillsState {
  return {
    ...state,
    target: null,
  };
}

export function clearAllPills(state: ComposerPillsState): ComposerPillsState {
  if (!state.target && state.files.length === 0) return state;
  return {
    target: null,
    files: [],
  };
}

export function popLastPill(state: ComposerPillsState): ComposerPillsState {
  if (state.files.length > 0) {
    return {
      ...state,
      files: state.files.slice(0, -1),
    };
  }
  if (state.target) {
    return {
      ...state,
      target: null,
    };
  }
  return state;
}

export function hasPills(state: ComposerPillsState): boolean {
  return Boolean(state.target) || state.files.length > 0;
}

export function removeTrailingTrigger(text: string): string {
  return text.replace(/(?:^|\s)[@/$][^\s]*$/, "").trimEnd();
}
