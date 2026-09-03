import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { getCurrentAppLocale } from "@/lib/i18n/locale";
import type { NativeContextUsage } from "@/lib/types";
import { cn, formatCompactTokens } from "@/lib/utils";

const CATEGORIES = [
  { key: "mcp_tokens", labelKey: "contextCategoryMcp", swatch: "bg-sky-500" },
  { key: "system_tool_tokens", labelKey: "contextCategorySystemTools", swatch: "bg-sky-400" },
  { key: "skill_tokens", labelKey: "contextCategorySkills", swatch: "bg-sky-300" },
  { key: "system_prompt_tokens", labelKey: "contextCategorySystemPrompt", swatch: "bg-cyan-400" },
  { key: "other_tokens", labelKey: "contextCategoryOther", swatch: "bg-slate-400" },
  { key: "message_tokens", labelKey: "contextCategoryMessages", swatch: "bg-slate-500" },
] as const;

function tokenOf(usage: NativeContextUsage, key: (typeof CATEGORIES)[number]["key"]): number {
  return usage[key] ?? 0;
}

export function ContextCapacity({ usage }: { usage?: NativeContextUsage }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const locale = getCurrentAppLocale();

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onPointer);
    return () => window.removeEventListener("pointerdown", onPointer);
  }, [open]);

  if (!usage || usage.limit_tokens <= 0) return null;

  const used = usage.used_tokens;
  const limit = usage.limit_tokens;
  const occupancy = Math.min(100, (used / limit) * 100);
  const prompt = usage.prompt_tokens ?? 0;
  const cached = usage.cached_tokens ?? 0;
  const cacheRate = prompt > 0 ? (cached / prompt) * 100 : null;
  const usedLabel = formatCompactTokens(used, locale);
  const limitLabel = formatCompactTokens(limit, locale);
  const rows = CATEGORIES.map((category) => ({
    ...category,
    tokens: tokenOf(usage, category.key),
  })).filter((row) => row.tokens > 0);
  const percentBase = used > 0 ? used : rows.reduce((sum, row) => sum + row.tokens, 0);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-xs text-muted-foreground outline-none hover:text-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <span>
          {usedLabel} / {limitLabel}
        </span>
        {cacheRate != null ? (
          <span>· {t("contextCacheShort", { rate: Math.round(cacheRate) })}</span>
        ) : null}
      </button>
      {open ? (
        <div className="absolute bottom-full left-0 z-20 mb-2 w-72 rounded-lg border bg-popover p-3 text-xs shadow-md">
          <div className="flex items-center gap-2 text-muted-foreground">
            <span className="flex-1">{t("contextCapacity")}</span>
            <span className="tabular-nums text-foreground">
              {usedLabel}/{limitLabel} ({occupancy.toFixed(1)}%)
            </span>
          </div>
          {cacheRate != null ? (
            <p className="mt-1 text-muted-foreground">
              {t("contextCacheRate", { rate: cacheRate.toFixed(1) })}
            </p>
          ) : null}
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
            <div className="h-full rounded-full bg-sky-500" style={{ width: `${occupancy}%` }} />
          </div>
          {rows.length > 0 ? (
            <ul className="mt-3 space-y-1.5">
              {rows.map((row) => {
                const share = percentBase > 0 ? (row.tokens / percentBase) * 100 : 0;
                return (
                  <li key={row.key} className="flex items-center gap-2">
                    <span className={cn("size-1.5 shrink-0 rounded-full", row.swatch)} />
                    <span className="min-w-0 flex-1 truncate text-muted-foreground">
                      {t(row.labelKey)}
                    </span>
                    <span className="tabular-nums text-foreground">{share.toFixed(1)}%</span>
                  </li>
                );
              })}
            </ul>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
