import { describe, expect, it } from "vitest";

import { I18N_NAMESPACES } from "./index";

const locales = ["zh-CN", "en"] as const;

function flattenKeys(value: unknown, prefix = ""): string[] {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
      flattenKeys(child, prefix ? `${prefix}.${key}` : key),
    );
  }
  return [prefix];
}

describe("locale key parity", () => {
  it.each(I18N_NAMESPACES)("%s keys match between zh-CN and en", async (ns) => {
    const zh = (await import(`../../locales/zh-CN/${ns}.json`)).default;
    const en = (await import(`../../locales/en/${ns}.json`)).default;
    expect(flattenKeys(en).sort()).toEqual(flattenKeys(zh).sort());
  });

  it("covers both locale folders", () => {
    expect(locales).toHaveLength(2);
  });
});
