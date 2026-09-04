import { PanelLeft } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Composer } from "@/components/session/Composer";
import { Button } from "@/components/ui/button";
import { GLOBAL_SHORTCUTS, shortcutDisplay } from "@/lib/shortcuts";
import { greetingPeriod } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";
import { QuickPromptChips } from "./QuickPromptChips";

export function HomeEmptyState() {
  const { t } = useTranslation("layout");
  const { t: tNav } = useTranslation("nav");
  const collapsed = useUiStore((state) => state.sidebarCollapsed);
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);
  const sidebarShortcut = GLOBAL_SHORTCUTS.find((s) => s.id === "toggle-sidebar");
  const shortcutHint = sidebarShortcut ? ` (${shortcutDisplay(sidebarShortcut)})` : "";

  return (
    <div className="relative flex h-full flex-col items-center justify-center gap-8 px-6">
      {collapsed ? (
        <div className="absolute left-4 top-3">
          <Button
            size="icon-sm"
            variant="ghost"
            className="h-7 w-7 rounded-lg text-muted-foreground transition-all hover:text-foreground"
            title={`${tNav("shortcuts.toggleSidebar")}${shortcutHint}`}
            onClick={toggleSidebar}
          >
            <PanelLeft className="size-4" />
          </Button>
        </div>
      ) : null}
      <h1 className="text-2xl font-medium tracking-tight">{t(`greeting.${greetingPeriod()}`)}</h1>
      <Composer />
      <QuickPromptChips />
    </div>
  );
}
