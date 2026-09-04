import { AlertCircle, FileCode2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { diffLineClass } from "@/lib/gitHelpers";
import type { GitFileDiff } from "@/lib/types";

export function DiffView({ diff }: { diff: GitFileDiff | null }) {
  const { t } = useTranslation("git");
  if (!diff) {
    return null;
  }
  if (diff.is_binary) {
    return (
      <div className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
        <FileCode2 className="size-4 opacity-50" />
        {t("binary")}
      </div>
    );
  }
  const lines = (diff.patch || "").split("\n");
  return (
    <div className="overflow-x-auto text-[11px] leading-5">
      {diff.truncated ? (
        <div className="flex items-center gap-1.5 border-b border-amber-500/20 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-300">
          <AlertCircle className="size-3.5 shrink-0" />
          <span>{t("truncated")}</span>
        </div>
      ) : null}
      <pre className="py-1.5 font-mono">
        {lines.map((line, index) => {
          const isAdd = line.startsWith("+") && !line.startsWith("+++");
          const isDel = line.startsWith("-") && !line.startsWith("---");
          const isHunk = line.startsWith("@@");
          return (
            <div
              key={`${index}-${line.slice(0, 24)}`}
              className={`whitespace-pre px-3 py-0.5 transition-colors ${diffLineClass(line)} ${
                isAdd
                  ? "border-l-2 border-emerald-500"
                  : isDel
                    ? "border-l-2 border-rose-500"
                    : isHunk
                      ? "bg-muted/60 font-semibold text-sky-700 dark:text-sky-300"
                      : ""
              }`}
            >
              {line || " "}
            </div>
          );
        })}
      </pre>
    </div>
  );
}
