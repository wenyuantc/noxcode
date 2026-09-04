import { FolderOpen, Search, SquarePen } from "lucide-react";
import { useTranslation } from "react-i18next";

import { shortcutDisplay, GLOBAL_SHORTCUTS } from "@/lib/shortcuts";
import { useUiStore } from "@/stores/uiStore";

export function SidebarCommands({
  onNewSession,
  onOpenWorkspace,
}: {
  onNewSession: () => void;
  onOpenWorkspace: () => void;
}) {
  const { t } = useTranslation("nav");
  const setCommandOpen = useUiStore((state) => state.setCommandOpen);
  const items = [
    {
      icon: SquarePen,
      label: t("newSession"),
      display: shortcutDisplay(GLOBAL_SHORTCUTS[0]!),
      onClick: onNewSession,
    },
    {
      icon: Search,
      label: t("search"),
      display: shortcutDisplay(GLOBAL_SHORTCUTS[1]!),
      onClick: () => setCommandOpen(true),
    },
    {
      icon: FolderOpen,
      label: t("shortcuts.openWorkspace"),
      display: shortcutDisplay(GLOBAL_SHORTCUTS[2]!),
      onClick: onOpenWorkspace,
    },
  ];

  return (
    <div className="space-y-1 px-2.5 pt-2 pb-1.5">
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          onClick={item.onClick}
          className="group flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-sidebar-foreground/90 transition-all duration-150 hover:bg-sidebar-accent/80 hover:text-sidebar-foreground active:scale-[0.99]"
        >
          <item.icon className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-sidebar-foreground" />
          <span className="flex-1 text-left tracking-tight">{item.label}</span>
          <kbd className="inline-flex items-center rounded border border-sidebar-border/80 bg-background/50 px-1.5 py-0.5 font-mono text-[10px] font-medium text-muted-foreground shadow-xs">
            {item.display}
          </kbd>
        </button>
      ))}
    </div>
  );
}
