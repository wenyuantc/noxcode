import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { lookupPathText, parseReadResultLines } from "@/lib/sessionLines";

export function LookupResultCard({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const isRead = item.text.startsWith("[读取]");
  const title = isRead ? `[读取] ${lookupPathText(item)}` : (item.text.split("\n")[0] ?? item.text);
  const lines = item.result && isRead ? parseReadResultLines(item.result) : null;

  return (
    <div className="mt-1">
      <p className="truncate font-mono text-xs text-cyan-600 dark:text-cyan-400">{title}</p>
      {lines ? (
        <pre className="mt-1 max-h-80 overflow-auto rounded-lg border bg-muted/30 px-3 py-2 font-mono text-xs leading-5">
          {lines.map((row, index) => (
            <div key={`${item.id}-${index}`} className="flex gap-3">
              <span className="w-10 shrink-0 select-none text-right text-muted-foreground/70">
                {row.line ?? ""}
              </span>
              <span className="min-w-0 flex-1 whitespace-pre-wrap break-all">{row.text}</span>
            </div>
          ))}
        </pre>
      ) : item.result ? (
        <pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap rounded-lg border bg-muted/30 px-3 py-2 font-mono text-xs text-muted-foreground">
          {item.result}
        </pre>
      ) : (
        <p className="mt-1 text-xs text-muted-foreground">{t("toolResult")}</p>
      )}
    </div>
  );
}
