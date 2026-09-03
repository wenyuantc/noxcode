import { useHotkeys } from "react-hotkeys-hook";
import { useNavigate } from "react-router-dom";

import { shortcutKeys, GLOBAL_SHORTCUTS } from "@/lib/shortcuts";
import { useUiStore } from "@/stores/uiStore";

export function useAppHotkeys(onNewSession: () => void, onOpenWorkspace: () => void) {
  const navigate = useNavigate();
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);
  const setCommandOpen = useUiStore((state) => state.setCommandOpen);
  const toggleGit = useUiStore((state) => state.toggleGit);

  const find = (id: string) => GLOBAL_SHORTCUTS.find((item) => item.id === id)!;

  useHotkeys(shortcutKeys(find("command-palette")), (event) => {
    event.preventDefault();
    setCommandOpen(true);
  });
  useHotkeys(shortcutKeys(find("new-session")), (event) => {
    event.preventDefault();
    onNewSession();
  });
  useHotkeys(shortcutKeys(find("open-workspace")), (event) => {
    event.preventDefault();
    onOpenWorkspace();
  });
  useHotkeys(shortcutKeys(find("toggle-sidebar")), (event) => {
    event.preventDefault();
    toggleSidebar();
  });
  useHotkeys(shortcutKeys(find("toggle-git")), (event) => {
    event.preventDefault();
    toggleGit();
  });
  useHotkeys("meta+,", (event) => {
    event.preventDefault();
    void navigate("/settings");
  });
}
