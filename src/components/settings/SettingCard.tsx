import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function SettingCard({
  title,
  description,
  badge,
  children,
  className,
}: {
  title: string;
  description?: string;
  badge?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("rounded-xl border bg-card p-4", className)}>
      <div className="mb-3">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-medium">{title}</h2>
          {badge ? (
            <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">
              {badge}
            </span>
          ) : null}
        </div>
        {description ? <p className="mt-1 text-xs text-muted-foreground">{description}</p> : null}
      </div>
      {children}
    </section>
  );
}
