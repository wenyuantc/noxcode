import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import { languageFromPath } from "@/lib/codeLanguage";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import {
  lookupPathText,
  parseReadResultLines,
  sessionLineBody,
  toolTitle,
} from "@/lib/sessionLines";
import { cn } from "@/lib/utils";

export function LookupResultCard({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const body = sessionLineBody(item.text);
  const isRead = body.startsWith("[读取]");
  const failed = item.ok === false;
  const title = item.toolName ?? (isRead ? `[读取] ${lookupPathText(item)}` : toolTitle(item.text));
  const lines = item.result && isRead ? parseReadResultLines(item.result) : null;
  const language = languageFromPath(lookupPathText(item));

  return (
    <div className="mt-1">
      <p
        className={cn(
          "truncate font-mono text-xs",
          failed ? "text-red-600 dark:text-red-400" : "text-cyan-600 dark:text-cyan-400",
        )}
      >
        {failed ? `${t("toolFailed")} · ${title}` : title}
      </p>
      {item.images?.length ? (
        <div className="mt-1 flex flex-wrap gap-2">
          {item.images.map((image) => (
            <img
              key={`${item.id}-${image.name}`}
              src={image.data_url}
              alt={t("toolImageAlt", { name: image.name })}
              className="max-h-48 max-w-full rounded-md border"
            />
          ))}
        </div>
      ) : null}
      {lines ? (
        <CodeBlock
          className="mt-1 max-h-80"
          code={lines.map((row) => row.text).join("\n")}
          language={language}
          lineNumbers={lines.map((row) => row.line)}
        />
      ) : item.result ? (
        <CodeBlock className="mt-1 max-h-80" code={item.result} language={language} />
      ) : (
        <p className="mt-1 text-xs text-muted-foreground">{t("toolResult")}</p>
      )}
    </div>
  );
}
