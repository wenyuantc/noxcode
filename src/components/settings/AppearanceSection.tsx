import { Check, Laptop, Moon, Sun } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodePreview } from "@/components/code/CodePreview";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  CODE_FONT_SIZE_MAX,
  CODE_FONT_SIZE_MIN,
  UI_FONT_SIZE_MAX,
  UI_FONT_SIZE_MIN,
} from "@/lib/codeAppearance";
import { CODE_THEMES, type CodeThemeId } from "@/lib/codeThemes";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";
import { SettingCard, SettingRow } from "./SettingCard";

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

function PxInput({
  id,
  value,
  min,
  max,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    if (!focused) setDraft(String(value));
  }, [focused, value]);

  const commit = (raw: string) => {
    const next = Number(raw);
    if (!Number.isFinite(next)) {
      setDraft(String(value));
      return;
    }
    onChange(next);
  };

  return (
    <div className="relative w-20">
      <Input
        id={id}
        className="h-8 pr-7 text-right font-mono text-xs"
        type="number"
        min={min}
        max={max}
        step={1}
        value={focused ? draft : value}
        onFocus={() => {
          setFocused(true);
          setDraft(String(value));
        }}
        onBlur={() => {
          setFocused(false);
          commit(draft);
        }}
        onChange={(event) => {
          const raw = event.target.value;
          setDraft(raw);
          const next = Number(raw);
          if (Number.isFinite(next) && next >= min && next <= max) {
            onChange(next);
          }
        }}
      />
      <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-muted-foreground">
        px
      </span>
    </div>
  );
}

function CodeThemeSelect({
  id,
  value,
  onChange,
}: {
  id: string;
  value: CodeThemeId;
  onChange: (value: CodeThemeId) => void;
}) {
  return (
    <Select value={value} onValueChange={(next) => onChange(next as CodeThemeId)}>
      <SelectTrigger id={id} className="h-8 w-44 bg-background text-xs">
        <SelectValue />
      </SelectTrigger>
      <SelectContent align="end">
        {CODE_THEMES.map((theme) => (
          <SelectItem key={theme.id} value={theme.id} className="text-xs">
            {theme.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

export function AppearanceSection() {
  const { t } = useTranslation("settings");
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const uiFontSize = useUiStore((state) => state.uiFontSize);
  const setUiFontSize = useUiStore((state) => state.setUiFontSize);
  const codeThemeLight = useUiStore((state) => state.codeThemeLight);
  const setCodeThemeLight = useUiStore((state) => state.setCodeThemeLight);
  const codeThemeDark = useUiStore((state) => state.codeThemeDark);
  const setCodeThemeDark = useUiStore((state) => state.setCodeThemeDark);
  const codeLineNumbers = useUiStore((state) => state.codeLineNumbers);
  const setCodeLineNumbers = useUiStore((state) => state.setCodeLineNumbers);
  const codeSoftWrap = useUiStore((state) => state.codeSoftWrap);
  const setCodeSoftWrap = useUiStore((state) => state.setCodeSoftWrap);
  const codeFontSize = useUiStore((state) => state.codeFontSize);
  const setCodeFontSize = useUiStore((state) => state.setCodeFontSize);

  return (
    <div className="space-y-6">
      <SettingCard
        title={t("appearance.theme")}
        description={t("appearance.themeHint")}
        badge={t(`appearance.${theme}`)}
        contentClassName="p-0"
      >
        <div className="p-5">
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
                  <div className="relative flex h-20 w-full overflow-hidden rounded-lg border border-border/60 p-2 shadow-2xs">
                    {option.id === "system" ? (
                      <div className="flex h-full w-full overflow-hidden rounded-md">
                        <div className="flex h-full w-1/2 flex-col gap-1 border-r border-neutral-300 bg-neutral-100 p-1.5">
                          <div className="h-1.5 w-6 rounded-full bg-neutral-300" />
                          <div className="h-1.5 w-full rounded-xs bg-neutral-200" />
                          <div className="h-1.5 w-4/5 rounded-xs bg-neutral-200" />
                        </div>
                        <div className="flex h-full w-1/2 flex-col gap-1 bg-neutral-900 p-1.5">
                          <div className="h-1.5 w-6 rounded-full bg-neutral-700" />
                          <div className="h-1.5 w-full rounded-xs bg-neutral-800" />
                          <div className="h-1.5 w-4/5 rounded-xs bg-neutral-800" />
                        </div>
                      </div>
                    ) : option.id === "light" ? (
                      <div className="flex h-full w-full flex-col gap-1.5 rounded-md border border-neutral-200 bg-neutral-50 p-2">
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
                      <div className="flex h-full w-full flex-col gap-1.5 rounded-md border border-neutral-800 bg-neutral-950 p-2">
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
        </div>
        <div className="divide-y divide-border/50 border-t border-border/50">
          <SettingRow
            title={t("appearance.uiFontSize")}
            description={t("appearance.uiFontSizeHint")}
          >
            <PxInput
              id="ui-font-size"
              value={uiFontSize}
              min={UI_FONT_SIZE_MIN}
              max={UI_FONT_SIZE_MAX}
              onChange={setUiFontSize}
            />
          </SettingRow>
        </div>
      </SettingCard>

      <SettingCard
        title={t("appearance.codeSettings")}
        description={t("appearance.codeSettingsHint")}
        divided
      >
        <SettingRow
          title={t("appearance.codeThemeLight")}
          description={t("appearance.codeThemeLightHint")}
        >
          <CodeThemeSelect
            id="code-theme-light"
            value={codeThemeLight}
            onChange={setCodeThemeLight}
          />
        </SettingRow>
        <SettingRow
          title={t("appearance.codeThemeDark")}
          description={t("appearance.codeThemeDarkHint")}
        >
          <CodeThemeSelect id="code-theme-dark" value={codeThemeDark} onChange={setCodeThemeDark} />
        </SettingRow>
        <SettingRow
          title={t("appearance.lineNumbers")}
          description={t("appearance.lineNumbersHint")}
        >
          <Switch
            id="code-line-numbers"
            checked={codeLineNumbers}
            onCheckedChange={setCodeLineNumbers}
          />
        </SettingRow>
        <SettingRow title={t("appearance.softWrap")} description={t("appearance.softWrapHint")}>
          <Switch id="code-soft-wrap" checked={codeSoftWrap} onCheckedChange={setCodeSoftWrap} />
        </SettingRow>
        <SettingRow
          title={t("appearance.codeFontSize")}
          description={t("appearance.codeFontSizeHint")}
        >
          <PxInput
            id="code-font-size"
            value={codeFontSize}
            min={CODE_FONT_SIZE_MIN}
            max={CODE_FONT_SIZE_MAX}
            onChange={setCodeFontSize}
          />
        </SettingRow>
      </SettingCard>

      <SettingCard title={t("appearance.preview")} description={t("appearance.previewHint")}>
        <CodePreview />
      </SettingCard>
    </div>
  );
}
