import { ChevronRight, SquareTerminal } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { commandText } from "@/lib/sessionLines";

export function TerminalRow({ item, running }: { item: GroupedSessionItem; running?: boolean }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  const command = commandText(item);

  return (
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-2 text-sm"
        onClick={() => setOpen((value) => !value)}
      >
        <SquareTerminal className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="shrink-0 text-muted-foreground">{t("terminal")}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{command}</span>
        {running ? (
          <span className="shrink-0 text-xs text-muted-foreground">{t("running")}</span>
        ) : null}
        <ChevronRight
          className={`size-3.5 shrink-0 text-muted-foreground transition ${open ? "rotate-90" : ""}`}
        />
      </button>
      {open ? (
        <pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap rounded-lg border bg-muted/30 px-3 py-2 font-mono text-xs">
          <span className="text-muted-foreground">$ </span>
          {command}
          {item.result ? `\n${item.result}` : ""}
        </pre>
      ) : null}
    </div>
  );
}
