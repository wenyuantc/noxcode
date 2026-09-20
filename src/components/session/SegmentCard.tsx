import { ChevronRight } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export function SegmentCard({
  icon,
  iconClassName,
  badge,
  title,
  running = false,
  tone = "default",
  defaultOpen = false,
  open: controlledOpen,
  onOpenChange,
  contentClassName,
  children,
}: {
  icon: ReactNode;
  iconClassName?: string;
  badge?: string;
  title: ReactNode;
  running?: boolean;
  tone?: "default" | "danger";
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  contentClassName?: string;
  children: ReactNode;
}) {
  const { t } = useTranslation("sessions");
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;
  const danger = tone === "danger";

  const handleToggle = () => {
    const next = !open;
    if (!isControlled) {
      setInternalOpen(next);
    }
    onOpenChange?.(next);
  };

  return (
    <div
      className={cn(
        "rounded-xl border transition-all duration-150",
        danger
          ? "border-rose-500/30 bg-rose-500/5 hover:border-rose-500/50 dark:bg-rose-500/[0.06]"
          : "border-border/60 bg-muted/15 hover:border-border/80",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left text-xs"
        onClick={handleToggle}
      >
        <span
          className={cn(
            "flex size-5 shrink-0 items-center justify-center rounded-md",
            danger ? "bg-rose-500/15 text-rose-600 dark:text-rose-400" : iconClassName,
          )}
        >
          {icon}
        </span>
        {badge ? (
          <span className="inline-flex shrink-0 items-center rounded border border-border/40 bg-background/50 px-1.5 py-0.5 text-badge font-medium text-muted-foreground">
            {badge}
          </span>
        ) : null}
        <span
          className={cn(
            "min-w-0 flex-1 truncate text-left font-medium tracking-tight",
            danger ? "text-rose-600 dark:text-rose-400" : "text-foreground/80",
          )}
        >
          {title}
        </span>
        {running ? (
          <span className="flex shrink-0 items-center gap-1.5 text-meta font-medium text-amber-500">
            <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
            {t("running")}
          </span>
        ) : null}
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground/70 transition-transform duration-150",
            open && "rotate-90",
          )}
        />
      </button>
      {open ? (
        <div className={cn("border-t border-border/40 p-3", contentClassName)}>{children}</div>
      ) : null}
    </div>
  );
}
