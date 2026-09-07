import { AlertCircle, HelpCircle, Loader2, Send, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { answerNativePlanQuestion } from "@/lib/backend";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
import { PLAN_QUESTION_OTHER, resolvePlanQuestionAnswer } from "@/lib/nativePlanQuestion";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";

export function PlanAskCard({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore(
    (state) => Object.values(state.planQuestions[sessionId] ?? {})[0],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selections, setSelections] = useState<string[]>([]);
  const [otherTexts, setOtherTexts] = useState<string[]>([]);

  const active = pending?.session_record_id === sessionId ? pending : null;

  const requestId = active?.request_id;
  const questionCount = active?.questions.length ?? 0;

  useEffect(() => {
    setSelections(Array.from({ length: questionCount }, () => ""));
    setOtherTexts(Array.from({ length: questionCount }, () => ""));
    setError(null);
  }, [requestId, questionCount]);

  if (!active) return null;

  const answers = active.questions.map((question, index) =>
    resolvePlanQuestionAnswer(question.options, selections[index] ?? "", otherTexts[index] ?? ""),
  );
  const numbered = active.questions.length > 1;
  const canSubmit = answers.every((item) => item);

  const submit = async (skipped: boolean) => {
    if (busy) return;
    const current = active;
    setBusy(true);
    setError(null);
    try {
      await resolveSessionRequest({ ...current, kind: "question" }, () =>
        answerNativePlanQuestion(
          current.session_record_id,
          current.request_id,
          skipped,
          answers.map((item) => item ?? ""),
        ),
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/85 text-card-foreground shadow-xs backdrop-blur-md transition-all dark:border-border/60 dark:bg-card/50">
      <div className="h-0.5 w-full bg-gradient-to-r from-purple-500 via-primary/50 to-purple-500/20" />

      <div className="flex items-center justify-between border-b border-border/50 bg-muted/20 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-purple-500/10 text-purple-600 dark:text-purple-400">
            <HelpCircle className="size-4" strokeWidth={2} />
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold tracking-tight text-foreground">
              {t("planAskLabel")}
            </p>
            <p className="truncate text-xs text-muted-foreground">{t("planAskHint")}</p>
          </div>
        </div>
        <button
          type="button"
          className="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          title={t("planAskClose")}
          aria-label={t("planAskClose")}
          disabled={busy}
          onClick={() => submit(true)}
        >
          <X className="size-3.5" />
        </button>
      </div>

      <div className="p-4">
        <div className="space-y-4">
          {active.questions.map((question, index) => {
            const hasOptions = question.options.length >= 2;
            const selected = selections[index] ?? "";
            const showOther = !hasOptions || selected === PLAN_QUESTION_OTHER;
            return (
              <div key={`${question.prompt}-${index}`} className="space-y-2.5">
                {numbered ? (
                  <p className="text-xs font-semibold tracking-wide text-primary">
                    {t("planAskNumbered", { index: index + 1 })}
                  </p>
                ) : null}
                <p className="text-sm font-medium leading-relaxed text-foreground">
                  {question.prompt}
                </p>
                {hasOptions ? (
                  <div className="flex flex-col gap-1.5">
                    {question.options.map((option) => (
                      <button
                        key={option}
                        type="button"
                        className={cn(
                          "group flex w-full items-center justify-between rounded-xl border px-3.5 py-2.5 text-left text-sm transition-all",
                          selected === option
                            ? "border-primary/80 bg-primary/5 font-medium shadow-xs"
                            : "border-border/70 text-foreground/90 hover:border-border hover:bg-muted/40",
                        )}
                        onClick={() =>
                          setSelections((current) => {
                            const next = [...current];
                            next[index] = option;
                            return next;
                          })
                        }
                      >
                        <span className="leading-snug">{option}</span>
                        <div
                          className={cn(
                            "flex size-4 shrink-0 items-center justify-center rounded-full border transition-all",
                            selected === option
                              ? "border-primary bg-primary text-primary-foreground"
                              : "border-muted-foreground/40 group-hover:border-muted-foreground",
                          )}
                        >
                          {selected === option ? (
                            <div className="size-1.5 rounded-full bg-current" />
                          ) : null}
                        </div>
                      </button>
                    ))}
                    <button
                      type="button"
                      className={cn(
                        "group flex w-full items-center justify-between rounded-xl border px-3.5 py-2.5 text-left text-sm transition-all",
                        selected === PLAN_QUESTION_OTHER
                          ? "border-primary/80 bg-primary/5 font-medium shadow-xs"
                          : "border-border/70 text-muted-foreground hover:border-border hover:bg-muted/40 hover:text-foreground",
                      )}
                      onClick={() =>
                        setSelections((current) => {
                          const next = [...current];
                          next[index] = PLAN_QUESTION_OTHER;
                          return next;
                        })
                      }
                    >
                      <span className="leading-snug">{t("planAskOther")}</span>
                      <div
                        className={cn(
                          "flex size-4 shrink-0 items-center justify-center rounded-full border transition-all",
                          selected === PLAN_QUESTION_OTHER
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-muted-foreground/40 group-hover:border-muted-foreground",
                        )}
                      >
                        {selected === PLAN_QUESTION_OTHER ? (
                          <div className="size-1.5 rounded-full bg-current" />
                        ) : null}
                      </div>
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
                    className="bg-background/70 text-xs"
                  />
                ) : null}
              </div>
            );
          })}
        </div>

        {error ? (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            <AlertCircle className="size-4 shrink-0" />
            <span>{error}</span>
          </div>
        ) : null}

        <div className="mt-4 flex items-center justify-between border-t border-border/50 pt-3">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => submit(true)}
            className="h-8 text-xs text-muted-foreground hover:text-foreground"
          >
            {t("planAskCancel")}
          </Button>
          <Button
            type="button"
            variant="default"
            size="sm"
            disabled={!canSubmit || busy}
            onClick={() => {
              void submit(false);
            }}
            className="h-8 gap-1.5 text-xs font-medium bg-primary text-primary-foreground shadow-xs hover:bg-primary/90"
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Send className="size-3.5" />}
            <span>{t("planAskSend")}</span>
          </Button>
        </div>
      </div>
    </div>
  );
}
