import { finishNativeInput } from "@/lib/backend";
import type { NativeSessionRuntime } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";

export async function finishIdleSession(sessionId: string): Promise<void> {
  const state = useSessionStore.getState();
  const live = state.liveBySession[sessionId];
  if (!live) return;
  if (state.turnState[sessionId] !== "waiting_input")
    throw new Error("请等待当前任务完成后再修改配置或结束会话");
  await finishNativeInput(sessionId);
  useSessionStore.getState().onExit({ ...live, code: 0 });
}

export async function changeSessionConfiguration(
  sessionId: string | null,
  changes: Partial<NativeSessionRuntime>,
): Promise<void> {
  if (!sessionId) return;
  await finishIdleSession(sessionId);
  const state = useSessionStore.getState();
  const runtime = state.configurationBySession[sessionId];
  if (runtime) state.setConfiguration(sessionId, { ...runtime, ...changes });
}
