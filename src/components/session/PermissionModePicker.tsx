import { ChevronDown, ClipboardList, Hand, ShieldAlert, ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { updateNativeSettings } from "@/lib/backend";
import type { NativePermissionMode } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";

type ComposerPermissionChoice = NativePermissionMode | "plan";

const MODES: ComposerPermissionChoice[] = ["confirm", "auto_edit", "plan", "full"];

function ModeIcon({ mode, className }: { mode: ComposerPermissionChoice; className?: string }) {
  const iconClass = cn("size-3.5 shrink-0", className);
  switch (mode) {
    case "confirm":
      return <Hand className={iconClass} />;
    case "auto_edit":
      return <ShieldCheck className={iconClass} />;
    case "plan":
      return <ClipboardList className={iconClass} />;
    case "full":
      return <ShieldAlert className={iconClass} />;
  }
}

export function PermissionModePicker() {
  const { t } = useTranslation("sessions");
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const planMode = useUiStore((state) => state.composerPlanMode);
  const setPlanMode = useUiStore((state) => state.setComposerPlanMode);
  const persisted: NativePermissionMode = native?.permission_mode ?? "confirm";
  const selected: ComposerPermissionChoice = planMode ? "plan" : persisted;

  const selectMode = (value: string | null) => {
    if (value !== "confirm" && value !== "auto_edit" && value !== "plan" && value !== "full") {
      return;
    }
    if (value === "plan") {
      setPlanMode(true);
      return;
    }
    setPlanMode(false);
    if (value !== persisted) {
      void updateNativeSettings({ permission_mode: value }).then(setNative);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(
          "inline-flex h-7 items-center justify-between gap-1 rounded-md border bg-background px-2 text-xs outline-none",
          selected === "full" && "text-amber-600",
        )}
      >
        <ModeIcon mode={selected} />
        <span className="truncate">{t(`permission.${selected}.title`)}</span>
        <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuRadioGroup value={selected} onValueChange={selectMode}>
          {MODES.map((mode) => (
            <DropdownMenuRadioItem
              key={mode}
              value={mode}
              closeOnClick
              className="items-start py-2"
            >
              <ModeIcon mode={mode} className="mt-0.5" />
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="font-medium">{t(`permission.${mode}.title`)}</span>
                <span className="text-xs text-muted-foreground">
                  {t(`permission.${mode}.description`)}
                </span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
