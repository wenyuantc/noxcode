export type ComposerMentionKeyAction =
  | { type: "move"; nextIndex: number }
  | { type: "confirm" }
  | { type: "dismiss" }
  | { type: "togglePlanMode" }
  | { type: "send" }
  | { type: "none" };

export function clampMentionIndex(index: number, count: number): number {
  if (count <= 0) return 0;
  if (index < 0) return 0;
  if (index >= count) return count - 1;
  return index;
}

export function resolveComposerMentionKey(input: {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
  mentionVisible: boolean;
  itemCount: number;
  activeIndex: number;
}): ComposerMentionKeyAction {
  if (input.isComposing) return { type: "none" };

  const canNavigate = input.mentionVisible && input.itemCount > 0;
  if (canNavigate) {
    if (input.key === "ArrowDown" || input.key === "Down") {
      return {
        type: "move",
        nextIndex: clampMentionIndex(input.activeIndex + 1, input.itemCount),
      };
    }
    if (input.key === "ArrowUp" || input.key === "Up") {
      return {
        type: "move",
        nextIndex: clampMentionIndex(input.activeIndex - 1, input.itemCount),
      };
    }
    if (input.key === "Enter" && !input.shiftKey) return { type: "confirm" };
    if (input.key === "Tab" && !input.shiftKey) return { type: "confirm" };
    if (input.key === "Escape") return { type: "dismiss" };
  }

  if (input.key === "Tab" && input.shiftKey) return { type: "togglePlanMode" };
  if (input.key === "Enter" && !input.shiftKey) return { type: "send" };
  return { type: "none" };
}
