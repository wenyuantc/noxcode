import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getNativeSteerSnapshot } from "@/lib/backend";
import {
  prepareSteerAttempt,
  submitSteerAttempt,
  steerError,
  type SteerAttempt,
} from "@/lib/nativeSteer";
import { useSessionStore } from "@/stores/sessionStore";
import { useSteerStore } from "@/stores/steerStore";

export function useNativeSteer(sessionId: string | null | undefined) {
  const { t } = useTranslation("sessions");
  const instance = useSessionStore((state) =>
    sessionId ? state.liveBySession[sessionId]?.input_queue_id : undefined,
  );
  const snapshot = useSteerStore((state) => (sessionId ? state.snapshots[sessionId] : undefined));
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{
    sessionId: string | null | undefined;
    message: string;
  } | null>(null);
  const error = failure?.sessionId === sessionId ? (failure?.message ?? null) : null;
  const setError = (message: string | null) =>
    setFailure(message === null ? null : { sessionId, message });
  const attempt = useRef<SteerAttempt | null>(null);
  const pending = useRef(false);

  useEffect(() => {
    if (!sessionId) return;
    let cancelled = false;
    void Promise.resolve()
      .then(() => getNativeSteerSnapshot(sessionId))
      .then((value) => {
        if (!cancelled) useSessionStore.getState().onSteerSnapshot(value);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [sessionId, instance]);

  const submit = async (text: string, imagePaths: string[]) => {
    if (pending.current) return false;
    const retryKey = JSON.stringify([sessionId, text, imagePaths]);
    const retry = attempt.current?.key === retryKey ? attempt.current : null;
    const turnId = retry?.turnId ?? snapshot?.turn_id;
    if (!sessionId || !turnId) {
      setError(t("steer.noTurn"));
      return false;
    }
    if (new TextEncoder().encode(text).length > 200 * 1024 || imagePaths.length > 8) {
      setError(t("steer.limit"));
      return false;
    }
    attempt.current = prepareSteerAttempt(attempt.current, sessionId, turnId, text, imagePaths);
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      const receipt = await submitSteerAttempt(
        attempt.current,
        sessionId,
        turnId,
        text,
        imagePaths,
      );
      attempt.current = null;
      if (receipt.status === "rejected" || receipt.status === "cancelled") {
        setError(receipt.error || t(`steer.${receipt.status}`));
        return false;
      }
      // Query also covers delivery before the event listener was mounted.
      void Promise.resolve()
        .then(() => getNativeSteerSnapshot(sessionId))
        .then((value) => useSessionStore.getState().onSteerSnapshot(value))
        .catch(() => undefined);
      return true;
    } catch (reason) {
      const failure = steerError(reason);
      setError(failure.message);
      if (failure.definitive) attempt.current = null;
      // Keep the ID and draft on transport errors: retry may find an accepted receipt.
      return false;
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return {
    snapshot,
    busy,
    error,
    submit,
    canSubmit: Boolean(
      snapshot?.turn_id || (attempt.current && attempt.current.sessionId === sessionId),
    ),
  };
}
