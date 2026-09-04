import { AlertCircle, FileCode2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { highlightCode, type HighlightResult } from "@/lib/codeHighlighter";
import { resolveActiveCodeTheme } from "@/lib/codeAppearance";
import { diffGutterWidth, parseDiffLineNumbers, type DiffLineInfo } from "@/lib/diffLineNumbers";
import type { GitFileDiff } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";

function lineAccent(type: DiffLineInfo["type"]): string {
  if (type === "add") return "border-l-2 border-emerald-500 bg-emerald-500/8";
  if (type === "del") return "border-l-2 border-rose-500 bg-rose-500/8";
  if (type === "hunk") return "bg-muted/60 font-semibold text-sky-700 dark:text-sky-300";
  return "";
}

export function DiffView({ diff }: { diff: GitFileDiff | null }) {
  const { t } = useTranslation("git");
  const isDark = useUiStore((state) => state.isDark);
  const codeThemeLight = useUiStore((state) => state.codeThemeLight);
  const codeThemeDark = useUiStore((state) => state.codeThemeDark);
  const showLineNumbers = useUiStore((state) => state.codeLineNumbers);
  const softWrap = useUiStore((state) => state.codeSoftWrap);
  const fontSize = useUiStore((state) => state.codeFontSize);
  const theme = resolveActiveCodeTheme({ codeThemeLight, codeThemeDark }, isDark);
  const patch = diff?.patch || "";
  const lines = useMemo(() => parseDiffLineNumbers(patch), [patch]);
  const gutterWidth = diffGutterWidth(lines);

  const [highlighted, setHighlighted] = useState<HighlightResult | null>(null);

  useEffect(() => {
    if (!patch || diff?.is_binary) {
      setHighlighted(null);
      return;
    }
    let cancelled = false;
    void highlightCode(patch, "diff", theme).then((next) => {
      if (!cancelled) setHighlighted(next);
    });
    return () => {
      cancelled = true;
    };
  }, [diff?.is_binary, patch, theme]);

  if (!diff) {
    return null;
  }
  if (diff.is_binary) {
    return (
      <div className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
        <FileCode2 className="size-4 opacity-50" />
        {t("binary")}
      </div>
    );
  }

  return (
    <div
      className="code-surface overflow-auto"
      data-wrap={softWrap ? "1" : "0"}
      style={{
        fontSize,
        backgroundColor: highlighted?.bg || undefined,
        color: highlighted?.fg || undefined,
      }}
    >
      {diff.truncated ? (
        <div className="flex items-center gap-1.5 border-b border-amber-500/20 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-300">
          <AlertCircle className="size-3.5 shrink-0" />
          <span>{t("truncated")}</span>
        </div>
      ) : null}
      <pre className="m-0 py-1.5 font-mono leading-[1.55]">
        {lines.map((line, index) => {
          const html = highlighted?.lines[index]?.html;
          return (
            <div
              key={`${index}-${line.text.slice(0, 24)}`}
              className={cn("code-surface-line flex px-3 py-0.5", lineAccent(line.type))}
            >
              {showLineNumbers ? (
                <>
                  <span
                    className="code-surface-gutter mr-2 shrink-0"
                    style={{ width: `${gutterWidth}ch` }}
                  >
                    {line.oldLine ?? ""}
                  </span>
                  <span
                    className="code-surface-gutter mr-3 shrink-0"
                    style={{ width: `${gutterWidth}ch` }}
                  >
                    {line.newLine ?? ""}
                  </span>
                </>
              ) : null}
              {html != null ? (
                <span
                  className="code-surface-text min-w-0 flex-1"
                  dangerouslySetInnerHTML={{ __html: html || " " }}
                />
              ) : (
                <span className="code-surface-text min-w-0 flex-1">{line.text || " "}</span>
              )}
            </div>
          );
        })}
      </pre>
    </div>
  );
}
