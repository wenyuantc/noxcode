import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useSteerStore } from "@/stores/steerStore";
import { useUiStore } from "@/stores/uiStore";

export function SteerInputs({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const receipts = useSteerStore((state) => state.snapshots[sessionId]?.receipts);
  const [restored, setRestored] = [
    useUiStore((state) => state.composerDraft),
    useUiStore((state) => state.setComposerDraft),
  ];
  if (!receipts?.length) return null;
  return (
    <div
      className="mb-2 max-h-44 space-y-2 overflow-y-auto text-xs"
      aria-label={t("steer.history")}
      aria-live="polite"
    >
      {receipts.map((receipt) => (
        <div key={receipt.input_id} className="rounded-lg border border-border/60 p-2">
          <span className="font-medium">{t(`steer.${receipt.status}`)}</span>
          <p className="line-clamp-3 whitespace-pre-wrap break-words">
            {receipt.text.slice(0, 2000)}
          </p>
          {receipt.image_count > 0 ? (
            <p>{t("steer.images", { count: receipt.image_count })}</p>
          ) : null}
          {receipt.error ? <p className="text-destructive">{receipt.error}</p> : null}
          {receipt.status === "cancelled" || receipt.status === "rejected" ? (
            <>
              {receipt.image_count > 0 ? <p>{t("steer.reselect")}</p> : null}
              <Button
                size="sm"
                variant="ghost"
                disabled={!receipt.text}
                onClick={() =>
                  setRestored(restored ? `${restored}\n${receipt.text}` : receipt.text)
                }
              >
                {t("steer.restore")}
              </Button>
            </>
          ) : null}
        </div>
      ))}
    </div>
  );
}
