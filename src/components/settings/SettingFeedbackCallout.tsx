import { AlertCircle, AlertTriangle, CheckCircle2, Info, Loader2, X } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export type FeedbackVariant = "success" | "error" | "info" | "warning" | "loading";

interface SettingFeedbackCalloutProps {
  variant?: FeedbackVariant;
  title?: string;
  message: ReactNode;
  onClose?: () => void;
  className?: string;
}

const VARIANT_CONFIGS = {
  success: {
    icon: CheckCircle2,
    wrapper:
      "border-emerald-500/30 bg-emerald-500/10 text-emerald-950 dark:text-emerald-100 dark:border-emerald-500/25",
    iconColor: "text-emerald-600 dark:text-emerald-400",
  },
  error: {
    icon: AlertCircle,
    wrapper:
      "border-destructive/30 bg-destructive/10 text-destructive dark:border-destructive/25 dark:text-destructive-foreground",
    iconColor: "text-destructive",
  },
  warning: {
    icon: AlertTriangle,
    wrapper:
      "border-amber-500/30 bg-amber-500/10 text-amber-950 dark:text-amber-100 dark:border-amber-500/25",
    iconColor: "text-amber-600 dark:text-amber-400",
  },
  info: {
    icon: Info,
    wrapper: "border-border/80 bg-muted/60 text-foreground",
    iconColor: "text-muted-foreground",
  },
  loading: {
    icon: Loader2,
    wrapper: "border-primary/30 bg-primary/10 text-foreground",
    iconColor: "text-primary animate-spin",
  },
};

export function SettingFeedbackCallout({
  variant = "info",
  title,
  message,
  onClose,
  className,
}: SettingFeedbackCalloutProps) {
  if (!message) return null;

  const config = VARIANT_CONFIGS[variant];
  const Icon = config.icon;

  return (
    <div
      role="status"
      className={cn(
        "relative flex items-start gap-3 rounded-xl border px-3.5 py-2.5 text-xs shadow-2xs transition-all duration-150 animate-in fade-in-50",
        config.wrapper,
        className,
      )}
    >
      <Icon className={cn("mt-0.5 size-4 shrink-0", config.iconColor)} />
      <div className="min-w-0 flex-1">
        {title ? <p className="mb-0.5 font-medium leading-tight">{title}</p> : null}
        <div className="leading-relaxed opacity-95">{message}</div>
      </div>
      {onClose ? (
        <button
          type="button"
          onClick={onClose}
          className="-mr-1 -mt-1 rounded-md p-1 opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label="Close"
        >
          <X className="size-3.5" />
        </button>
      ) : null}
    </div>
  );
}
