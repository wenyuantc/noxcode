import { finishNativeInput } from "@/lib/backend";
import { mergeSessionRuntime, resolveSessionSelection } from "@/lib/sessionModel";
import type { NativeSessionRuntime } from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

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
  const session = useWorkspaceStore.getState().sessions.find((item) => item.id === sessionId);
  const channels = useChannelStore.getState();
  const selection = resolveSessionSelection({
    sessionId,
    runtime: state.configurationBySession[sessionId],
    session,
    fallbackChannelId: channels.activeChannelId,
    fallbackModelId: channels.activeModelId,
  });
  state.setConfiguration(
    sessionId,
    mergeSessionRuntime(state.configurationBySession[sessionId], changes, {
      channelId: selection.channelId,
      modelId: selection.modelId,
      permissionMode: useSettingsStore.getState().native?.permission_mode ?? "default",
      planMode: Object.prototype.hasOwnProperty.call(state.planModeBySession, sessionId)
        ? state.planModeBySession[sessionId] === true
        : false,
    }),
  );
}
