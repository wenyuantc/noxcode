import type { ComponentType, ReactNode } from "react";

import { cn } from "@/lib/utils";

export interface SettingCardProps {
  title?: string;
  description?: string;
  icon?: ComponentType<{ className?: string }>;
  badge?: ReactNode;
  headerAction?: ReactNode;
  children: ReactNode;
  divided?: boolean;
  className?: string;
  contentClassName?: string;
}

export function SettingCard({
  title,
  description,
  icon: Icon,
  badge,
  headerAction,
  children,
  divided = false,
  className,
  contentClassName,
}: SettingCardProps) {
  const hasHeader = Boolean(title || description || badge || headerAction || Icon);

  return (
    <section
      className={cn(
        "rounded-2xl border border-border/70 bg-card/95 shadow-xs transition-all duration-150",
        className,
      )}
    >
      {hasHeader ? (
        <div className="flex items-start justify-between gap-4 border-b border-border/50 px-5 py-4">
          <div className="flex items-start gap-3">
            {Icon ? (
              <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/50 text-muted-foreground">
                <Icon className="size-3.5" />
              </div>
            ) : null}
            <div className="space-y-0.5">
              <div className="flex flex-wrap items-center gap-2">
                {title ? (
                  <h2 className="text-sm font-medium tracking-tight text-foreground">{title}</h2>
                ) : null}
                {badge ? (
                  <span className="inline-flex items-center rounded-full border border-border/70 bg-muted/60 px-2 py-0.5 font-mono text-[10px] font-medium text-muted-foreground">
                    {badge}
                  </span>
                ) : null}
              </div>
              {description ? (
                <p className="text-xs text-muted-foreground leading-relaxed">{description}</p>
              ) : null}
            </div>
          </div>
          {headerAction ? <div className="shrink-0">{headerAction}</div> : null}
        </div>
      ) : null}
      <div className={cn(divided ? "divide-y divide-border/50" : "p-5", contentClassName)}>
        {children}
      </div>
    </section>
  );
}

export interface SettingRowProps {
  title: ReactNode;
  description?: ReactNode;
  icon?: ComponentType<{ className?: string }>;
  badge?: ReactNode;
  children?: ReactNode;
  className?: string;
  vertical?: boolean;
}

export function SettingRow({
  title,
  description,
  icon: Icon,
  badge,
  children,
  className,
  vertical = false,
}: SettingRowProps) {
  return (
    <div
      className={cn(
        "flex py-3.5 px-5 transition-colors",
        vertical
          ? "flex-col gap-2.5"
          : "flex-col sm:flex-row sm:items-center sm:justify-between gap-3",
        className,
      )}
    >
      <div className="flex items-start gap-3">
        {Icon ? (
          <div className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md border border-border/50 bg-muted/40 text-muted-foreground">
            <Icon className="size-3" />
          </div>
        ) : null}
        <div className="min-w-0 flex-1 space-y-0.5">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-foreground tracking-tight">{title}</span>
            {badge ? (
              <span className="inline-flex items-center rounded-full bg-muted px-1.5 py-0.2 text-[10px] text-muted-foreground font-mono">
                {badge}
              </span>
            ) : null}
          </div>
          {description ? (
            <p className="text-[11px] text-muted-foreground leading-relaxed max-w-xl">
              {description}
            </p>
          ) : null}
        </div>
      </div>
      {children ? <div className="shrink-0 flex items-center gap-2">{children}</div> : null}
    </div>
  );
}
