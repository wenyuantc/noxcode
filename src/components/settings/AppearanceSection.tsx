import { Check, Laptop, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";
import { SettingCard } from "./SettingCard";

interface ThemeOption {
  id: ThemeMode;
  labelKey: string;
  descKey: string;
  icon: typeof Sun;
}

const THEME_OPTIONS: ThemeOption[] = [
  {
    id: "system",
    labelKey: "appearance.system",
    descKey: "appearance.systemHint",
    icon: Laptop,
  },
  {
    id: "light",
    labelKey: "appearance.light",
    descKey: "appearance.lightHint",
    icon: Sun,
  },
  {
    id: "dark",
    labelKey: "appearance.dark",
    descKey: "appearance.darkHint",
    icon: Moon,
  },
];

export function AppearanceSection() {
  const { t } = useTranslation("settings");
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);

  return (
    <div className="space-y-6">
      <SettingCard
        title={t("appearance.theme")}
        description={t("appearance.themeHint")}
        badge={t(`appearance.${theme}`)}
      >
        <div
          role="radiogroup"
          aria-label={t("appearance.theme")}
          className="grid grid-cols-1 gap-3 sm:grid-cols-3"
        >
          {THEME_OPTIONS.map((option) => {
            const isSelected = theme === option.id;
            const Icon = option.icon;

            return (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={isSelected}
                onClick={() => setTheme(option.id)}
                className={cn(
                  "group relative flex flex-col items-start gap-3 rounded-xl border p-4 text-left transition-all duration-150 active:scale-[0.98]",
                  isSelected
                    ? "border-primary bg-primary/5 shadow-xs ring-2 ring-primary/20"
                    : "border-border/70 bg-card hover:border-border hover:bg-muted/40",
                )}
              >
                {/* 模拟窗口微缩图 */}
                <div className="relative flex h-20 w-full overflow-hidden rounded-lg border border-border/60 p-2 shadow-2xs">
                  {option.id === "system" ? (
                    <div className="flex h-full w-full overflow-hidden rounded-md">
                      {/* 左侧浅色半边 */}
                      <div className="flex h-full w-1/2 flex-col gap-1 bg-neutral-100 p-1.5 border-r border-neutral-300">
                        <div className="h-1.5 w-6 rounded-full bg-neutral-300" />
                        <div className="h-1.5 w-full rounded-xs bg-neutral-200" />
                        <div className="h-1.5 w-4/5 rounded-xs bg-neutral-200" />
                      </div>
                      {/* 右侧深色半边 */}
                      <div className="flex h-full w-1/2 flex-col gap-1 bg-neutral-900 p-1.5">
                        <div className="h-1.5 w-6 rounded-full bg-neutral-700" />
                        <div className="h-1.5 w-full rounded-xs bg-neutral-800" />
                        <div className="h-1.5 w-4/5 rounded-xs bg-neutral-800" />
                      </div>
                    </div>
                  ) : option.id === "light" ? (
                    <div className="flex h-full w-full flex-col gap-1.5 rounded-md bg-neutral-50 p-2 border border-neutral-200">
                      <div className="flex items-center gap-1">
                        <div className="size-1.5 rounded-full bg-red-400" />
                        <div className="size-1.5 rounded-full bg-amber-400" />
                        <div className="size-1.5 rounded-full bg-green-400" />
                      </div>
                      <div className="h-2 w-12 rounded bg-neutral-300" />
                      <div className="h-1.5 w-full rounded bg-neutral-200" />
                      <div className="h-1.5 w-3/4 rounded bg-neutral-200" />
                    </div>
                  ) : (
                    <div className="flex h-full w-full flex-col gap-1.5 rounded-md bg-neutral-950 p-2 border border-neutral-800">
                      <div className="flex items-center gap-1">
                        <div className="size-1.5 rounded-full bg-neutral-700" />
                        <div className="size-1.5 rounded-full bg-neutral-700" />
                        <div className="size-1.5 rounded-full bg-neutral-700" />
                      </div>
                      <div className="h-2 w-12 rounded bg-neutral-700" />
                      <div className="h-1.5 w-full rounded bg-neutral-800" />
                      <div className="h-1.5 w-3/4 rounded bg-neutral-800" />
                    </div>
                  )}

                  {isSelected ? (
                    <div className="absolute right-2 top-2 flex size-5 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-2xs">
                      <Check className="size-3 stroke-[2.5]" />
                    </div>
                  ) : null}
                </div>

                {/* 标签与描述 */}
                <div className="flex items-center gap-2">
                  <Icon
                    className={cn(
                      "size-4 shrink-0 transition-colors",
                      isSelected ? "text-primary" : "text-muted-foreground",
                    )}
                  />
                  <span className="text-xs font-semibold tracking-tight text-foreground">
                    {t(option.labelKey)}
                  </span>
                </div>
              </button>
            );
          })}
        </div>
      </SettingCard>
    </div>
  );
}
