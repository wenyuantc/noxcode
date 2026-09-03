import { create } from "zustand";

import { applyTheme, cycleTheme, getThemePreference, type ThemeMode } from "@/lib/theme";

const WIDTH_KEY = "noxcode:sidebar-width";
const COLLAPSED_KEY = "noxcode:sidebar-collapsed";
const PLAN_MODE_KEY = "noxcode:composer-plan-mode";

function readNumber(key: string, fallback: number) {
  const raw = typeof window === "undefined" ? null : window.localStorage.getItem(key);
  const value = raw ? Number(raw) : fallback;
  return Number.isFinite(value) ? value : fallback;
}

interface UiState {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  commandOpen: boolean;
  gitOpen: boolean;
  gitFocusPath: string | null;
  composerDraft: string;
  composerPlanMode: boolean;
  theme: ThemeMode;
  setSidebarWidth: (width: number) => void;
  toggleSidebar: () => void;
  setCommandOpen: (open: boolean) => void;
  toggleGit: () => void;
  openGitPreview: (path: string | null) => void;
  setComposerDraft: (value: string) => void;
  setComposerPlanMode: (value: boolean) => void;
  setTheme: (mode: ThemeMode) => void;
  cycleTheme: () => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  sidebarWidth: Math.min(480, Math.max(200, readNumber(WIDTH_KEY, 260))),
  sidebarCollapsed: typeof window !== "undefined" && localStorage.getItem(COLLAPSED_KEY) === "1",
  commandOpen: false,
  gitOpen: false,
  gitFocusPath: null,
  composerDraft: "",
  composerPlanMode: typeof window !== "undefined" && localStorage.getItem(PLAN_MODE_KEY) === "1",
  theme: getThemePreference(),
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
  setTheme: (mode) => {
    applyTheme(mode);
    set({ theme: mode });
  },
  cycleTheme: () => {
    const mode = cycleTheme(get().theme);
    applyTheme(mode);
    set({ theme: mode });
  },
}));
