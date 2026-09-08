import { afterEach, describe, expect, it, vi } from "vitest";

import {
  installDisableDefaultContextMenu,
  preventDefaultContextMenu,
} from "./disableDefaultContextMenu";

describe("preventDefaultContextMenu", () => {
  it("cancels the default browser / WebView menu", () => {
    const event = new Event("contextmenu", { cancelable: true });
    preventDefaultContextMenu(event);
    expect(event.defaultPrevented).toBe(true);
  });
});

describe("installDisableDefaultContextMenu", () => {
  const listeners = new Map<string, EventListenerOrEventListenerObject>();

  const target: Pick<EventTarget, "addEventListener" | "removeEventListener"> = {
    addEventListener(type, listener, options) {
      expect(type).toBe("contextmenu");
      expect(options).toBe(true);
      listeners.set(type, listener);
    },
    removeEventListener(type, listener, options) {
      expect(type).toBe("contextmenu");
      expect(options).toBe(true);
      if (listeners.get(type) === listener) {
        listeners.delete(type);
      }
    },
  };

  afterEach(() => {
    listeners.clear();
    vi.unstubAllGlobals();
  });

  it("does nothing outside the Tauri desktop shell", () => {
    const uninstall = installDisableDefaultContextMenu(target, false);
    expect(listeners.size).toBe(0);
    uninstall();
    expect(listeners.size).toBe(0);
  });

  it("captures contextmenu in the desktop app and can uninstall", () => {
    const uninstall = installDisableDefaultContextMenu(target, true);
    const listener = listeners.get("contextmenu");
    expect(listener).toEqual(expect.any(Function));

    const event = new Event("contextmenu", { cancelable: true });
    (listener as EventListener)(event);
    expect(event.defaultPrevented).toBe(true);

    uninstall();
    expect(listeners.size).toBe(0);
  });
});
