import { SquareTerminal } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { commandText } from "@/lib/sessionLines";
import { SegmentCard } from "./SegmentCard";

export function TerminalRow({ item, running }: { item: GroupedSessionItem; running?: boolean }) {
  const { t } = useTranslation("sessions");
  const command = commandText(item);

  return (
    <SegmentCard
      icon={<SquareTerminal className="size-3" />}
      iconClassName="bg-sky-500/15 text-sky-500 dark:text-sky-400"
      badge={t("terminal")}
      title={
        command ? <span className="font-mono text-code-sm text-foreground/90">{command}</span> : ""
      }
      running={running}
      contentClassName="bg-black/25"
    >
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap font-mono text-code-sm leading-relaxed text-foreground/90 select-text">
        <span className="text-emerald-500">$ </span>
        {command}
        {item.result ? `\n${item.result}` : ""}
      </pre>
    </SegmentCard>
  );
}
