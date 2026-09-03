export type ThemeMode = "light" | "dark" | "system";

const THEME_STORAGE_KEY = "theme";
const THEME_MODE_STORAGE_KEY = "theme-mode";

export const THEME_CHANGE_EVENT = "noxcode:theme-change";

function isThemeMode(value: string | null): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

function getSystemPrefersDark() {
  return typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function getThemePreference(): ThemeMode {
  if (typeof window === "undefined") return "system";
  const storedMode = localStorage.getItem(THEME_MODE_STORAGE_KEY);
  if (isThemeMode(storedMode)) return storedMode;
  const legacyTheme = localStorage.getItem(THEME_STORAGE_KEY);
  if (legacyTheme === "light" || legacyTheme === "dark") return legacyTheme;
  return "system";
}

export function isDarkThemeMode(mode: ThemeMode) {
  if (mode === "system") return getSystemPrefersDark();
  return mode === "dark";
}

export function applyTheme(mode: ThemeMode) {
  const isDark = isDarkThemeMode(mode);
  document.documentElement.classList.toggle("dark", isDark);
  document.documentElement.style.colorScheme = isDark ? "dark" : "light";
  localStorage.setItem(THEME_MODE_STORAGE_KEY, mode);
  localStorage.setItem(THEME_STORAGE_KEY, isDark ? "dark" : "light");
  window.dispatchEvent(
    new CustomEvent(THEME_CHANGE_EVENT, {
      detail: { mode, isDark },
    }),
  );
  return isDark;
}

export function cycleTheme(mode: ThemeMode): ThemeMode {
  if (mode === "system") return "light";
  if (mode === "light") return "dark";
  return "system";
}
