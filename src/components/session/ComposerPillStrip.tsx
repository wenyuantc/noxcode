import { Bot, FileText, Package, X, Zap } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { ComposerPillsState, ComposerTargetPill } from "@/lib/composerPills";
import { cn } from "@/lib/utils";

interface ComposerPillStripProps {
  pills: ComposerPillsState;
  onRemoveTarget: () => void;
  onRemoveFile: (filePath: string) => void;
  className?: string;
}

function PillContainer({
  children,
  className,
  title,
  onRemove,
  removeLabel,
}: {
  children: ReactNode;
  className?: string;
  title?: string;
  onRemove: () => void;
  removeLabel: string;
}) {
  return (
    <span
      title={title}
      className={cn(
        "group/pill inline-flex h-6.5 max-w-full items-center gap-1.5 rounded-md border px-2 text-xs font-medium transition-all duration-150 select-none",
        className,
      )}
    >
      {children}
      <button
        type="button"
        tabIndex={-1}
        aria-label={removeLabel}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
        className="ml-0.5 -mr-0.5 inline-flex size-4 cursor-pointer items-center justify-center rounded-sm opacity-60 transition-opacity hover:opacity-100 hover:bg-black/10 dark:hover:bg-white/15"
      >
        <X className="size-3" />
      </button>
    </span>
  );
}

function fileNameFromPath(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function TargetPillItem({
  target,
  onRemove,
}: {
  target: ComposerTargetPill;
  onRemove: () => void;
}) {
  const { t } = useTranslation("sessions");

  if (target.kind === "skill") {
    const tooltip = [target.name, target.description, target.sourceLabel]
      .filter(Boolean)
      .join(" · ");
    return (
      <PillContainer
        title={tooltip}
        onRemove={onRemove}
        removeLabel={t("removePill", { name: target.name })}
        className="border-cyan-500/30 bg-cyan-500/10 text-cyan-900 dark:text-cyan-200"
      >
        <Package className="size-3.5 shrink-0 text-cyan-600 dark:text-cyan-400" />
        <span className="truncate">{target.name}</span>
        {target.sourceLabel ? (
          <span className="text-[10px] text-cyan-700/70 dark:text-cyan-300/70">
            {target.sourceLabel}
          </span>
        ) : null}
      </PillContainer>
    );
  }

  if (target.kind === "subagent") {
    const tooltip = [target.name, target.description].filter(Boolean).join(" · ");
    return (
      <PillContainer
        title={tooltip}
        onRemove={onRemove}
        removeLabel={t("removePill", { name: target.name })}
        className="border-purple-500/30 bg-purple-500/10 text-purple-900 dark:text-purple-200"
      >
        <Bot className="size-3.5 shrink-0 text-purple-600 dark:text-purple-400" />
        <span className="truncate">{target.name}</span>
      </PillContainer>
    );
  }

  // command
  const tooltip = [`/${target.name}`, target.argumentHint, target.description]
    .filter(Boolean)
    .join(" · ");
  return (
    <PillContainer
      title={tooltip}
      onRemove={onRemove}
      removeLabel={t("removePill", { name: target.name })}
      className="border-amber-500/30 bg-amber-500/10 text-amber-900 dark:text-amber-200"
    >
      <Zap className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
      <span className="font-mono truncate">/{target.name}</span>
    </PillContainer>
  );
}

export function ComposerPillStrip({
  pills,
  onRemoveTarget,
  onRemoveFile,
  className,
}: ComposerPillStripProps) {
  const { t } = useTranslation("sessions");

  if (!pills.target && pills.files.length === 0) return null;

  return (
    <div
      role="region"
      aria-label={t("composerPillsTitle", { defaultValue: "Attached context and targets" })}
      className={cn(
        "flex flex-wrap items-center gap-1.5 px-3.5 pt-2.5 pb-1 select-none",
        className,
      )}
    >
      {pills.target ? <TargetPillItem target={pills.target} onRemove={onRemoveTarget} /> : null}

      {pills.files.map((file) => {
        const shortName = fileNameFromPath(file);
        return (
          <PillContainer
            key={file}
            title={file}
            onRemove={() => onRemoveFile(file)}
            removeLabel={t("removePill", { name: shortName })}
            className="border-border/80 bg-muted/60 text-foreground/90 hover:bg-muted/80"
          >
            <FileText className="size-3.5 shrink-0 text-muted-foreground" />
            <span className="max-w-48 truncate">{shortName}</span>
          </PillContainer>
        );
      })}
    </div>
  );
}
