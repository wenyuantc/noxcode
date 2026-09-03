import { Brain, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { formatSessionDuration, thinkingDurationSeconds, thinkingText } from "@/lib/sessionLines";

export function ThinkingRow({ items, nowMs }: { items: GroupedSessionItem[]; nowMs?: number }) {
  const { t } = useTranslation("sessions");
  const [open, setOpen] = useState(true);
  const seconds = thinkingDurationSeconds(items, nowMs);
  const label =
    seconds < 1
      ? t("thinkingForBrief")
      : t("thinkingFor", { duration: formatSessionDuration(t, seconds) });
  const body = thinkingText(items);

  return (
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-2 text-sm text-muted-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <Brain className="size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-left">{label}</span>
        <ChevronRight className={`size-3.5 shrink-0 transition ${open ? "rotate-90" : ""}`} />
      </button>
      {open ? (
        <pre className="mt-1 max-h-80 overflow-auto whitespace-pre-wrap rounded-md border px-3 py-2 text-xs leading-5 text-muted-foreground">
          {body}
        </pre>
      ) : null}
    </div>
  );
}
