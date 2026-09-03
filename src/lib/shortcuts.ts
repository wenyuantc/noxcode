export type ShortcutCategory = "global" | "session";

export interface ShortcutDef {
  id: string;
  keys: string;
  display: string;
  descriptionKey: string;
  category: ShortcutCategory;
}

export const GLOBAL_SHORTCUTS: ShortcutDef[] = [
  {
    id: "new-session",
    keys: "meta+n",
    display: "⌘N",
    descriptionKey: "shortcuts.newSession",
    category: "global",
  },
  {
    id: "command-palette",
    keys: "meta+k",
    display: "⌘K",
    descriptionKey: "shortcuts.commandPalette",
    category: "global",
  },
  {
    id: "open-workspace",
    keys: "meta+o",
    display: "⌘O",
    descriptionKey: "shortcuts.openWorkspace",
    category: "global",
  },
  {
    id: "toggle-sidebar",
    keys: "meta+b",
    display: "⌘B",
    descriptionKey: "shortcuts.toggleSidebar",
    category: "global",
  },
  {
    id: "toggle-git",
    keys: "meta+shift+g",
    display: "⌘⇧G",
    descriptionKey: "shortcuts.toggleGit",
    category: "session",
  },
];

export function isMac(): boolean {
  return typeof navigator !== "undefined" && navigator.platform.includes("Mac");
}

export function shortcutKeys(def: ShortcutDef): string {
  if (isMac()) return def.keys;
  return def.keys.replace(/meta/g, "ctrl");
}

export function shortcutDisplay(def: ShortcutDef): string {
  if (isMac()) return def.display;
  return def.display.replace(/⌘/g, "Ctrl+").replace(/⇧/g, "Shift+");
}
