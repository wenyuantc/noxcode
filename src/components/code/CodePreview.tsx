import { useTranslation } from "react-i18next";

import type { CodeThemeId } from "@/lib/codeThemes";
import { codeThemeLabel } from "@/lib/codeThemes";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";

import { CodeBlock } from "./CodeBlock";

const PREVIEW_CODE = `type User = {
  id: number;
  name: string;
};

function greet(user: User): string {
  // 同时预览浅色与深色代码主题
  const message = \`Hello, \${user.name}!\`;
  console.log(message);
  return message;
}
`;

function PreviewPane({
  theme,
  active,
  activeLabel,
}: {
  theme: CodeThemeId;
  active: boolean;
  activeLabel: string;
}) {
  return (
    <div className="min-w-0 space-y-2">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-foreground">{codeThemeLabel(theme)}</span>
        {active ? (
          <span className="inline-flex items-center rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
            {activeLabel}
          </span>
        ) : null}
      </div>
      <CodeBlock
        code={PREVIEW_CODE}
        language="typescript"
        theme={theme}
        className={cn(active && "ring-2 ring-primary/25")}
      />
    </div>
  );
}

export function CodePreview() {
  const { t } = useTranslation("settings");
  const isDark = useUiStore((state) => state.isDark);
  const lightTheme = useUiStore((state) => state.codeThemeLight);
  const darkTheme = useUiStore((state) => state.codeThemeDark);

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <PreviewPane
        theme={lightTheme}
        active={!isDark}
        activeLabel={t("appearance.previewActive")}
      />
      <PreviewPane theme={darkTheme} active={isDark} activeLabel={t("appearance.previewActive")} />
    </div>
  );
}
