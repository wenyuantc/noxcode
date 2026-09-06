import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let data: Map<string, string>;

beforeEach(() => {
  vi.resetModules();
  data = new Map();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => data.set(key, value),
    removeItem: (key: string) => data.delete(key),
  };
  vi.stubGlobal("localStorage", storage);
  vi.stubGlobal("window", { localStorage: storage, matchMedia: () => ({ matches: false }) });
});

afterEach(() => vi.unstubAllGlobals());

describe("Git UI preferences", () => {
  it("defaults to 380 pixels and persists a clamped width", async () => {
    const { useUiStore } = await import("./uiStore");
    expect(useUiStore.getState().gitPanelWidth).toBe(380);
    useUiStore.getState().setGitPanelWidth(620);
    expect(data.get("noxcode:git-panel-width")).toBe("620");
    useUiStore.getState().setGitPanelWidth(1000);
    expect(useUiStore.getState().gitPanelWidth).toBe(800);
    useUiStore.getState().setGitPanelWidth(20);
    expect(data.get("noxcode:git-panel-width")).toBe("320");
    expect(data.has("noxcode:sidebar-width")).toBe(false);
  });

  it.each([
    ["650", 650],
    ["900", 800],
    ["broken", 380],
    ["Infinity", 380],
  ])("restores the saved preference %s as %s", async (stored, expected) => {
    data.set("noxcode:git-panel-width", String(stored));
    const { useUiStore } = await import("./uiStore");
    expect(useUiStore.getState().gitPanelWidth).toBe(expected);
  });

  it("consumes previews without closing the panel and allows the same file again", async () => {
    const { useUiStore } = await import("./uiStore");
    useUiStore.getState().openGitPreview("src/main.ts");
    expect(useUiStore.getState().gitFocusPath).toBe("src/main.ts");
    useUiStore.getState().clearGitPreview();
    expect(useUiStore.getState().gitOpen).toBe(true);
    expect(useUiStore.getState().gitFocusPath).toBeNull();
    useUiStore.getState().openGitPreview("src/main.ts");
    expect(useUiStore.getState().gitFocusPath).toBe("src/main.ts");
    useUiStore.getState().toggleGit();
    useUiStore.getState().toggleGit();
    expect(useUiStore.getState().gitFocusPath).toBeNull();
  });
});
