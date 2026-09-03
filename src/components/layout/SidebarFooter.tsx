import { Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

export function SidebarFooter() {
  const { t } = useTranslation("nav");
  const navigate = useNavigate();

  return (
    <div className="flex items-center justify-end gap-2 border-t border-sidebar-border px-3 py-2">
      <button
        type="button"
        className="rounded-md p-1 hover:bg-sidebar-accent"
        onClick={() => void navigate("/settings")}
        title={t("settings")}
      >
        <Settings className="size-4" />
      </button>
    </div>
  );
}
