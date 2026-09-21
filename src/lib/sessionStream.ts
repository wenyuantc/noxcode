import type { NativeAssistantFragment } from "@/lib/types";
import {
  assistantItemsText,
  classifyLine,
  isThinkingItem,
  thinkingText,
  type GroupedSessionItem,
  type SessionTurnBlock,
  type TurnSegment,
} from "@/lib/sessionLines";

export interface SessionStreamFragment {
  id: string;
  kind: string;
  text: string;
  startedAt: string;
  assistant?: NativeAssistantFragment;
}

export const EMPTY_STREAM: SessionStreamFragment[] = [];

function covers(persisted: string, live: string): boolean {
  if (!persisted || !live) return false;
  return persisted.includes(live) || live.includes(persisted);
}

export function applyTextDelta(
  parts: readonly SessionStreamFragment[],
  delta: { kind: string; text: string; clear: boolean; assistant?: NativeAssistantFragment | null },
  nowIso: string,
): SessionStreamFragment[] {
  if (delta.clear) return [];
  if (!delta.text) return parts.length === 0 ? [] : (parts as SessionStreamFragment[]);
  if (delta.assistant) {
    const index = parts.findIndex(
      (part) =>
        part.kind === delta.kind &&
        part.assistant?.chain_id === delta.assistant!.chain_id &&
        part.assistant.part === delta.assistant!.part,
    );
    if (index >= 0) {
      const next = parts.slice();
      next[index] = { ...next[index]!, text: next[index]!.text + delta.text };
      return next;
    }
    return [
      ...parts,
      {
        id: `${delta.assistant.chain_id}:${delta.assistant.part}:${delta.kind}`,
        kind: delta.kind,
        text: delta.text,
        startedAt: nowIso,
        assistant: delta.assistant,
      },
    ];
  }
  const last = parts[parts.length - 1];
  if (last && !last.assistant && last.kind === delta.kind) {
    const next = parts.slice();
    next[next.length - 1] = { ...last, text: last.text + delta.text };
    return next;
  }
  return [
    ...parts,
    {
      id: `${delta.kind}-${parts.length}`,
      kind: delta.kind,
      text: delta.text,
      startedAt: nowIso,
    },
  ];
}

function persistedBodies(block: SessionTurnBlock, kind: "thinking" | "assistant"): string[] {
  return block.segments
    .filter((segment) => segment.kind === kind)
    .map((segment) =>
      kind === "thinking" ? thinkingText(segment.items) : assistantItemsText(segment.items),
    );
}

export function isFragmentCovered(
  block: SessionTurnBlock,
  fragment: SessionStreamFragment,
): boolean {
  if (!fragment.text) return true;
  if (fragment.assistant) {
    return block.assistant.some(
      (item) =>
        item.assistant?.chain_id === fragment.assistant!.chain_id &&
        item.assistant.part >= fragment.assistant!.part,
    );
  }
  const kind = fragment.kind === "reasoning" ? "thinking" : "assistant";
  return persistedBodies(block, kind).some((body) => covers(body, fragment.text));
}

function appendFragment(block: SessionTurnBlock, part: SessionStreamFragment): SessionTurnBlock {
  const item: GroupedSessionItem = {
    id: part.id,
    kind: part.kind === "reasoning" ? "system" : "assistant",
    text: part.text,
    createdAt: part.startedAt,
    assistant: part.assistant,
    streaming: true,
  };
  if (part.assistant && part.kind === "text") {
    const previous = block.assistant.find(
      (existing) => existing.assistant?.chain_id === part.assistant!.chain_id,
    );
    if (previous) {
      const merged = {
        ...previous,
        text: previous.text + part.text,
        assistant: part.assistant,
        streaming: true,
      };
      return {
        ...block,
        assistant: block.assistant.map((i) => (i === previous ? merged : i)),
        segments: block.segments.map((s) => ({
          ...s,
          items: s.items.map((i) => (i === previous ? merged : i)),
        })),
      };
    }
  }
  const targetKind = part.kind === "reasoning" ? "thinking" : "assistant";
  const segments = [...block.segments];
  const last = segments[segments.length - 1];
  if (last?.kind === targetKind) {
    segments[segments.length - 1] = { ...last, items: [...last.items, item] };
  } else {
    segments.push({ kind: targetKind, items: [item] });
  }
  return {
    ...block,
    assistant: targetKind === "assistant" ? [...block.assistant, item] : block.assistant,
    segments,
  };
}

export function attachLiveFragments(
  block: SessionTurnBlock,
  parts: readonly SessionStreamFragment[],
): SessionTurnBlock {
  let next = block;
  let changed = false;
  for (const part of parts) {
    if (!part.text || isFragmentCovered(next, part)) continue;
    next = appendFragment(next, part);
    changed = true;
  }
  return changed ? next : block;
}

export function pruneCoveredFragments(
  parts: readonly SessionStreamFragment[],
  lineText: string,
  assistant?: NativeAssistantFragment | null,
): SessionStreamFragment[] {
  if (parts.length === 0) return parts as SessionStreamFragment[];
  if (assistant)
    return parts.filter(
      (part) =>
        !part.assistant ||
        part.assistant.chain_id !== assistant.chain_id ||
        part.assistant.part > assistant.part,
    );
  const item: GroupedSessionItem = {
    id: "",
    kind: classifyLine(lineText),
    text: lineText,
    createdAt: "",
  };
  let body: string | null = null;
  let kind: string | null = null;
  if (isThinkingItem(item)) {
    body = thinkingText([item]);
    kind = "reasoning";
  } else if (item.kind === "assistant") {
    body = lineText;
    kind = "text";
  }
  if (!body || !kind) return parts as SessionStreamFragment[];
  const next = parts.filter(
    (part) => part.assistant || part.kind !== kind || !covers(body, part.text),
  );
  return next.length === parts.length ? (parts as SessionStreamFragment[]) : next;
}

export function turnSegmentReactKey(
  blockId: string,
  segment: TurnSegment,
  index: number,
  segments: readonly TurnSegment[],
): string {
  if (segment.kind === "thinking" || segment.kind === "assistant") {
    let occurrence = 0;
    for (let i = 0; i < index; i += 1) {
      if (segments[i]?.kind === segment.kind) occurrence += 1;
    }
    return `${blockId}:${segment.kind}:${occurrence}`;
  }
  return `${segment.kind}-${segment.items[0]?.id ?? index}`;
}
