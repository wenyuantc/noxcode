import { describe, expect, it } from "vitest";

import {
  CODE_FONT_SIZE_MAX,
  CODE_FONT_SIZE_MIN,
  DEFAULT_CODE_APPEARANCE,
  UI_FONT_SIZE_MAX,
  UI_FONT_SIZE_MIN,
  clampCodeFontSize,
  clampUiFontSize,
  persistCodeAppearance,
  readCodeAppearance,
  resolveActiveCodeTheme,
  type AppearanceStorage,
} from "./codeAppearance";

function memoryStorage(initial: Record<string, string> = {}): AppearanceStorage {
  const data = { ...initial };
  return {
    getItem(key) {
      return data[key] ?? null;
    },
    setItem(key, value) {
      data[key] = value;
    },
  };
}

describe("codeAppearance", () => {
  it("returns defaults when storage is empty", () => {
    expect(readCodeAppearance(memoryStorage())).toEqual(DEFAULT_CODE_APPEARANCE);
  });

  it("reads and persists a complete appearance", () => {
    const storage = memoryStorage();
    persistCodeAppearance(
      {
        uiFontSize: 16,
        codeThemeLight: "vitesse-light",
        codeThemeDark: "catppuccin-mocha",
        codeLineNumbers: false,
        codeSoftWrap: true,
        codeFontSize: 14,
      },
      storage,
    );
    expect(readCodeAppearance(storage)).toEqual({
      uiFontSize: 16,
      codeThemeLight: "vitesse-light",
      codeThemeDark: "catppuccin-mocha",
      codeLineNumbers: false,
      codeSoftWrap: true,
      codeFontSize: 14,
    });
  });

  it("falls back on invalid stored values", () => {
    const storage = memoryStorage({
      "noxcode:ui-font-size": "not-a-number",
      "noxcode:code-theme-light": "monokai",
      "noxcode:code-theme-dark": "",
      "noxcode:code-line-numbers": "maybe",
      "noxcode:code-soft-wrap": "maybe",
      "noxcode:code-font-size": "999",
    });
    expect(readCodeAppearance(storage)).toEqual({
      ...DEFAULT_CODE_APPEARANCE,
      codeFontSize: CODE_FONT_SIZE_MAX,
    });
  });

  it("clamps font sizes to the documented range", () => {
    expect(clampUiFontSize(Number.NaN)).toBe(DEFAULT_CODE_APPEARANCE.uiFontSize);
    expect(clampUiFontSize(3)).toBe(UI_FONT_SIZE_MIN);
    expect(clampUiFontSize(40)).toBe(UI_FONT_SIZE_MAX);
    expect(clampCodeFontSize(Number.NaN)).toBe(DEFAULT_CODE_APPEARANCE.codeFontSize);
    expect(clampCodeFontSize(2)).toBe(CODE_FONT_SIZE_MIN);
    expect(clampCodeFontSize(40)).toBe(CODE_FONT_SIZE_MAX);
  });

  it("resolves the active code theme from the effective interface mode", () => {
    const appearance = {
      codeThemeLight: "catppuccin-latte" as const,
      codeThemeDark: "vitesse-dark" as const,
    };
    expect(resolveActiveCodeTheme(appearance, false)).toBe("catppuccin-latte");
    expect(resolveActiveCodeTheme(appearance, true)).toBe("vitesse-dark");
  });
});
