import { Check, Loader2, Pencil, Trash2, Undo2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  listNativeQueuedInputs,
  removeNativeQueuedInput,
  updateNativeQueuedInput,
} from "@/lib/backend";
import type { NativeInputQueue, NativeQueuedInput } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";

function QueuedInputRow({
  sessionId,
  item,
  index,
}: {
  sessionId: string;
  item: NativeQueuedInput;
  index: number;
}) {
  const { t } = useTranslation("sessions");
  const [draft, setDraft] = useState(item.text);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const act = async (send: () => Promise<NativeInputQueue>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      useSessionStore.getState().onInputQueue(await send());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const save = () => act(() => updateNativeQueuedInput(sessionId, item.id, draft, false));
  return (
    <li className="grid grid-cols-[1.25rem_minmax(0,1fr)_auto] items-start gap-2 py-2 text-sm">
      <span className="pt-1 text-xs tabular-nums text-muted-foreground">{index + 1}.</span>
      <div className="min-w-0">
        {item.editing ? (
          <Textarea
            autoFocus
            aria-label={t("queuedInput.edit")}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            disabled={busy}
            className="min-h-16 resize-y text-sm"
            onKeyDown={(event) => {
              if (
                (event.metaKey || event.ctrlKey) &&
                event.key === "Enter" &&
                (draft.trim() || item.image_count)
              ) {
                event.preventDefault();
                void save();
              }
              if (event.key === "Escape") {
                event.preventDefault();
                void act(() => updateNativeQueuedInput(sessionId, item.id, null, false));
              }
            }}
          />
        ) : (
          <p className="whitespace-pre-wrap break-words pt-0.5 [overflow-wrap:anywhere]">
            {item.text}
          </p>
        )}
        {item.image_count > 0 ? (
          <span className="text-xs text-muted-foreground">
            {t("queuedInput.images", { count: item.image_count })}
          </span>
        ) : null}
        {item.editing ? (
          <span className="text-xs text-muted-foreground">{t("queuedInput.editing")}</span>
        ) : null}
        {error ? (
          <p role="alert" className="break-words text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 gap-0.5">
        {item.editing ? (
          <>
            <Button
              size="icon"
              variant="ghost"
              className="size-7"
              title={t("queuedInput.save")}
              aria-label={t("queuedInput.save")}
              disabled={busy || (!draft.trim() && !item.image_count)}
              onClick={() => void save()}
            >
              {busy ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Check className="size-3.5" />
              )}
            </Button>
            <Button
              size="icon"
              variant="ghost"
              className="size-7"
              title={t("queuedInput.cancelEdit")}
              aria-label={t("queuedInput.cancelEdit")}
              disabled={busy}
              onClick={() =>
                void act(() => updateNativeQueuedInput(sessionId, item.id, null, false))
              }
            >
              <Undo2 className="size-3.5" />
            </Button>
          </>
        ) : (
          <Button
            size="icon"
            variant="ghost"
            className="size-7"
            title={t("queuedInput.edit")}
            aria-label={t("queuedInput.edit")}
            disabled={busy}
            onClick={() => {
              setDraft(item.text);
              void act(() => updateNativeQueuedInput(sessionId, item.id, null, true));
            }}
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Pencil className="size-3.5" />}
          </Button>
        )}
        <Button
          size="icon"
          variant="ghost"
          className="size-7"
          title={t("queuedInput.remove")}
          aria-label={t("queuedInput.remove")}
          disabled={busy}
          onClick={() => void act(() => removeNativeQueuedInput(sessionId, item.id))}
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>
    </li>
  );
}

export function QueuedInputs({ sessionId }: { sessionId: string }) {
  const { t } = useTranslation("sessions");
  const live = useSessionStore((state) => state.liveBySession[sessionId]);
  const queue = useSessionStore((state) => state.inputQueueBySession[sessionId]);
  const queueId = live?.input_queue_id;
  const isLive = Boolean(live);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!isLive) return;
    let cancelled = false;
    setError(null);
    void listNativeQueuedInputs(sessionId)
      .then((snapshot) => {
        if (!cancelled) useSessionStore.getState().onInputQueue(snapshot);
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, queueId, isLive]);
  if (!live || (!queue?.items.length && !error)) return null;
  return (
    <section
      aria-label={t("queuedInput.title")}
      className="mb-2 border-b border-border/50 px-2 pb-1"
    >
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>{t("queuedInput.title")}</span>
        <span className="tabular-nums">{queue?.items.length ?? 0}</span>
      </div>
      <ol className="max-h-60 divide-y divide-border/50 overflow-y-auto">
        {queue?.items.map((item, index) => (
          <QueuedInputRow
            key={`${sessionId}:${item.id}`}
            sessionId={sessionId}
            item={item}
            index={index}
          />
        ))}
      </ol>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
