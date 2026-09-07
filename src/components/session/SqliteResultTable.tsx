import { AlertCircle, Check, Code, Copy, Table } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import type { ParsedSqliteResult } from "@/lib/sessionLines";
import { cn } from "@/lib/utils";

function formatTsv(data: ParsedSqliteResult): string {
  const header = data.columns.join("\t");
  const rows = data.rows.map((row) =>
    row
      .map((cell) => {
        if (cell === null || cell === undefined) return "NULL";
        if (typeof cell === "object") return JSON.stringify(cell);
        return String(cell).replace(/[\t\n\r]/g, " ");
      })
      .join("\t"),
  );
  return [header, ...rows].join("\n");
}

function renderCellValue(cell: unknown) {
  if (cell === null || cell === undefined) {
    return <span className="italic text-muted-foreground/45">NULL</span>;
  }
  if (typeof cell === "boolean") {
    return (
      <span className="font-semibold text-amber-600 dark:text-amber-400">
        {cell ? "true" : "false"}
      </span>
    );
  }
  if (typeof cell === "number") {
    return <span className="text-emerald-600 dark:text-emerald-400">{cell}</span>;
  }
  if (typeof cell === "object") {
    const text = JSON.stringify(cell);
    return (
      <span className="text-violet-600 dark:text-violet-400" title={text}>
        {text}
      </span>
    );
  }
  const text = String(cell);
  return (
    <span className="truncate text-foreground/85" title={text}>
      {text}
    </span>
  );
}

export function SqliteResultTable({
  data,
  rawResult,
}: {
  data: ParsedSqliteResult;
  rawResult: string;
}) {
  const { t } = useTranslation("sessions");
  const [view, setView] = useState<"table" | "json">("table");
  const [copied, setCopied] = useState(false);

  const prettyJson = useMemo(() => {
    try {
      return JSON.stringify(JSON.parse(rawResult), null, 2);
    } catch {
      return rawResult;
    }
  }, [rawResult]);

  const handleCopy = async () => {
    try {
      const text = view === "table" ? formatTsv(data) : prettyJson;
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore clipboard error
    }
  };

  return (
    <div className="mt-1 space-y-1.5">
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
        <div className="flex items-center gap-1.5">
          <span className="inline-flex items-center rounded border border-border/50 bg-background/60 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {t("tableRowsCount", { count: data.rowCount })}
          </span>
          {data.truncated ? (
            <span className="inline-flex items-center gap-1 rounded border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              <AlertCircle className="size-2.5" />
              {t("tableTruncated")}
            </span>
          ) : null}
        </div>
        <div className="flex items-center gap-1">
          <div className="flex items-center rounded-md border border-border/50 bg-muted/30 p-0.5 text-[10px]">
            <button
              type="button"
              onClick={() => setView("table")}
              className={cn(
                "flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 transition-colors",
                view === "table"
                  ? "bg-background font-medium text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Table className="size-3" />
              {t("tableView")}
            </button>
            <button
              type="button"
              onClick={() => setView("json")}
              className={cn(
                "flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 transition-colors",
                view === "json"
                  ? "bg-background font-medium text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Code className="size-3" />
              {t("jsonView")}
            </button>
          </div>
          <button
            type="button"
            onClick={handleCopy}
            title={t("copy")}
            className="flex size-6 cursor-pointer items-center justify-center rounded-md border border-border/50 bg-background/50 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          >
            {copied ? <Check className="size-3 text-emerald-500" /> : <Copy className="size-3" />}
          </button>
        </div>
      </div>

      {view === "table" ? (
        data.rows.length === 0 ? (
          <div className="rounded-lg border border-border/60 bg-background/40 py-6 text-center text-xs text-muted-foreground">
            {t("tableEmpty")}
          </div>
        ) : (
          <div className="max-h-80 overflow-auto rounded-lg border border-border/60 bg-background/50">
            <table className="min-w-full border-collapse font-mono text-[11px] leading-snug">
              <thead>
                <tr className="sticky top-0 z-10 border-b border-border/60 bg-muted/95 backdrop-blur-xs">
                  <th className="w-10 border-r border-border/30 px-2 py-1.5 text-center text-[10px] font-medium text-muted-foreground/60 select-none">
                    #
                  </th>
                  {data.columns.map((col, idx) => (
                    <th
                      key={`${col}-${idx}`}
                      className="border-r border-border/30 px-3 py-1.5 text-left font-semibold text-foreground/90 whitespace-nowrap last:border-r-0"
                    >
                      {col}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {data.rows.map((row, rowIdx) => (
                  <tr
                    key={`row-${rowIdx}`}
                    className="border-b border-border/20 transition-colors last:border-b-0 odd:bg-muted/15 hover:bg-muted/35"
                  >
                    <td className="border-r border-border/30 px-2 py-1 text-center text-[10px] text-muted-foreground/50 select-none">
                      {rowIdx + 1}
                    </td>
                    {data.columns.map((col, colIdx) => (
                      <td
                        key={`${col}-${colIdx}`}
                        className="max-w-xs truncate border-r border-border/20 px-3 py-1 whitespace-nowrap last:border-r-0"
                      >
                        {renderCellValue(row[colIdx])}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : (
        <CodeBlock className="mt-1 max-h-80" code={prettyJson} language="json" />
      )}
    </div>
  );
}
