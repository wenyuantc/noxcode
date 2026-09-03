import { useTranslation } from "react-i18next";

import { greetingPeriod } from "@/lib/utils";
import { Composer } from "@/components/session/Composer";
import { QuickPromptChips } from "./QuickPromptChips";

export function HomeEmptyState() {
  const { t } = useTranslation("layout");
  return (
    <div className="flex h-full flex-col items-center justify-center gap-8 px-6">
      <h1 className="text-2xl font-medium tracking-tight">{t(`greeting.${greetingPeriod()}`)}</h1>
      <Composer />
      <QuickPromptChips />
    </div>
  );
}
