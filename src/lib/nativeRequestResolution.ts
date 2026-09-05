import { useSessionStore } from "@/stores/sessionStore";
import type { NativeRequestResolved } from "@/lib/types";

export async function resolveSessionRequest(
  request: NativeRequestResolved,
  send: () => Promise<void>,
): Promise<void> {
  await send();
  useSessionStore.getState().resolveRequest(request);
}
