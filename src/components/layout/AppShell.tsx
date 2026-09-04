import { useEffect, useRef } from "react";

import { CommandPalette } from "@/components/command/CommandPalette";
import { HomeEmptyState } from "@/components/home/HomeEmptyState";
import { SessionView } from "@/components/session/SessionView";
import { useAppHotkeys } from "@/hooks/useAppHotkeys";
import { openLocalWorkspace } from "@/lib/openLocalWorkspace";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { SidebarCommands } from "./SidebarCommands";
import { SidebarFooter } from "./SidebarFooter";
import { SidebarTree } from "./SidebarTree";

export function AppShell() {
  const collapsed = useUiStore((state) => state.sidebarCollapsed);
  const width = useUiStore((state) => state.sidebarWidth);
  const setWidth = useUiStore((state) => state.setSidebarWidth);
  const selected = useSessionStore((state) => state.selectedSessionId);
  const dragging = useRef(false);

  useEffect(() => {
    void Promise.all([
      useWorkspaceStore.getState().load(),
      useChannelStore.getState().load(),
      useSettingsStore.getState().load(),
    ]);
  }, []);

  const newSession = () => {
    useSessionStore.getState().selectSession(null);
    useUiStore.getState().setComposerDraft("");
  };

  const openWorkspace = () => {
    void openLocalWorkspace();
  };

  useAppHotkeys(newSession, openWorkspace);

  return (
    <div className="flex h-full overflow-hidden bg-background">
      {!collapsed ? (
        <aside
          className="flex shrink-0 flex-col border-r border-sidebar-border bg-sidebar"
          style={{ width }}
        >
          <SidebarCommands onNewSession={newSession} onOpenWorkspace={openWorkspace} />
          <SidebarTree />
          <SidebarFooter />
        </aside>
      ) : null}
      {!collapsed ? (
        <div
          className="w-1 cursor-col-resize hover:bg-ring/40"
          onMouseDown={() => {
            dragging.current = true;
            const onMove = (event: MouseEvent) => {
              if (dragging.current) setWidth(event.clientX);
            };
            const onUp = () => {
              dragging.current = false;
              window.removeEventListener("mousemove", onMove);
              window.removeEventListener("mouseup", onUp);
            };
            window.addEventListener("mousemove", onMove);
            window.addEventListener("mouseup", onUp);
          }}
        />
      ) : null}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="min-h-0 min-w-0 flex-1 overflow-hidden">
          {selected ? <SessionView /> : <HomeEmptyState />}
        </div>
      </main>
      <CommandPalette onNewSession={newSession} onOpenWorkspace={openWorkspace} />
    </div>
  );
}
