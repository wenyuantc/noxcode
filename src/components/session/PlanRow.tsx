import { ClipboardList } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Textarea } from "@/components/ui/textarea";
import { resolveNativePlanApproval } from "@/lib/backend";
import { resolveSessionRequest } from "@/lib/nativeRequestResolution";
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
  const pendingApproval = useSessionStore(
    (state) => Object.values(state.planApprovals[sessionId] ?? {})[0],
  );
  const pendingAsk = useSessionStore(
    (state) => Object.values(state.planQuestions[sessionId] ?? {})[0],
  );

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

  return (
    <div className="rounded-lg border bg-muted/30 px-4 py-3">
      <p className="mb-2 text-sm font-medium">{title}</p>
      <AssistantMarkdown text={parsed.body} variant="plan" />
    </div>
  );
}

export function PendingPlanApproval({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const pendingApproval = useSessionStore(
    (state) => Object.values(state.planApprovals[sessionId] ?? {})[0],
  );
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setFeedback("");
    setError(null);
  }, [pendingApproval?.request_id]);
  if (!pendingApproval) return null;

  const resolve = async (approved: boolean) => {
    if (busy) return;
    const current = pendingApproval;
    setBusy(true);
    setError(null);
    try {
      await resolveSessionRequest({ ...current, kind: "plan_approval" }, () =>
        resolveNativePlanApproval(
          current.session_record_id,
          current.request_id,
          approved,
          feedback.trim() || undefined,
        ),
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-xl border bg-muted/30 px-4 py-3">
      <div className="mb-2 flex items-baseline gap-2">
        <span className="text-xs font-semibold tracking-wide text-muted-foreground">
          {t("planLabel")}
        </span>
        <span className="text-sm font-medium">{t("planWaitingApproval")}</span>
      </div>
      <AssistantMarkdown text={pendingApproval.plan} variant="plan" />
      <div className="mt-3 space-y-2">
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <Textarea
          value={feedback}
          placeholder={t("planApprovalFeedbackPlaceholder")}
          onChange={(event) => setFeedback(event.target.value)}
        />
        <div className="flex flex-col items-center gap-2">
          <PlanPillButton
            disabled={busy}
            onClick={() => {
              void resolve(true);
            }}
          >
            {t("planApprovalApprove")}
          </PlanPillButton>
          <button
            type="button"
            className="text-sm text-muted-foreground hover:text-foreground"
            disabled={busy}
            onClick={() => resolve(false)}
          >
            {t("planApprovalReject")}
          </button>
        </div>
      </div>
    </div>
  );
}
