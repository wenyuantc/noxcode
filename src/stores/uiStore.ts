import { create } from "zustand";

import {
  applyCodeAppearance,
  clampCodeFontSize,
  clampUiFontSize,
  persistCodeAppearance,
  readCodeAppearance,
  type CodeAppearance,
} from "@/lib/codeAppearance";
import type { CodeThemeId } from "@/lib/codeThemes";
import {
  applyTheme,
  cycleTheme,
  getThemePreference,
  isDarkThemeMode,
  type ThemeMode,
} from "@/lib/theme";

const WIDTH_KEY = "noxcode:sidebar-width";
const COLLAPSED_KEY = "noxcode:sidebar-collapsed";
const PLAN_MODE_KEY = "noxcode:composer-plan-mode";
const THINKING_LEVEL_KEY = "noxcode:composer-thinking-level";

function readNumber(key: string, fallback: number) {
  const raw = typeof window === "undefined" ? null : window.localStorage.getItem(key);
  const value = raw ? Number(raw) : fallback;
  return Number.isFinite(value) ? value : fallback;
}

function readThinkingLevel(): string | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(THINKING_LEVEL_KEY)?.trim();
  return raw || null;
}

const initialAppearance = readCodeAppearance();

interface UiState extends CodeAppearance {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  commandOpen: boolean;
  gitOpen: boolean;
  gitFocusPath: string | null;
  composerDraft: string;
  composerPlanMode: boolean;
  composerThinkingLevel: string | null;
  theme: ThemeMode;
  isDark: boolean;
  setSidebarWidth: (width: number) => void;
  toggleSidebar: () => void;
  setCommandOpen: (open: boolean) => void;
  toggleGit: () => void;
  openGitPreview: (path: string | null) => void;
  setComposerDraft: (value: string) => void;
  setComposerPlanMode: (value: boolean) => void;
  setComposerThinkingLevel: (value: string | null) => void;
  setTheme: (mode: ThemeMode) => void;
  cycleTheme: () => void;
  setIsDark: (isDark: boolean) => void;
  setUiFontSize: (value: number) => void;
  setCodeThemeLight: (value: CodeThemeId) => void;
  setCodeThemeDark: (value: CodeThemeId) => void;
  setCodeLineNumbers: (value: boolean) => void;
  setCodeSoftWrap: (value: boolean) => void;
  setCodeFontSize: (value: number) => void;
}

function appearanceFromState(state: UiState): CodeAppearance {
  return {
    uiFontSize: state.uiFontSize,
    codeThemeLight: state.codeThemeLight,
    codeThemeDark: state.codeThemeDark,
    codeLineNumbers: state.codeLineNumbers,
    codeSoftWrap: state.codeSoftWrap,
    codeFontSize: state.codeFontSize,
  };
}

function commitAppearance(partial: Partial<CodeAppearance>) {
  useUiStore.setState((state) => {
    const next = { ...appearanceFromState(state), ...partial };
    persistCodeAppearance(next);
    applyCodeAppearance(next);
    return next;
  });
}

export const useUiStore = create<UiState>((set, get) => ({
  sidebarWidth: Math.min(480, Math.max(200, readNumber(WIDTH_KEY, 260))),
  sidebarCollapsed: typeof window !== "undefined" && localStorage.getItem(COLLAPSED_KEY) === "1",
  commandOpen: false,
  gitOpen: false,
  gitFocusPath: null,
  composerDraft: "",
  composerPlanMode: typeof window !== "undefined" && localStorage.getItem(PLAN_MODE_KEY) === "1",
  composerThinkingLevel: readThinkingLevel(),
  theme: getThemePreference(),
  isDark: isDarkThemeMode(getThemePreference()),
  ...initialAppearance,
  setSidebarWidth: (width) => {
    const next = Math.min(480, Math.max(200, width));
    localStorage.setItem(WIDTH_KEY, String(next));
    set({ sidebarWidth: next });
  },
  toggleSidebar: () => {
    const next = !get().sidebarCollapsed;
    localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0");
    set({ sidebarCollapsed: next });
  },
  setCommandOpen: (open) => set({ commandOpen: open }),
  toggleGit: () => set({ gitOpen: !get().gitOpen }),
  openGitPreview: (path) => set({ gitOpen: true, gitFocusPath: path }),
  setComposerDraft: (value) => set({ composerDraft: value }),
  setComposerPlanMode: (value) => {
    localStorage.setItem(PLAN_MODE_KEY, value ? "1" : "0");
    set({ composerPlanMode: value });
  },
  setComposerThinkingLevel: (value) => {
    const next = value?.trim() || null;
    if (next) localStorage.setItem(THINKING_LEVEL_KEY, next);
    else localStorage.removeItem(THINKING_LEVEL_KEY);
    set({ composerThinkingLevel: next });
  },
  setTheme: (mode) => {
    const isDark = applyTheme(mode);
    set({ theme: mode, isDark });
  },
  cycleTheme: () => {
    const mode = cycleTheme(get().theme);
    const isDark = applyTheme(mode);
    set({ theme: mode, isDark });
  },
  setIsDark: (isDark) => set({ isDark }),
  setUiFontSize: (value) => commitAppearance({ uiFontSize: clampUiFontSize(value) }),
  setCodeThemeLight: (value) => commitAppearance({ codeThemeLight: value }),
  setCodeThemeDark: (value) => commitAppearance({ codeThemeDark: value }),
  setCodeLineNumbers: (value) => commitAppearance({ codeLineNumbers: value }),
  setCodeSoftWrap: (value) => commitAppearance({ codeSoftWrap: value }),
  setCodeFontSize: (value) => commitAppearance({ codeFontSize: clampCodeFontSize(value) }),
}));
