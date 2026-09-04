import {
  DEFAULT_CODE_THEME_DARK,
  DEFAULT_CODE_THEME_LIGHT,
  parseCodeThemeId,
  type CodeThemeId,
} from "./codeThemes";
import { isDarkThemeMode, type ThemeMode } from "./theme";

export const UI_FONT_SIZE_KEY = "noxcode:ui-font-size";
export const CODE_THEME_LIGHT_KEY = "noxcode:code-theme-light";
export const CODE_THEME_DARK_KEY = "noxcode:code-theme-dark";
export const CODE_LINE_NUMBERS_KEY = "noxcode:code-line-numbers";
export const CODE_SOFT_WRAP_KEY = "noxcode:code-soft-wrap";
export const CODE_FONT_SIZE_KEY = "noxcode:code-font-size";

export const UI_FONT_SIZE_MIN = 12;
export const UI_FONT_SIZE_MAX = 20;
export const CODE_FONT_SIZE_MIN = 10;
export const CODE_FONT_SIZE_MAX = 22;

export interface CodeAppearance {
  uiFontSize: number;
  codeThemeLight: CodeThemeId;
  codeThemeDark: CodeThemeId;
  codeLineNumbers: boolean;
  codeSoftWrap: boolean;
  codeFontSize: number;
}

export const DEFAULT_CODE_APPEARANCE: CodeAppearance = {
  uiFontSize: 14,
  codeThemeLight: DEFAULT_CODE_THEME_LIGHT,
  codeThemeDark: DEFAULT_CODE_THEME_DARK,
  codeLineNumbers: true,
  codeSoftWrap: false,
  codeFontSize: 12,
};

export interface AppearanceStorage {
  getItem(key: string): string | null;
  setItem?(key: string, value: string): void;
}

function readStorage(storage?: AppearanceStorage | null): AppearanceStorage | null {
  if (storage) return storage;
  if (typeof window === "undefined") return null;
  return window.localStorage;
}

export function clampInt(value: number, min: number, max: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function clampUiFontSize(value: number): number {
  return clampInt(value, UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX, DEFAULT_CODE_APPEARANCE.uiFontSize);
}

export function clampCodeFontSize(value: number): number {
  return clampInt(
    value,
    CODE_FONT_SIZE_MIN,
    CODE_FONT_SIZE_MAX,
    DEFAULT_CODE_APPEARANCE.codeFontSize,
  );
}

function parseStoredNumber(raw: string | null, fallback: number): number {
  if (raw == null || raw.trim() === "") return fallback;
  return Number(raw);
}

function parseStoredFlag(raw: string | null, fallback: boolean): boolean {
  if (raw == null) return fallback;
  if (raw === "1") return true;
  if (raw === "0") return false;
  return fallback;
}

export function readCodeAppearance(storage?: AppearanceStorage | null): CodeAppearance {
  const store = readStorage(storage);
  if (!store) return { ...DEFAULT_CODE_APPEARANCE };

  return {
    uiFontSize: clampUiFontSize(
      parseStoredNumber(store.getItem(UI_FONT_SIZE_KEY), DEFAULT_CODE_APPEARANCE.uiFontSize),
    ),
    codeThemeLight: parseCodeThemeId(
      store.getItem(CODE_THEME_LIGHT_KEY),
      DEFAULT_CODE_APPEARANCE.codeThemeLight,
    ),
    codeThemeDark: parseCodeThemeId(
      store.getItem(CODE_THEME_DARK_KEY),
      DEFAULT_CODE_APPEARANCE.codeThemeDark,
    ),
    codeLineNumbers: parseStoredFlag(
      store.getItem(CODE_LINE_NUMBERS_KEY),
      DEFAULT_CODE_APPEARANCE.codeLineNumbers,
    ),
    codeSoftWrap: parseStoredFlag(
      store.getItem(CODE_SOFT_WRAP_KEY),
      DEFAULT_CODE_APPEARANCE.codeSoftWrap,
    ),
    codeFontSize: clampCodeFontSize(
      parseStoredNumber(store.getItem(CODE_FONT_SIZE_KEY), DEFAULT_CODE_APPEARANCE.codeFontSize),
    ),
  };
}

export function persistCodeAppearance(
  appearance: CodeAppearance,
  storage?: AppearanceStorage | null,
) {
  const store = readStorage(storage);
  if (!store?.setItem) return;
  store.setItem(UI_FONT_SIZE_KEY, String(appearance.uiFontSize));
  store.setItem(CODE_THEME_LIGHT_KEY, appearance.codeThemeLight);
  store.setItem(CODE_THEME_DARK_KEY, appearance.codeThemeDark);
  store.setItem(CODE_LINE_NUMBERS_KEY, appearance.codeLineNumbers ? "1" : "0");
  store.setItem(CODE_SOFT_WRAP_KEY, appearance.codeSoftWrap ? "1" : "0");
  store.setItem(CODE_FONT_SIZE_KEY, String(appearance.codeFontSize));
}

export function applyCodeAppearance(appearance: CodeAppearance) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.style.setProperty("--ui-font-size", `${appearance.uiFontSize}px`);
  root.style.setProperty("--code-font-size", `${appearance.codeFontSize}px`);
  root.dataset.codeWrap = appearance.codeSoftWrap ? "1" : "0";
  root.dataset.codeLineNumbers = appearance.codeLineNumbers ? "1" : "0";
}

export function resolveActiveCodeTheme(
  appearance: Pick<CodeAppearance, "codeThemeLight" | "codeThemeDark">,
  isDark: boolean,
): CodeThemeId {
  return isDark ? appearance.codeThemeDark : appearance.codeThemeLight;
}

export function resolveActiveCodeThemeFromMode(
  appearance: Pick<CodeAppearance, "codeThemeLight" | "codeThemeDark">,
  mode: ThemeMode,
): CodeThemeId {
  return resolveActiveCodeTheme(appearance, isDarkThemeMode(mode));
}
