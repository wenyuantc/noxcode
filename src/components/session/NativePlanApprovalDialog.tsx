import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { resolveNativePlanApproval } from "@/lib/backend";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";

export function NativePlanApprovalDialog() {
  const { t } = useTranslation("sessions");
  const pending = useSessionStore((state) => state.planApproval);
  const setPlanApproval = useSessionStore((state) => state.setPlanApproval);
  const [feedback, setFeedback] = useState("");

  useEffect(() => {
    setFeedback("");
  }, [pending]);

  const resolve = (approved: boolean) => {
    if (!pending) return;
    const current = pending;
    setPlanApproval(null);
    void resolveNativePlanApproval(
      current.session_record_id,
      current.request_id,
      approved,
      feedback.trim() || undefined,
    );
  };

  return (
    <Dialog open={Boolean(pending)} onOpenChange={(open) => !open && pending && resolve(false)}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("planApprovalTitle")}</DialogTitle>
          <DialogDescription>{t("planApprovalHint")}</DialogDescription>
        </DialogHeader>
        <div className="max-h-[50vh] overflow-y-auto rounded-md border bg-muted/30 p-3">
          <AssistantMarkdown text={pending?.plan ?? ""} />
        </div>
        <Textarea
          value={feedback}
          placeholder={t("planApprovalFeedbackPlaceholder")}
          onChange={(event) => setFeedback(event.target.value)}
        />
        <DialogFooter className="gap-2">
          <Button variant="outline" onClick={() => resolve(false)}>
            {t("planApprovalReject")}
          </Button>
          <Button onClick={() => resolve(true)}>{t("planApprovalApprove")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
