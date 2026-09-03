import { ClipboardList } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Textarea } from "@/components/ui/textarea";
import { resolveNativePlanApproval } from "@/lib/backend";
import type { GroupedSessionItem, PlanLineStatus } from "@/lib/sessionLines";
import { parsePlanLine } from "@/lib/sessionLines";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";

export function PlanPillButton({
  children,
  disabled,
  onClick,
}: {
  children: ReactNode;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="inline-flex h-9 items-center justify-center rounded-full bg-foreground px-6 text-sm font-medium text-background transition hover:bg-foreground/90 disabled:pointer-events-none disabled:opacity-50"
    >
      {children}
    </button>
  );
}

function statusLabel(
  t: (key: string, options?: Record<string, string>) => string,
  status: PlanLineStatus | null,
  body: string,
  questionSummary: string | null,
): string {
  switch (status) {
    case "entered":
      return t("planEntered");
    case "waiting_approval":
      return t("planWaitingApproval");
    case "waiting_question":
      return questionSummary
        ? t("planWaitingQuestionDetail", { summary: questionSummary })
        : t("planWaitingQuestion");
    case "execute":
      return t("planStartExecute");
    default:
      return body || t("planDocument");
  }
}

export function PlanRow({ item, sessionId }: { item: GroupedSessionItem; sessionId: string }) {
  const { t } = useTranslation("sessions");
  const parsed = parsePlanLine(item.text);
  const planApproval = useSessionStore((state) => state.planApproval);
  const planQuestion = useSessionStore((state) => state.planQuestion);
  const setPlanApproval = useSessionStore((state) => state.setPlanApproval);
  const [feedback, setFeedback] = useState("");

  const pendingApproval = planApproval?.session_record_id === sessionId ? planApproval : null;
  const pendingAsk = planQuestion?.session_record_id === sessionId ? planQuestion : null;

  useEffect(() => {
    setFeedback("");
  }, [pendingApproval?.request_id]);

  if (!parsed) return null;

  if (parsed.kind === "status") {
    if (parsed.status === "waiting_question" && pendingAsk) return null;
    if (parsed.status === "waiting_approval" && pendingApproval) return null;
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <ClipboardList className="size-3.5 shrink-0" />
        <span>{statusLabel(t, parsed.status, parsed.body, parsed.questionSummary)}</span>
      </p>
    );
  }

  const title = parsed.title ?? t("planDocument");
  const showApproval = Boolean(pendingApproval);

  const resolve = (approved: boolean) => {
    if (!pendingApproval) return;
    const current = pendingApproval;
    setPlanApproval(null);
    void resolveNativePlanApproval(
      current.session_record_id,
      current.request_id,
      approved,
      feedback.trim() || undefined,
    );
  };

  return (
    <div className="rounded-xl border bg-muted/30 px-4 py-3">
      <div className="mb-2 flex items-baseline gap-2">
        <span className="text-xs font-semibold tracking-wide text-muted-foreground">
          {t("planLabel")}
        </span>
        <span className="text-sm font-medium">{title}</span>
      </div>
      <AssistantMarkdown text={parsed.body} variant="plan" />
      {showApproval ? (
        <div className="mt-3 space-y-2">
          <Textarea
            value={feedback}
            placeholder={t("planApprovalFeedbackPlaceholder")}
            onChange={(event) => setFeedback(event.target.value)}
          />
          <div className="flex flex-col items-center gap-2">
            <PlanPillButton onClick={() => resolve(true)}>{t("planContinueTask")}</PlanPillButton>
            <button
              type="button"
              className="text-sm text-muted-foreground hover:text-foreground"
              onClick={() => resolve(false)}
            >
              {t("planApprovalReject")}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
