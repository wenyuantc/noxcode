import {
  bundledLanguages,
  createHighlighter,
  type BundledLanguage,
  type Highlighter,
  type SpecialLanguage,
} from "shiki";

import { CODE_THEME_IDS, type CodeThemeId } from "./codeThemes";
import { normalizeLanguage } from "./codeLanguage";

const PRELOAD_LANGS = [
  "typescript",
  "tsx",
  "javascript",
  "jsx",
  "python",
  "rust",
  "go",
  "json",
  "jsonc",
  "bash",
  "markdown",
  "css",
  "html",
  "yaml",
  "toml",
  "sql",
  "java",
  "c",
  "cpp",
  "diff",
  "xml",
  "dockerfile",
  "ini",
] as const;

const MAX_HIGHLIGHT_CHARS = 200_000;
const CACHE_LIMIT = 80;

export interface HighlightedLine {
  html: string;
}

export interface HighlightResult {
  fg: string;
  bg: string;
  lines: HighlightedLine[];
}

let highlighterPromise: Promise<Highlighter> | null = null;
const cache = new Map<string, HighlightResult>();

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fontStyleToCss(style: number): string {
  if (style <= 0) return "";
  const parts: string[] = [];
  if (style & 1) parts.push("font-style:italic");
  if (style & 2) parts.push("font-weight:700");
  if (style & 4) parts.push("text-decoration:underline");
  return parts.join(";");
}

function plainResult(code: string): HighlightResult {
  return {
    fg: "",
    bg: "",
    lines: code.split("\n").map((line) => ({ html: escapeHtml(line) })),
  };
}

function remember(key: string, result: HighlightResult): HighlightResult {
  cache.set(key, result);
  if (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest) cache.delete(oldest);
  }
  return result;
}

async function getHighlighter(): Promise<Highlighter> {
  highlighterPromise ??= createHighlighter({
    themes: [...CODE_THEME_IDS],
    langs: [...PRELOAD_LANGS],
  });
  return highlighterPromise;
}

async function resolveLanguage(highlighter: Highlighter, lang: string): Promise<string> {
  const normalized = normalizeLanguage(lang);
  if (normalized === "plaintext") return "text";
  const loaded = highlighter.getLoadedLanguages();
  if (loaded.includes(normalized)) return normalized;
  if (normalized in bundledLanguages) {
    try {
      await highlighter.loadLanguage(normalized as keyof typeof bundledLanguages);
      return normalized;
    } catch {
      return "text";
    }
  }
  return "text";
}

export async function highlightCode(
  code: string,
  lang: string,
  theme: CodeThemeId,
): Promise<HighlightResult> {
  if (code.length > MAX_HIGHLIGHT_CHARS) return plainResult(code);
  const key = `${theme}\0${lang}\0${code}`;
  const hit = cache.get(key);
  if (hit) return hit;

  try {
    const highlighter = await getHighlighter();
    const resolvedLang = await resolveLanguage(highlighter, lang);
    const tokens = highlighter.codeToTokens(code, {
      lang: resolvedLang as BundledLanguage | SpecialLanguage,
      theme,
    });
    return remember(key, {
      fg: tokens.fg ?? "",
      bg: tokens.bg ?? "",
      lines: tokens.tokens.map((line) => ({
        html: line
          .map((token) => {
            const color = token.color ? `color:${token.color}` : "";
            const font = token.fontStyle ? fontStyleToCss(token.fontStyle) : "";
            const style = [color, font].filter(Boolean).join(";");
            const content = escapeHtml(token.content);
            return style ? `<span style="${style}">${content}</span>` : content;
          })
          .join(""),
      })),
    });
  } catch {
    return remember(key, plainResult(code));
  }
}

export function splitPlainLines(code: string): string[] {
  return code.split("\n");
}
