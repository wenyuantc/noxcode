import { Popover } from "@base-ui/react/popover";
import type { LucideIcon } from "lucide-react";
import type { ComponentProps, ReactNode, RefObject } from "react";

import { cn } from "@/lib/utils";

interface ComposerMentionMenuProps {
  open: boolean;
  anchorRef: RefObject<HTMLDivElement | null>;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  listRef: RefObject<HTMLDivElement | null>;
  id: string;
  label: string;
  onDismiss: () => void;
  children: ReactNode;
}

export function ComposerMentionMenu({
  open,
  anchorRef,
  inputRef,
  listRef,
  id,
  label,
  onDismiss,
  children,
}: ComposerMentionMenuProps) {
  return (
    <Popover.Root
      open={open}
      onOpenChange={(nextOpen, details) => {
        if (nextOpen) return;
        if (
          details.reason === "outside-press" &&
          details.event.target instanceof Node &&
          inputRef.current?.contains(details.event.target)
        ) {
          details.cancel();
          return;
        }
        onDismiss();
      }}
    >
      <Popover.Portal>
        <Popover.Positioner
          anchor={anchorRef}
          side="top"
          align="start"
          sideOffset={8}
          collisionPadding={8}
          collisionAvoidance={{ side: "none", align: "shift", fallbackAxisSide: "none" }}
          className="z-50 w-(--anchor-width) max-w-[calc(100vw-16px)]"
        >
          <Popover.Popup
            ref={listRef}
            id={id}
            role="listbox"
            aria-label={label}
            initialFocus={inputRef}
            finalFocus={false}
            className="max-h-[min(20rem,var(--available-height))] overflow-x-hidden overflow-y-auto overscroll-contain rounded-xl border border-border/80 bg-popover p-1 text-popover-foreground shadow-lg ring-1 ring-foreground/5 outline-none dark:shadow-black/30"
          >
            {children}
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}

interface ComposerMentionOptionProps extends ComponentProps<"button"> {
  active: boolean;
  icon: LucideIcon;
  label: string;
  description?: string;
  sourceLabel?: string;
}

export function ComposerMentionOption({
  active,
  icon: Icon,
  label,
  description,
  sourceLabel,
  ...props
}: ComposerMentionOptionProps) {
  return (
    <button
      {...props}
      type="button"
      role="option"
      aria-selected={active}
      tabIndex={-1}
      data-mention-active={active ? "true" : undefined}
      title={[label, description, sourceLabel].filter(Boolean).join("\n")}
      className={cn(
        "flex min-h-8 w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm transition-colors",
        active ? "bg-accent text-accent-foreground" : "hover:bg-accent/70",
      )}
      onMouseDown={(event) => event.preventDefault()}
    >
      <Icon aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
      <span className={cn("min-w-0 truncate", description ? "max-w-[55%]" : "flex-1")}>
        {label}
      </span>
      {description ? (
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{description}</span>
      ) : null}
      {sourceLabel ? (
        <span className="hidden max-w-24 shrink-0 truncate text-xs text-muted-foreground/70 sm:inline">
          {sourceLabel}
        </span>
      ) : null}
    </button>
  );
}
