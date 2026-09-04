import { useTranslation } from "react-i18next";

import type { ComposerSlashGroup, ComposerSlashItem } from "@/lib/composerSlash";
import { groupComposerSlashItems } from "@/lib/composerSlash";
import { cn } from "@/lib/utils";

const GROUP_LABEL: Record<ComposerSlashGroup, "slashCommands" | "slashSkills" | "slashSubagents"> =
  {
    commands: "slashCommands",
    skills: "slashSkills",
    subagents: "slashSubagents",
  };

interface ComposerSlashMenuProps {
  items: ComposerSlashItem[];
  activeIndex: number;
  listRef: React.RefObject<HTMLDivElement | null>;
  onHover: (index: number) => void;
  onPick: (item: ComposerSlashItem) => void;
}

export function ComposerSlashMenu({
  items,
  activeIndex,
  listRef,
  onHover,
  onPick,
}: ComposerSlashMenuProps) {
  const { t } = useTranslation("sessions");
  const grouped = groupComposerSlashItems(items);
  let cursor = -1;

  if (items.length === 0) {
    return (
      <div className="mx-3 mb-2 rounded-xl border border-border/60 bg-popover p-2 text-sm shadow-md">
        <p className="text-xs text-muted-foreground">{t("slashEmpty")}</p>
      </div>
    );
  }

  return (
    <div
      ref={listRef}
      className="mx-3 mb-2 max-h-64 overflow-y-auto rounded-xl border border-border/60 bg-popover p-1 text-sm shadow-md"
    >
      <p className="px-2.5 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {t("slashTitle")}
      </p>
      {grouped.map((section) => (
        <div key={section.group} className="mb-1">
          <p className="px-2.5 py-1 text-[10px] text-muted-foreground">
            {t(GROUP_LABEL[section.group])}
          </p>
          {section.items.map((item) => {
            cursor += 1;
            const index = cursor;
            return (
              <button
                key={item.key}
                type="button"
                data-mention-active={index === activeIndex ? "true" : undefined}
                className={cn(
                  "block w-full rounded-lg px-2.5 py-1.5 text-left transition-colors",
                  index === activeIndex ? "bg-accent text-accent-foreground" : "hover:bg-accent/70",
                )}
                onMouseEnter={() => onHover(index)}
                onClick={() => onPick(item)}
              >
                <span className="flex items-center gap-2">
                  <span className="truncate text-xs font-medium">
                    {item.group === "skills"
                      ? `$${item.name}`
                      : item.group === "commands"
                        ? `/${item.name}`
                        : item.name}
                  </span>
                  {item.sourceLabel ? (
                    <span className="truncate text-[10px] text-muted-foreground">
                      {item.sourceLabel}
                    </span>
                  ) : null}
                </span>
                {item.description ? (
                  <span className="block truncate text-[11px] text-muted-foreground">
                    {item.description}
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}
