export const CODE_THEME_IDS = [
  "github-light",
  "github-dark",
  "vitesse-light",
  "vitesse-dark",
  "min-light",
  "min-dark",
  "github-light-high-contrast",
  "github-dark-high-contrast",
  "catppuccin-latte",
  "catppuccin-mocha",
] as const;

export type CodeThemeId = (typeof CODE_THEME_IDS)[number];

export interface CodeThemeOption {
  id: CodeThemeId;
  label: string;
}

export const CODE_THEMES: readonly CodeThemeOption[] = [
  { id: "github-light", label: "GitHub Light" },
  { id: "github-dark", label: "GitHub Dark" },
  { id: "vitesse-light", label: "Vitesse Light" },
  { id: "vitesse-dark", label: "Vitesse Dark" },
  { id: "min-light", label: "Minimal Light" },
  { id: "min-dark", label: "Minimal Dark" },
  { id: "github-light-high-contrast", label: "GitHub HC Light" },
  { id: "github-dark-high-contrast", label: "GitHub HC Dark" },
  { id: "catppuccin-latte", label: "Catppuccin Latte" },
  { id: "catppuccin-mocha", label: "Catppuccin Mocha" },
] as const;

export const DEFAULT_CODE_THEME_LIGHT: CodeThemeId = "github-light";
export const DEFAULT_CODE_THEME_DARK: CodeThemeId = "github-dark";

export function isCodeThemeId(value: string | null | undefined): value is CodeThemeId {
  return Boolean(value && (CODE_THEME_IDS as readonly string[]).includes(value));
}

export function parseCodeThemeId(
  value: string | null | undefined,
  fallback: CodeThemeId,
): CodeThemeId {
  return isCodeThemeId(value) ? value : fallback;
}

export function codeThemeLabel(id: CodeThemeId): string {
  return CODE_THEMES.find((theme) => theme.id === id)?.label ?? id;
}
