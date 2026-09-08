import { isTauri } from "@tauri-apps/api/core";

/** 拦截 WebView 默认右键菜单（Back / Reload）。应用内自定义菜单自行 preventDefault 后仍可打开。 */
export function preventDefaultContextMenu(event: Event): void {
  event.preventDefault();
}

export function installDisableDefaultContextMenu(
  target: Pick<EventTarget, "addEventListener" | "removeEventListener"> = document,
  enabled = typeof document !== "undefined" && isTauri(),
): () => void {
  if (!enabled) {
    return () => {};
  }

  const onContextMenu = (event: Event) => {
    preventDefaultContextMenu(event);
  };

  target.addEventListener("contextmenu", onContextMenu, true);
  return () => {
    target.removeEventListener("contextmenu", onContextMenu, true);
  };
}
