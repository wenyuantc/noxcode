import { FilePenLine } from "lucide-react";
import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import { languageFromPath } from "@/lib/codeLanguage";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import { fileActionKey, filePathText } from "@/lib/sessionLines";
import { SegmentCard } from "./SegmentCard";

export function FileChangeRow({
  items,
  grouped,
}: {
  items: GroupedSessionItem[];
  grouped?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const first = items[0];
  const label = grouped
    ? t("changesGroup", { count: items.length })
    : first
      ? `${t(fileActionKey(first.text))} ${filePathText(first)}`
      : "";

  return (
    <SegmentCard
      icon={<FilePenLine className="size-3" />}
      iconClassName="bg-emerald-500/15 text-emerald-500/90 dark:text-emerald-400"
      badge={grouped ? t("changesBadge") : t("fileBadge")}
      title={label}
    >
      <ul className="space-y-2 text-xs">
        {items.map((item) => (
          <li key={item.id} className="rounded-lg border border-border/40 bg-muted/20 p-2">
            <p className="truncate font-mono text-code-sm font-medium text-foreground/90">
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
    </SegmentCard>
  );
}
