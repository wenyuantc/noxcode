import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function UsageTooltipProvider({ children }: { children: ReactNode }) {
  return (
    <TooltipPrimitive.Provider delay={50} closeDelay={100}>
      {children}
    </TooltipPrimitive.Provider>
  );
}

export function UsageTooltip({
  children,
  content,
  side = "top",
  sideOffset = 6,
  disabled = false,
  className,
  triggerClassName,
}: {
  children: ReactNode;
  content: ReactNode;
  side?: "top" | "bottom" | "left" | "right";
  sideOffset?: number;
  disabled?: boolean;
  className?: string;
  triggerClassName?: string;
}) {
  if (disabled || !content) {
    return <>{children}</>;
  }

  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger
        render={(props) => (
          <div
            {...props}
            className={cn(triggerClassName ?? "inline-flex h-full w-full", props.className)}
          >
            {children}
          </div>
        )}
      />
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Positioner
          side={side}
          sideOffset={sideOffset}
          className="isolate z-50 pointer-events-none"
        >
          <TooltipPrimitive.Popup
            className={cn(
              "z-50 min-w-36 max-w-xs rounded-lg border border-neutral-800 bg-neutral-900/95 px-3 py-2 text-xs text-neutral-100 shadow-xl backdrop-blur-xs transition-all",
              "data-[side=top]:slide-in-from-bottom-1 data-[side=bottom]:slide-in-from-top-1 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95",
              className,
            )}
          >
            {content}
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}
