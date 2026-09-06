import { Bot, CircleSlash, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { ComposerSlashGroup, ComposerSlashItem } from "@/lib/composerSlash";
import { groupComposerSlashItems } from "@/lib/composerSlash";
import { ComposerMentionOption } from "./ComposerMentionMenu";

const GROUP_LABEL: Record<ComposerSlashGroup, "slashCommands" | "slashSkills" | "slashSubagents"> =
  {
    commands: "slashCommands",
    skills: "slashSkills",
    subagents: "slashSubagents",
  };

interface ComposerSlashMenuProps {
  items: ComposerSlashItem[];
  activeIndex: number;
  listId: string;
  emptyLabel: string;
  onHover: (index: number) => void;
  onPick: (item: ComposerSlashItem) => void;
}

export function ComposerSlashMenu({
  items,
  activeIndex,
  listId,
  emptyLabel,
  onHover,
  onPick,
}: ComposerSlashMenuProps) {
  const { t } = useTranslation("sessions");
  const grouped = groupComposerSlashItems(items);
  let cursor = -1;

  if (items.length === 0) {
    return (
      <p role="status" className="px-2.5 py-2 text-xs text-muted-foreground">
        {emptyLabel}
      </p>
    );
  }

  return (
    <>
      {grouped.map((section) => (
        <div
          key={section.group}
          role="group"
          aria-label={t(GROUP_LABEL[section.group])}
          className="not-first:mt-1 not-first:border-t not-first:border-border/50 not-first:pt-1"
        >
          {section.items.map((item) => {
            cursor += 1;
            const index = cursor;
            return (
              <ComposerMentionOption
                key={item.key}
                id={`${listId}-${index}`}
                active={index === activeIndex}
                icon={
                  item.group === "skills" ? Sparkles : item.group === "commands" ? CircleSlash : Bot
                }
                label={
                  item.group === "skills"
                    ? `$${item.name}`
                    : item.group === "commands"
                      ? `/${item.name}`
                      : item.name
                }
                description={item.argumentHint || item.description}
                sourceLabel={item.sourceLabel}
                onMouseEnter={() => onHover(index)}
                onClick={() => onPick(item)}
              />
            );
          })}
        </div>
      ))}
    </>
  );
}
