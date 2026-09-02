import { NATIVE_THINKING_LEVELS, type AiChannelModel, type ModelCatalogEntry } from "@/lib/types";

export const FALLBACK_THINKING_LEVELS = ["low", "medium", "high"];

export function emptyChannelModel(id = ""): AiChannelModel {
  return {
    id,
    context_tokens: null,
    max_output_tokens: null,
    thinking_enabled: null,
    thinking_level: null,
    thinking_levels: null,
  };
}

export function catalogThinkingLevels(entry: ModelCatalogEntry | null): string[] {
  if (entry && entry.thinking_levels.length > 0) {
    return [...entry.thinking_levels];
  }
  return [...FALLBACK_THINKING_LEVELS];
}

export function uniqueThinkingLevels(levels: string[]): string[] {
  const seen = new Set<string>();
  const next: string[] = [];
  for (const level of levels) {
    const trimmed = level.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    next.push(trimmed);
  }
  return next;
}

export function defaultThinkingLevel(levels: string[], preferred?: string | null): string | null {
  if (levels.length === 0) return null;
  const current = preferred?.trim();
  if (current && levels.includes(current)) return current;
  return levels.find((level) => level === "medium") ?? levels[0] ?? null;
}

export function withThinkingLevels(
  model: AiChannelModel,
  levels: string[] | null,
  options?: { thinkingEnabled?: boolean | null },
): AiChannelModel {
  const thinkingEnabled = options?.thinkingEnabled ?? model.thinking_enabled;
  const nextLevels = levels == null ? null : uniqueThinkingLevels(levels);
  const thinkingLevel =
    thinkingEnabled === true
      ? defaultThinkingLevel(nextLevels ?? [], model.thinking_level)
      : thinkingEnabled === false
        ? null
        : model.thinking_level;
  return {
    ...model,
    thinking_enabled: thinkingEnabled,
    thinking_levels: nextLevels,
    thinking_level: thinkingLevel,
  };
}

export function displayedThinkingLevels(
  model: AiChannelModel,
  catalog: ModelCatalogEntry[] = [],
): string[] {
  return uniqueThinkingLevels([
    ...NATIVE_THINKING_LEVELS,
    ...catalog.flatMap((item) => item.thinking_levels),
    ...(model.thinking_levels ?? []),
  ]);
}

export function selectedThinkingLevels(
  model: AiChannelModel,
  entry: ModelCatalogEntry | null,
): string[] {
  if (model.thinking_levels == null) {
    return catalogThinkingLevels(entry);
  }
  return uniqueThinkingLevels(model.thinking_levels);
}

export function canSaveChannelModels(
  models: AiChannelModel[],
  catalog: ModelCatalogEntry[] = [],
): boolean {
  return models.every((model) => {
    if (!model.id.trim()) return true;
    if (model.thinking_enabled !== true) return true;
    return selectedThinkingLevels(model, lookupModelCatalog(catalog, model.id)).length > 0;
  });
}

export function materializeThinkingLevels(
  catalog: ModelCatalogEntry[],
  model: AiChannelModel,
): AiChannelModel {
  const next = applyCatalogToModel(catalog, model);
  if (next.thinking_enabled !== true) {
    return next;
  }
  const entry = lookupModelCatalog(catalog, next.id);
  const levels = selectedThinkingLevels(next, entry);
  return withThinkingLevels(next, levels, { thinkingEnabled: true });
}

export function normalizeModelKey(value: string): string {
  const trimmed = value.trim();
  const last = trimmed.split(/[/:]/).pop() ?? trimmed;
  return last.replace(/_/g, "-").toLowerCase();
}

export function lookupModelCatalog(
  catalog: ModelCatalogEntry[],
  modelId: string,
): ModelCatalogEntry | null {
  const raw = modelId.trim();
  if (!raw) return null;
  const key = normalizeModelKey(raw);
  const exact = catalog.find(
    (entry) =>
      entry.id.toLowerCase() === raw.toLowerCase() ||
      entry.aliases.some((alias) => alias.toLowerCase() === raw.toLowerCase()),
  );
  if (exact) return exact;
  const exactKey = catalog.find(
    (entry) =>
      normalizeModelKey(entry.id) === key ||
      entry.aliases.some((alias) => normalizeModelKey(alias) === key),
  );
  if (exactKey) return exactKey;
  const prefixMatches = catalog.filter((entry) => {
    const catalogKey = normalizeModelKey(entry.id);
    return catalogKey.length >= 6 && (key.startsWith(catalogKey) || catalogKey.startsWith(key));
  });
  if (prefixMatches.length === 0) return null;
  return prefixMatches.reduce((best, entry) =>
    normalizeModelKey(entry.id).length > normalizeModelKey(best.id).length ? entry : best,
  );
}

export function applyCatalogToModel(
  catalog: ModelCatalogEntry[],
  model: AiChannelModel,
  overwrite = false,
): AiChannelModel {
  const entry = lookupModelCatalog(catalog, model.id);
  if (!entry) return { ...model, id: model.id.trim() };
  const next: AiChannelModel = { ...model, id: model.id.trim() };
  if (overwrite || next.context_tokens == null) next.context_tokens = entry.context_tokens;
  if (overwrite || next.max_output_tokens == null) next.max_output_tokens = entry.max_output_tokens;
  if (overwrite || next.thinking_enabled == null) next.thinking_enabled = entry.thinking;
  if (overwrite || next.thinking_levels == null) {
    next.thinking_levels = entry.thinking_levels.length > 0 ? [...entry.thinking_levels] : [];
  } else {
    next.thinking_levels = uniqueThinkingLevels(next.thinking_levels);
  }
  if (!entry.thinking && (overwrite || model.thinking_enabled == null)) {
    next.thinking_enabled = false;
    if (overwrite || !model.thinking_level) next.thinking_level = null;
    if (overwrite || model.thinking_levels == null) next.thinking_levels = [];
  }
  if (next.thinking_enabled === true) {
    const allowed = uniqueThinkingLevels(next.thinking_levels ?? []);
    next.thinking_level = overwrite
      ? defaultThinkingLevel(allowed, null)
      : defaultThinkingLevel(allowed, next.thinking_level);
  } else if (overwrite || next.thinking_enabled === false) {
    next.thinking_level = null;
  }
  return next;
}

export function channelModelIds(models: AiChannelModel[]): string[] {
  return models.map((item) => item.id).filter((id) => id.trim().length > 0);
}
