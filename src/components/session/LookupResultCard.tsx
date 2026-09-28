import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import { languageFromPath } from "@/lib/codeLanguage";
import { getNativeToolImage } from "@/lib/backend";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import type { NativeToolImage } from "@/lib/types";
import {
  lookupPathText,
  parseReadResultLines,
  parseSqliteResult,
  parseToolHeader,
  sessionLineBody,
} from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { SqliteResultTable } from "./SqliteResultTable";

function ToolImage({ image, alt }: { image: NativeToolImage; alt: string }) {
  const [source, setSource] = useState(image.data_url);

  useEffect(() => {
    let current = true;
    if (image.data_url) {
      setSource(image.data_url);
    } else if (image.attachment_id) {
      setSource("");
      void getNativeToolImage(image.attachment_id)
        .then((dataUrl) => {
          if (current) setSource(dataUrl);
        })
        .catch(() => {
          if (current) setSource("");
        });
    }
    return () => {
      current = false;
    };
  }, [image.attachment_id, image.data_url]);

  return source ? (
    <img src={source} alt={alt} className="max-h-48 max-w-full rounded-md border" />
  ) : (
    <span className="text-xs text-muted-foreground">{image.name}</span>
  );
}

export function LookupResultCard({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const parsed = parseToolHeader(item);
  const body = sessionLineBody(item.text);
  const isRead = parsed.category === "read" || body.startsWith("[读取]");
  const isSqlite =
    parsed.category === "sqlite" ||
    body.startsWith("[工具] SQLiteQuery") ||
    item.toolName?.includes("SQLiteQuery") ||
    item.tool?.name === "SQLiteQuery";
  const lines = item.result && isRead ? parseReadResultLines(item.result) : null;
  const sqliteData = item.result && isSqlite ? parseSqliteResult(item.result) : null;
  const rawPath = parsed.category === "read" ? parsed.detail : lookupPathText(item);
  const language = languageFromPath(rawPath);

  return (
    <div className="rounded-lg border border-border/50 bg-muted/20 p-2.5 space-y-2">
      <div className="flex items-center gap-2 min-w-0">
        <span
          className={cn(
            "inline-flex shrink-0 items-center rounded border px-1.5 py-0.5 text-[10.5px] font-medium leading-none",
            parsed.badgeClass,
          )}
        >
          {parsed.badge}
        </span>
        <span
          className="min-w-0 flex-1 truncate font-mono text-[11.5px] font-medium text-foreground/90 select-text"
          title={parsed.detail}
        >
          {parsed.detail}
        </span>
        {parsed.failed ? (
          <span className="shrink-0 inline-flex items-center rounded border border-red-500/30 bg-red-500/10 px-1.5 py-0.5 text-[10px] font-medium text-red-600 dark:text-red-400">
            {t("toolFailed")}
          </span>
        ) : null}
      </div>
      {item.images?.length ? (
        <div className="mt-1 flex flex-wrap gap-2">
          {item.images.map((image) => (
            <ToolImage
              key={`${item.id}-${image.name}`}
              image={image}
              alt={t("toolImageAlt", { name: image.name })}
            />
          ))}
        </div>
      ) : null}
      {lines ? (
        <CodeBlock
          className="max-h-80"
          code={lines.map((row) => row.text).join("\n")}
          language={language}
          lineNumbers={lines.map((row) => row.line)}
        />
      ) : sqliteData ? (
        <SqliteResultTable data={sqliteData} rawResult={item.result!} />
      ) : item.result ? (
        <CodeBlock className="max-h-80" code={item.result} language={language} />
      ) : (
        <p className="text-xs text-muted-foreground">{t("toolResult")}</p>
      )}
    </div>
  );
}
