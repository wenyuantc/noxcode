import { useDeferredValue, useEffect, useMemo, useState } from "react";

import { highlightCode, type HighlightResult } from "@/lib/codeHighlighter";
import { normalizeLanguage } from "@/lib/codeLanguage";
import type { CodeThemeId } from "@/lib/codeThemes";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";

export interface CodeBlockProps {
  code: string;
  language?: string;
  theme?: CodeThemeId;
  lineNumbers?: Array<number | null>;
  showLineNumbers?: boolean;
  className?: string;
  preClassName?: string;
}

function gutterWidth(values: Array<number | null>): number {
  let max = 0;
  for (const value of values) {
    if (value != null) max = Math.max(max, value);
  }
  return Math.max(2, String(max).length);
}

export function CodeBlock({
  code,
  language,
  theme,
  lineNumbers,
  showLineNumbers,
  className,
  preClassName,
}: CodeBlockProps) {
  const deferredCode = useDeferredValue(code);
  const isDark = useUiStore((state) => state.isDark);
  const storeLight = useUiStore((state) => state.codeThemeLight);
  const storeDark = useUiStore((state) => state.codeThemeDark);
  const storeShowLines = useUiStore((state) => state.codeLineNumbers);
  const softWrap = useUiStore((state) => state.codeSoftWrap);
  const fontSize = useUiStore((state) => state.codeFontSize);
  const activeTheme = theme ?? (isDark ? storeDark : storeLight);
  const lang = normalizeLanguage(language);
  const linesOn = showLineNumbers ?? storeShowLines;
  const highlightKey = `${activeTheme}:${lang}:${deferredCode}`;

  const [result, setResult] = useState<HighlightResult | null>(null);
  const [readyKey, setReadyKey] = useState("");

  useEffect(() => {
    let cancelled = false;
    void highlightCode(deferredCode, lang, activeTheme).then((next) => {
      if (cancelled) return;
      setResult(next);
      setReadyKey(highlightKey);
    });
    return () => {
      cancelled = true;
    };
  }, [activeTheme, deferredCode, highlightKey, lang]);

  const highlighted = readyKey === highlightKey ? result : null;
  const fallbackLines = useMemo(() => code.split("\n"), [code]);
  const displayCount = highlighted?.lines.length || fallbackLines.length;
  const numbers = useMemo(() => {
    if (lineNumbers && lineNumbers.length > 0) {
      return Array.from({ length: displayCount }, (_, index) => lineNumbers[index] ?? null);
    }
    return Array.from({ length: displayCount }, (_, index) => index + 1);
  }, [displayCount, lineNumbers]);
  const numberWidth = gutterWidth(numbers);

  return (
    <div
      className={cn("code-surface overflow-auto rounded-md border border-border/70", className)}
      data-wrap={softWrap ? "1" : "0"}
      style={{
        fontSize,
        backgroundColor: highlighted?.bg || undefined,
        color: highlighted?.fg || undefined,
      }}
    >
      <pre className={cn("m-0 min-w-0 py-2 font-mono leading-[1.55]", preClassName)}>
        {Array.from({ length: displayCount }, (_, index) => {
          const html = highlighted?.lines[index]?.html;
          const text = fallbackLines[index] ?? "";
          return (
            <div key={index} className="code-surface-line flex px-3">
              {linesOn ? (
                <span
                  className="code-surface-gutter mr-3 shrink-0"
                  style={{ width: `${numberWidth}ch` }}
                >
                  {numbers[index] ?? ""}
                </span>
              ) : null}
              {html != null ? (
                <span
                  className="code-surface-text min-w-0 flex-1"
                  dangerouslySetInnerHTML={{ __html: html || " " }}
                />
              ) : (
                <span className="code-surface-text min-w-0 flex-1">{text || " "}</span>
              )}
            </div>
          );
        })}
      </pre>
    </div>
  );
}
