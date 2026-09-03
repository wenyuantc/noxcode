import { useTranslation } from "react-i18next";

import { diffLineClass } from "@/lib/gitHelpers";
import type { GitFileDiff } from "@/lib/types";

export function DiffView({ diff }: { diff: GitFileDiff | null }) {
  const { t } = useTranslation("git");
  if (!diff) {
    return <p className="px-3 py-2 text-xs text-muted-foreground">{t("empty")}</p>;
  }
  if (diff.is_binary) {
    return <p className="px-3 py-2 text-xs text-muted-foreground">{t("binary")}</p>;
  }
  const lines = (diff.patch || "").split("\n");
  return (
    <div className="overflow-auto">
      {diff.truncated ? <p className="px-3 py-1 text-xs text-amber-600">{t("truncated")}</p> : null}
      <pre className="px-3 py-2 font-mono text-[11px] leading-5">
        {lines.map((line, index) => (
          <div key={`${index}-${line.slice(0, 24)}`} className={diffLineClass(line)}>
            {line || " "}
          </div>
        ))}
      </pre>
    </div>
  );
}
