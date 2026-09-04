import { ChevronRight, FilePenLine } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import { languageFromPath } from "@/lib/codeLanguage";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import { fileActionKey, filePathText } from "@/lib/sessionLines";

import { cn } from "@/lib/utils";

export function FileChangeRow({
  items,
  grouped,
}: {
  items: GroupedSessionItem[];
  grouped?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const first = items[0];
  const label = grouped
    ? t("changesGroup", { count: items.length })
    : first
      ? `${t(fileActionKey(first.text))} ${filePathText(first)}`
      : "";

  return (
    <div className="rounded-xl border border-border/60 bg-muted/15 transition-all duration-150 hover:border-border/80">
      <button
        type="button"
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-xs"
        onClick={() => setOpen((value) => !value)}
      >
        <FilePenLine className="size-3.5 shrink-0 text-emerald-500/90 dark:text-emerald-400" />
        <span className="inline-flex items-center rounded border border-border/40 bg-background/50 px-1.5 py-0.5 font-medium text-[10px] text-muted-foreground">
          {grouped ? "修改" : "文件"}
        </span>
        <span className="min-w-0 flex-1 truncate text-left font-medium text-foreground/80">
          {label}
        </span>
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground/70 transition-transform duration-150",
            open && "rotate-90",
          )}
        />
      </button>
      {open ? (
        <div className="border-t border-border/40 p-3">
          <ul className="space-y-2 text-xs">
            {items.map((item) => (
              <li key={item.id} className="rounded-lg border border-border/40 bg-muted/20 p-2">
                <p className="truncate font-mono text-[11px] text-foreground/90 font-medium">
                  {filePathText(item)}
                </p>
                {item.result ? (
                  <CodeBlock
                    className="mt-1.5 max-h-48"
                    code={item.result}
                    language={languageFromPath(filePathText(item))}
                  />
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
