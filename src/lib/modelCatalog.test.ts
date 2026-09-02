import { describe, expect, it } from "vitest";

import {
  applyCatalogToModel,
  canSaveChannelModels,
  catalogThinkingLevels,
  defaultThinkingLevel,
  displayedThinkingLevels,
  emptyChannelModel,
  lookupModelCatalog,
  materializeThinkingLevels,
  selectedThinkingLevels,
  withThinkingLevels,
} from "@/lib/modelCatalog";
import type { AiChannelModel, ModelCatalogEntry } from "@/lib/types";

const catalog: ModelCatalogEntry[] = [
  {
    id: "gpt-4o",
    aliases: ["chatgpt-4o-latest"],
    vendor: "openai",
    label: "GPT-4o",
    context_tokens: 128000,
    max_output_tokens: 16384,
    thinking: false,
    thinking_levels: [],
  },
  {
    id: "deepseek-reasoner",
    aliases: ["deepseek-r1"],
    vendor: "deepseek",
    label: "DeepSeek Reasoner",
    context_tokens: 163840,
    max_output_tokens: 65536,
    thinking: true,
    thinking_levels: ["low", "medium", "high"],
  },
  {
    id: "gpt-5.6-luna",
    aliases: [],
    vendor: "openai",
    label: "GPT-5.6 Luna",
    context_tokens: 1000000,
    max_output_tokens: 128000,
    thinking: true,
    thinking_levels: ["low", "medium", "high", "xhigh", "max"],
  },
];

function model(partial: Partial<AiChannelModel> & { id: string }): AiChannelModel {
  return {
    ...emptyChannelModel(partial.id),
    ...partial,
  };
}

describe("model catalog lookup", () => {
  it("matches ids, aliases, and prefixed gateway ids", () => {
    expect(lookupModelCatalog(catalog, "gpt-4o")?.id).toBe("gpt-4o");
    expect(lookupModelCatalog(catalog, "chatgpt-4o-latest")?.id).toBe("gpt-4o");
    expect(lookupModelCatalog(catalog, "deepseek-ai/deepseek-r1")?.id).toBe("deepseek-reasoner");
  });

  it("fills metadata for a new model id", () => {
    const filled = applyCatalogToModel(catalog, emptyChannelModel("deepseek-reasoner"));
    expect(filled.context_tokens).toBe(163840);
    expect(filled.max_output_tokens).toBe(65536);
    expect(filled.thinking_enabled).toBe(true);
    expect(filled.thinking_level).toBe("medium");
    expect(filled.thinking_levels).toEqual(["low", "medium", "high"]);
  });

  it("overwrites thinking levels only when fill-from-catalog is explicit", () => {
    const filled = applyCatalogToModel(
      catalog,
      model({
        id: "gpt-5.6-luna",
        thinking_enabled: true,
        thinking_level: "high",
        thinking_levels: ["low", "high"],
      }),
      true,
    );
    expect(filled.thinking_levels).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(filled.thinking_level).toBe("medium");
  });

  it("keeps an explicit thinking level subset instead of merging catalog additions", () => {
    const filled = applyCatalogToModel(
      catalog,
      model({
        id: "gpt-5.6-luna",
        thinking_enabled: true,
        thinking_level: "high",
        thinking_levels: ["low", "high"],
      }),
    );
    expect(filled.thinking_levels).toEqual(["low", "high"]);
    expect(filled.thinking_level).toBe("high");
  });

  it("keeps an explicit empty thinking level list", () => {
    const filled = applyCatalogToModel(
      catalog,
      model({
        id: "deepseek-reasoner",
        thinking_enabled: false,
        thinking_level: null,
        thinking_levels: [],
      }),
    );
    expect(filled.thinking_levels).toEqual([]);
    expect(filled.thinking_enabled).toBe(false);
    expect(filled.thinking_level).toBeNull();
  });

  it("falls default thinking_level back to medium then the first selected level", () => {
    const withoutPreferred = applyCatalogToModel(
      catalog,
      model({
        id: "deepseek-reasoner",
        thinking_enabled: true,
        thinking_level: "xhigh",
        thinking_levels: ["low", "high"],
      }),
    );
    expect(withoutPreferred.thinking_level).toBe("low");

    const withMedium = applyCatalogToModel(
      catalog,
      model({
        id: "deepseek-reasoner",
        thinking_enabled: true,
        thinking_level: "xhigh",
        thinking_levels: ["low", "medium", "high"],
      }),
    );
    expect(withMedium.thinking_level).toBe("medium");
  });

  it("does not invent thinking for unknown models", () => {
    const filled = applyCatalogToModel(catalog, emptyChannelModel("custom-local-model"));
    expect(filled.thinking_enabled).toBeNull();
    expect(filled.thinking_level).toBeNull();
    expect(filled.thinking_levels).toBeNull();
  });
});

