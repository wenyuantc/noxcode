import { ChevronDown, ClipboardList, Hand, Hammer, ShieldAlert, ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useState } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { updateNativeSettings } from "@/lib/backend";
import { applyComposerPlanMode, resolveComposerPlanMode } from "@/lib/planMode";
import { isNativePermissionMode, type NativePermissionMode } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useSettingsStore } from "@/stores/settingsStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { changeSessionConfiguration, finishIdleSession } from "@/lib/sessionConfiguration";

type ComposerPermissionChoice = NativePermissionMode | "plan";

const MODES: ComposerPermissionChoice[] = ["default", "edit", "build", "plan", "yolo"];

function ModeIcon({ mode, className }: { mode: ComposerPermissionChoice; className?: string }) {
  const iconClass = cn("size-3.5 shrink-0", className);
  switch (mode) {
    case "default":
      return <Hand className={iconClass} />;
    case "edit":
      return <ShieldCheck className={iconClass} />;
    case "build":
      return <Hammer className={iconClass} />;
    case "plan":
      return <ClipboardList className={iconClass} />;
    case "yolo":
      return <ShieldAlert className={iconClass} />;
  }
}

export function PermissionModePicker({
  onError,
  disabled = false,
}: {
  onError?: (error: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const native = useSettingsStore((state) => state.native);
  const setNative = useSettingsStore((state) => state.setNative);
  const defaultPlanMode = useUiStore((state) => state.composerPlanMode);
  const setPlanMode = useUiStore((state) => state.setComposerPlanMode);
  const selectedSessionId = useSessionStore((state) => state.selectedSessionId);
  const runtime = useSessionStore((state) =>
    selectedSessionId ? state.configurationBySession[selectedSessionId] : undefined,
  );
  const [busy, setBusy] = useState(false);
  const planModeBySession = useSessionStore((state) => state.planModeBySession);
  const planMode = resolveComposerPlanMode(selectedSessionId, planModeBySession, defaultPlanMode);
  const runtimePermission = runtime?.permission_mode ?? native?.permission_mode;
  const persisted: NativePermissionMode = isNativePermissionMode(runtimePermission)
    ? runtimePermission
    : "default";
  const selected: ComposerPermissionChoice = planMode ? "plan" : persisted;

  const writePlanMode = (enabled: boolean) => {
    applyComposerPlanMode({
      enabled,
      sessionId: selectedSessionId,
      setDefault: setPlanMode,
      setSession: (id, next) => useSessionStore.getState().onPlanMode(id, next),
    });
  };

  const selectMode = async (value: string | null) => {
    if (busy || disabled || (value !== "plan" && !isNativePermissionMode(value))) return;
    setBusy(true);
    try {
      if (selectedSessionId) await finishIdleSession(selectedSessionId);
      if (value === "plan") {
        await changeSessionConfiguration(selectedSessionId, { plan_mode: true });
        writePlanMode(true);
        return;
      }
      if (!isNativePermissionMode(value)) {
        return;
      }
      if (!runtime && value !== persisted) {
        setNative(await updateNativeSettings({ permission_mode: value }));
      }
      await changeSessionConfiguration(selectedSessionId, {
        permission_mode: value,
        plan_mode: false,
      });
      writePlanMode(false);
    } catch (reason) {
      onError?.(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled || busy}
        className={cn(
          "inline-flex h-7 cursor-pointer items-center justify-between gap-1.5 rounded-lg border border-border/70 bg-background/80 px-2 text-xs font-medium shadow-2xs transition-all duration-150 outline-none hover:bg-muted/40",
          selected === "yolo" && "text-amber-500 font-semibold",
        )}
      >
        <ModeIcon mode={selected} />
        <span className="truncate">{t(`permission.${selected}.title`)}</span>
        <ChevronDown className="size-3 shrink-0 text-muted-foreground/70" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuRadioGroup
          value={selected}
          onValueChange={(value) => {
            void selectMode(value);
          }}
        >
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
