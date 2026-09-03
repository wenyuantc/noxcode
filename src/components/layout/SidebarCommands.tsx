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
    <div className="space-y-0.5 px-2 py-2">
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          onClick={item.onClick}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-sidebar-accent"
        >
          <item.icon className="size-4 text-muted-foreground" />
          <span className="flex-1 text-left">{item.label}</span>
          <kbd className="text-[10px] text-muted-foreground">{item.display}</kbd>
        </button>
      ))}
    </div>
  );
}