describe("thinking level selection", () => {
  it("treats null thinking_levels as the catalog default set", () => {
    const entry = lookupModelCatalog(catalog, "deepseek-reasoner");
    const unset = emptyChannelModel("deepseek-reasoner");
    expect(selectedThinkingLevels(unset, entry)).toEqual(["low", "medium", "high"]);
    expect(displayedThinkingLevels(unset, catalog)).toEqual([
      "none",
      "no_think",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("shows every known thinking level, not only the model's catalog subset", () => {
    const entry = lookupModelCatalog(catalog, "deepseek-reasoner");
    const custom = model({
      id: "deepseek-reasoner",
      thinking_enabled: true,
      thinking_levels: ["high"],
    });
    expect(selectedThinkingLevels(custom, entry)).toEqual(["high"]);
    expect(displayedThinkingLevels(custom, catalog)).toEqual([
      "none",
      "no_think",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
    expect(catalogThinkingLevels(entry)).toEqual(["low", "medium", "high"]);
  });

  it("keeps custom stored levels in the displayed checkbox list", () => {
    const entry = lookupModelCatalog(catalog, "deepseek-reasoner");
    const custom = model({
      id: "deepseek-reasoner",
      thinking_levels: ["high", "custom"],
    });
    expect(selectedThinkingLevels(custom, entry)).toEqual(["high", "custom"]);
    expect(displayedThinkingLevels(custom, catalog)).toEqual([
      "none",
      "no_think",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
      "custom",
    ]);
  });

  it("appends catalog-only extra levels after the known set", () => {
    const withExtra: ModelCatalogEntry[] = [
      ...catalog,
      {
        id: "future-model",
        aliases: [],
        vendor: "openai",
        label: "Future",
        context_tokens: 128000,
        max_output_tokens: 8192,
        thinking: true,
        thinking_levels: ["low", "ultra"],
      },
    ];
    expect(displayedThinkingLevels(emptyChannelModel("future-model"), withExtra)).toEqual([
      "none",
      "no_think",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
      "ultra",
    ]);
  });

  it("clears the runtime default when thinking is turned off", () => {
    const next = withThinkingLevels(
      model({
        id: "deepseek-reasoner",
        thinking_enabled: true,
        thinking_level: "high",
        thinking_levels: ["low", "high"],
      }),
      ["low", "high"],
      { thinkingEnabled: false },
    );
    expect(next.thinking_enabled).toBe(false);
    expect(next.thinking_level).toBeNull();
    expect(next.thinking_levels).toEqual(["low", "high"]);
  });

  it("materializes catalog defaults when thinking is first enabled from null", () => {
    const next = withThinkingLevels(
      emptyChannelModel("deepseek-reasoner"),
      ["low", "medium", "high"],
      {
        thinkingEnabled: true,
      },
    );
    expect(next.thinking_enabled).toBe(true);
    expect(next.thinking_levels).toEqual(["low", "medium", "high"]);
    expect(next.thinking_level).toBe("medium");
  });

  it("restores a valid default when thinking is turned back on", () => {
    const next = withThinkingLevels(
      model({
        id: "deepseek-reasoner",
        thinking_enabled: false,
        thinking_level: null,
        thinking_levels: ["low", "high"],
      }),
      ["low", "high"],
      { thinkingEnabled: true },
    );
    expect(next.thinking_enabled).toBe(true);
    expect(next.thinking_level).toBe("low");
  });

  it("blocks save when thinking is on but no levels are selected", () => {
    expect(
      canSaveChannelModels(
        [
          model({
            id: "deepseek-reasoner",
            thinking_enabled: true,
            thinking_levels: [],
          }),
        ],
        catalog,
      ),
    ).toBe(false);
    expect(
      canSaveChannelModels(
        [
          model({
            id: "custom-local-model",
            thinking_enabled: true,
            thinking_levels: [],
          }),
        ],
        catalog,
      ),
    ).toBe(false);
    expect(
      canSaveChannelModels(
        [
          model({
            id: "deepseek-reasoner",
            thinking_enabled: true,
            thinking_levels: ["low"],
          }),
        ],
        catalog,
      ),
    ).toBe(true);
    expect(
      canSaveChannelModels(
        [
          model({
            id: "gpt-4o",
            thinking_enabled: false,
            thinking_levels: [],
          }),
        ],
        catalog,
      ),
    ).toBe(true);
  });

  it("allows saving unknown models that still show the fallback thinking set", () => {
    const unknown = model({
      id: "custom-local-model",
      thinking_enabled: true,
      thinking_level: "high",
      thinking_levels: null,
    });
    expect(selectedThinkingLevels(unknown, lookupModelCatalog(catalog, unknown.id))).toEqual([
      "low",
      "medium",
      "high",
    ]);
    expect(canSaveChannelModels([unknown], catalog)).toBe(true);

    const materialized = materializeThinkingLevels(catalog, unknown);
    expect(materialized.thinking_levels).toEqual(["low", "medium", "high"]);
    expect(materialized.thinking_level).toBe("high");
    expect(canSaveChannelModels([materialized], catalog)).toBe(true);
  });

  it("prefers medium when choosing a default thinking level", () => {
    expect(defaultThinkingLevel(["low", "medium", "high"], "xhigh")).toBe("medium");
    expect(defaultThinkingLevel(["low", "high"], "medium")).toBe("low");
    expect(defaultThinkingLevel(["low", "high"], "high")).toBe("high");
    expect(defaultThinkingLevel([], "medium")).toBeNull();
  });
});
