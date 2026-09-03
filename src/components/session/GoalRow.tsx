import { CheckCircle2, Circle, Target } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { GroupedSessionItem } from "@/lib/sessionLines";
import { parseGoalLine } from "@/lib/sessionLines";

export function GoalRow({ item }: { item: GroupedSessionItem }) {
  const { t } = useTranslation("sessions");
  const goal = parseGoalLine(item.text);
  if (!goal) return null;
  if (goal.cleared) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Target className="size-3.5 shrink-0" />
        {t("goalCleared")}
      </p>
    );
  }
  const done = goal.checklist.filter((entry) => entry.done).length;
  return (
    <div className="rounded-md border bg-muted/30 px-3 py-2 text-sm">
      <div className="flex items-center gap-2">
        <Target className="size-3.5 shrink-0" />
        <span className="font-medium">{goal.title}</span>
        <span className="text-xs text-muted-foreground">
          {t(`goalStatus.${goal.status}`, { defaultValue: goal.status })}
          {goal.checklist.length > 0 ? ` · ${done}/${goal.checklist.length}` : ""}
        </span>
      </div>
      {goal.checklist.length > 0 ? (
        <ul className="mt-1 space-y-0.5">
          {goal.checklist.map((entry, index) => (
            <li
              key={`${entry.item}-${index}`}
              className="flex items-center gap-2 text-xs text-muted-foreground"
            >
              {entry.done ? (
                <CheckCircle2 className="size-3.5 shrink-0 text-emerald-600" />
              ) : (
                <Circle className="size-3.5 shrink-0" />
              )}
              <span className={entry.done ? "line-through" : ""}>{entry.item}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {goal.note ? <p className="mt-1 text-xs text-muted-foreground">{goal.note}</p> : null}
    </div>
  );
}
