import { Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

export function SidebarFooter() {
  const { t } = useTranslation("nav");
  const navigate = useNavigate();

  return (
    <div className="flex items-center justify-between border-t border-sidebar-border/70 px-3 py-2">
      <span className="text-[11px] font-medium tracking-tight text-muted-foreground/60 select-none">
        noxcode
      </span>
      <button
        type="button"
        className="flex items-center gap-1.5 rounded-lg p-1.5 text-muted-foreground transition-all duration-150 hover:bg-sidebar-accent/80 hover:text-sidebar-foreground"
        onClick={() => void navigate("/settings")}
        title={t("settings")}
        aria-label={t("settings")}
      >
        <Settings className="size-4" />
      </button>
    </div>
  );
}
