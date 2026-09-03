import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { answerNativePlanQuestion } from "@/lib/backend";
import { PLAN_QUESTION_OTHER, resolvePlanQuestionAnswer } from "@/lib/nativePlanQuestion";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useSessionStore } from "@/stores/sessionStore";

export function NativePlanQuestionDialog() {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore((state) => state.planQuestion);
  const setPlanQuestion = useSessionStore((state) => state.setPlanQuestion);
  const [selections, setSelections] = useState<string[]>([]);
  const [otherTexts, setOtherTexts] = useState<string[]>([]);

  useEffect(() => {
    setSelections(pending?.questions.map(() => "") ?? []);
    setOtherTexts(pending?.questions.map(() => "") ?? []);
  }, [pending]);

  const answers =
    pending?.questions.map((question, index) =>
      resolvePlanQuestionAnswer(question.options, selections[index] ?? "", otherTexts[index] ?? ""),
    ) ?? [];

  const submit = (skipped: boolean) => {
    if (!pending) return;
    const current = pending;
    setPlanQuestion(null);
    void answerNativePlanQuestion(
      current.session_record_id,
      current.request_id,
      skipped,
      answers.map((item) => item ?? ""),
    );
  };

  return (
    <Dialog open={Boolean(pending)} onOpenChange={(open) => !open && pending && submit(true)}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("planQuestionTitle")}</DialogTitle>
        </DialogHeader>
        <div className="max-h-80 space-y-4 overflow-y-auto">
          {pending?.questions.map((question, index) => (
            <div key={`${question.prompt}-${index}`} className="space-y-2">
              <p className="text-sm font-medium">{question.prompt}</p>
              {question.options.length >= 2 ? (
                <div className="flex flex-col gap-1">
                  {question.options.map((option) => (
                    <button
                      key={option}
                      type="button"
                      className={`rounded-md border px-2 py-1.5 text-left text-sm ${
                        selections[index] === option ? "border-primary bg-primary/10" : ""
                      }`}
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
                    className="rounded-md border px-2 py-1.5 text-left text-sm"
                    onClick={() =>
                      setSelections((current) => {
                        const next = [...current];
                        next[index] = PLAN_QUESTION_OTHER;
                        return next;
                      })
                    }
                  >
                    {t("planSkip")}
                  </button>
                  {selections[index] === PLAN_QUESTION_OTHER ? (
                    <Input
                      value={otherTexts[index] ?? ""}
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
              ) : (
                <Input
                  value={otherTexts[index] ?? ""}
                  onChange={(event) =>
                    setOtherTexts((current) => {
                      const next = [...current];
                      next[index] = event.target.value;
                      return next;
                    })
                  }
                />
              )}
            </div>
          ))}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => submit(true)}>
            {t("planSkip")}
          </Button>
          <Button disabled={!answers.every((item) => item)} onClick={() => submit(false)}>
            {t("planSubmit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
