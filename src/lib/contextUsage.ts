import { parseUsageLine } from "@/lib/sessionLines";
import type { AgentSession, AiChannel, NativeContextUsage } from "@/lib/types";

export const FALLBACK_CONTEXT_WINDOW_TOKENS = 128_000;

export function parseContextUsageJson(
  sessionId: string,
  json: string | null | undefined,
): NativeContextUsage | undefined {
  if (!json?.trim()) return undefined;
  try {
    const parsed = JSON.parse(json) as NativeContextUsage;
    if (
      typeof parsed.used_tokens !== "number" ||
      typeof parsed.limit_tokens !== "number" ||
      parsed.limit_tokens <= 0
    ) {
      return undefined;
    }
    return {
      ...parsed,
      session_record_id: parsed.session_record_id || sessionId,
    };
  } catch {
    return undefined;
  }
}

export function usageFromUsageLines(
  sessionId: string,
  lines: { text: string }[],
  limitTokens: number,
): NativeContextUsage | undefined {
  if (limitTokens <= 0) return undefined;
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const parsed = parseUsageLine(lines[index]?.text ?? "");
    if (!parsed) continue;
    const used = parsed.input ?? parsed.total;
    if (used == null || used < 0) continue;
    return {
      session_record_id: sessionId,
      used_tokens: used,
      limit_tokens: limitTokens,
      generation: 0,
      compactions: 0,
      prompt_tokens: parsed.input,
      cached_tokens: parsed.cache,
    };
  }
  return undefined;
}

export function resolveHistoryLimitTokens(
  session: Pick<AgentSession, "ai_channel_id"> | undefined,
  channels: AiChannel[],
  activeChannelId: string | null,
  activeModelId: string | null,
): number {
  const channel =
    channels.find((item) => item.id === session?.ai_channel_id) ??
    channels.find((item) => item.id === activeChannelId);
  const model = channel?.models.find((item) => item.id === activeModelId) ?? channel?.models[0];
  const tokens = model?.context_tokens;
  return tokens && tokens > 0 ? tokens : FALLBACK_CONTEXT_WINDOW_TOKENS;
}

export function resolveHistoricalUsage(input: {
  sessionId: string;
  contextUsageJson?: string | null;
  lines: { text: string }[];
  limitTokens: number;
}): NativeContextUsage | undefined {
  return (
    parseContextUsageJson(input.sessionId, input.contextUsageJson) ??
    usageFromUsageLines(input.sessionId, input.lines, input.limitTokens)
  );
}
