import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Input } from "@/components/ui/input";
import { answerNativePlanQuestion } from "@/lib/backend";
import { PLAN_QUESTION_OTHER, resolvePlanQuestionAnswer } from "@/lib/nativePlanQuestion";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { PlanPillButton } from "./PlanRow";

export function PlanAskCard({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore((state) => state.planQuestion);
  const setPlanQuestion = useSessionStore((state) => state.setPlanQuestion);
  const [selections, setSelections] = useState<string[]>([]);
  const [otherTexts, setOtherTexts] = useState<string[]>([]);

  const active = pending?.session_record_id === sessionId ? pending : null;

  const requestId = active?.request_id;
  const questionCount = active?.questions.length ?? 0;

  useEffect(() => {
    setSelections(Array.from({ length: questionCount }, () => ""));
    setOtherTexts(Array.from({ length: questionCount }, () => ""));
  }, [requestId, questionCount]);

  if (!active) return null;

  const answers = active.questions.map((question, index) =>
    resolvePlanQuestionAnswer(question.options, selections[index] ?? "", otherTexts[index] ?? ""),
  );
  const numbered = active.questions.length > 1;
  const canSubmit = answers.every((item) => item);

  const submit = (skipped: boolean) => {
    const current = active;
    setPlanQuestion(null);
    void answerNativePlanQuestion(
      current.session_record_id,
      current.request_id,
      skipped,
      answers.map((item) => item ?? ""),
    );
  };

  return (
    <div className="rounded-xl border bg-muted/30 px-4 py-3">
      <div className="flex items-start justify-between gap-2">
        <div>
          <p className="text-sm font-semibold">{t("planAskLabel")}</p>
          <p className="text-xs text-muted-foreground">{t("planAskHint")}</p>
        </div>
        <button
          type="button"
          className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          title={t("planAskClose")}
          aria-label={t("planAskClose")}
          onClick={() => submit(true)}
        >
          <X className="size-3.5" />
        </button>
      </div>

      <div className="mt-3 space-y-4">
        {active.questions.map((question, index) => {
          const hasOptions = question.options.length >= 2;
          const selected = selections[index] ?? "";
          const showOther = !hasOptions || selected === PLAN_QUESTION_OTHER;
          return (
            <div key={`${question.prompt}-${index}`} className="space-y-2">
              {numbered ? (
                <p className="text-sm font-medium">{t("planAskNumbered", { index: index + 1 })}</p>
              ) : null}
              <p className="text-xs text-muted-foreground">{t("planAskQuestionLabel")}</p>
              <p className="text-sm leading-6">{question.prompt}</p>
              {hasOptions ? (
                <div className="flex flex-col gap-1.5">
                  {question.options.map((option) => (
                    <button
                      key={option}
                      type="button"
                      className={cn(
                        "rounded-lg border px-3 py-2 text-left text-sm font-medium",
                        selected === option
                          ? "border-foreground bg-background"
                          : "border-border hover:bg-muted/60",
                      )}
                      onClick={() =>
                        setSelections((current) => {
                          const next = [...current];
                          next[index] = option;
                          return next;
                        })
                      }
                    >
                      {option}
                    </button>
                  ))}
                  <button
                    type="button"
                    className={cn(
                      "rounded-lg border px-3 py-2 text-left text-sm",
                      selected === PLAN_QUESTION_OTHER
                        ? "border-foreground bg-background font-medium"
                        : "border-border text-muted-foreground hover:bg-muted/60",
                    )}
                    onClick={() =>
                      setSelections((current) => {
                        const next = [...current];
                        next[index] = PLAN_QUESTION_OTHER;
                        return next;
                      })
                    }
                  >
                    {t("planAskOther")}
                  </button>
                </div>
              ) : null}
              {showOther ? (
                <Input
                  value={otherTexts[index] ?? ""}
                  placeholder={t("planAskPlaceholder")}
                  onChange={(event) =>
                    setOtherTexts((current) => {
                      const next = [...current];
                      next[index] = event.target.value;
                      return next;
                    })
                  }
                />
              ) : null}
            </div>
          );
        })}
      </div>

      <div className="mt-4 flex flex-col items-center gap-2">
        <PlanPillButton disabled={!canSubmit} onClick={() => submit(false)}>
          {t("planAskSend")}
        </PlanPillButton>
        <button
          type="button"
          className="text-sm text-muted-foreground hover:text-foreground"
          onClick={() => submit(true)}
        >
          {t("planAskCancel")}
        </button>
      </div>
    </div>
  );
}
