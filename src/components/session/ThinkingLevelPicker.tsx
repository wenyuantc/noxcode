import {
  Brain,
  ChevronDown,
  CircleOff,
  Flame,
  Gauge,
  Rabbit,
  Rocket,
  Sparkles,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

function EffortIcon({ level, className }: { level: string; className?: string }) {
  const iconClass = cn("size-3.5 shrink-0", className);
  switch (level) {
    case "none":
    case "no_think":
      return <CircleOff className={iconClass} />;
    case "minimal":
      return <Gauge className={iconClass} />;
    case "low":
      return <Rabbit className={iconClass} />;
    case "high":
      return <Sparkles className={iconClass} />;
    case "xhigh":
      return <Flame className={iconClass} />;
    case "max":
      return <Rocket className={iconClass} />;
    default:
      return <Brain className={iconClass} />;
  }
}

export function ThinkingLevelPicker({
  value,
  levels,
  onChange,
}: {
  value: string;
  levels: string[];
  onChange: (level: string) => void;
}) {
  const { t, i18n } = useTranslation("sessions");

  const titleOf = (level: string) => t(`effortLevels.${level}.title`, { defaultValue: level });
  const descriptionOf = (level: string) => {
    const key = `effortLevels.${level}.description`;
    return i18n.exists(key, { ns: "sessions" }) ? t(key) : "";
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="inline-flex h-7 items-center justify-between gap-1 rounded-md border bg-background px-2 text-xs outline-none">
        <EffortIcon level={value} />
        <span className="truncate">{titleOf(value)}</span>
        <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuRadioGroup value={value} onValueChange={(next) => next && onChange(next)}>
          {levels.map((level) => {
            const description = descriptionOf(level);
            return (
              <DropdownMenuRadioItem
                key={level}
                value={level}
                closeOnClick
                className="items-start py-2"
              >
                <EffortIcon level={level} className="mt-0.5" />
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="font-medium">{titleOf(level)}</span>
                  {description ? (
                    <span className="text-xs text-muted-foreground">{description}</span>
                  ) : null}
                </span>
              </DropdownMenuRadioItem>
            );
          })}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
