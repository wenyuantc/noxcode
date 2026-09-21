import { submitNativeSteer } from "@/lib/backend";

export interface SteerAttempt {
  sessionId: string;
  key: string;
  inputId: string;
  turnId: string;
}

export function prepareSteerAttempt(
  previous: SteerAttempt | null,
  sessionId: string,
  turnId: string,
  text: string,
  imagePaths: string[],
): SteerAttempt {
  const key = JSON.stringify([sessionId, text, imagePaths]);
  return previous?.key === key
    ? previous
    : { sessionId, key, inputId: crypto.randomUUID(), turnId };
}

export function submitSteerAttempt(
  attempt: SteerAttempt,
  sessionId: string,
  turnId: string,
  text: string,
  imagePaths: string[],
) {
  return submitNativeSteer(sessionId, attempt.turnId || turnId, attempt.inputId, text, imagePaths);
}

export function steerError(reason: unknown): { message: string; definitive: boolean } {
  if (typeof reason === "object" && reason !== null && "kind" in reason && "message" in reason) {
    return { message: String(reason.message), definitive: reason.kind === "rejected" };
  }
  return { message: String(reason), definitive: false };
}

export function canClearSteerDraft(
  submitted: { sessionId: string | null; text: string; paths: string[] },
  current: { sessionId: string | null; text: string; paths: string[] },
): boolean {
  return (
    submitted.sessionId === current.sessionId &&
    submitted.text === current.text &&
    JSON.stringify(submitted.paths) === JSON.stringify(current.paths)
  );
}
