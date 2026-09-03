import { ChevronRight, FilePenLine } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { fileActionKey, filePathText } from "@/lib/sessionLines";

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
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-2 text-sm text-muted-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <FilePenLine className="size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-left">{label}</span>
        <ChevronRight className={`size-3.5 shrink-0 transition ${open ? "rotate-90" : ""}`} />
      </button>
      {open ? (
        <ul className="mt-1 space-y-1 text-xs text-muted-foreground">
          {items.map((item) => (
            <li key={item.id} className="truncate font-mono">
              {filePathText(item)}
              {item.result ? (
                <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md border px-2 py-1">
                  {item.result}
                </pre>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
