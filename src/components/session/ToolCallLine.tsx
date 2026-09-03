import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { lineToneClass } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";

export function ToolCallLine({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(false);
  return (
    <div className={cn("rounded-md px-2 py-1 text-sm", lineToneClass(item.kind, item.text))}>
      <button type="button" className="w-full text-left" onClick={() => setOpen((value) => !value)}>
        {item.toolName ?? item.text.split("\n")[0]}
      </button>
      {open && item.result ? (
        <pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap text-xs text-muted-foreground">
          {item.result}
        </pre>
      ) : null}
      {open && !item.result ? (
        <p className="text-xs text-muted-foreground">{t("toolResult")}</p>
      ) : null}
    </div>
  );
}
