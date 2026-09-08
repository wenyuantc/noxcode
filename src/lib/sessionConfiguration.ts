import { finishNativeInput, updateNativeSessionConfiguration } from "@/lib/backend";
import {
  composerThinkingEnabled,
  composerThinkingLevels,
  resolveComposerThinkingLevel,
} from "@/lib/modelCatalog";
import { mergeSessionRuntime, resolveSessionSelection } from "@/lib/sessionModel";
import type {
  NativeSessionConfigurationEvent,
  NativeSessionRuntime,
  PendingSessionConfiguration,
} from "@/lib/types";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useUiStore } from "@/stores/uiStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";

export const SESSION_CONFIGURATION_SUPERSEDED = "已被更新的模型选择替换";

export function isModelRuntimeChange(changes: Partial<NativeSessionRuntime>): boolean {
  return changes.ai_channel_id !== undefined || changes.model !== undefined;
}

export function isConfigurationSuperseded(reason: unknown): boolean {
  return String(reason).includes(SESSION_CONFIGURATION_SUPERSEDED);
}

export async function finishIdleSession(sessionId: string): Promise<void> {
  const state = useSessionStore.getState();
  const live = state.liveBySession[sessionId];
  if (!live) return;
  if (state.turnState[sessionId] !== "waiting_input")
    throw new Error("请等待当前任务完成后再修改配置或结束会话");
  await finishNativeInput(sessionId);
  useSessionStore.getState().onExit({ ...live, code: 0 });
}

function fallbackPlanMode(sessionId: string): boolean {
  const state = useSessionStore.getState();
  return Object.prototype.hasOwnProperty.call(state.planModeBySession, sessionId)
    ? state.planModeBySession[sessionId] === true
    : false;
}

function applyLocalConfiguration(sessionId: string, changes: Partial<NativeSessionRuntime>): void {
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
      planMode: fallbackPlanMode(sessionId),
    }),
  );
}

function nextModelRuntime(
  sessionId: string,
  changes: Partial<NativeSessionRuntime>,
): NativeSessionRuntime {
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
  const merged = mergeSessionRuntime(state.configurationBySession[sessionId], changes, {
    channelId: selection.channelId,
    modelId: selection.modelId,
    permissionMode: useSettingsStore.getState().native?.permission_mode ?? "default",
    planMode: fallbackPlanMode(sessionId),
  });
  const channel = channels.channels.find((item) => item.id === merged.ai_channel_id);
  const model = channel?.models.find((item) => item.id === merged.model);
  const levels = composerThinkingLevels(model);
  return {
    ...merged,
    reasoning_effort: composerThinkingEnabled(model)
      ? resolveComposerThinkingLevel(
          levels,
          changes.reasoning_effort ?? merged.reasoning_effort,
          model?.thinking_level,
        )
      : null,
  };
}

async function changeSessionModel(
  sessionId: string,
  changes: Partial<NativeSessionRuntime>,
): Promise<NativeSessionConfigurationEvent | void> {
  const runtime = nextModelRuntime(sessionId, changes);
  const live = useSessionStore.getState().liveBySession[sessionId];
  if (!live) {
    applyLocalConfiguration(sessionId, {
      ai_channel_id: runtime.ai_channel_id,
      model: runtime.model,
      reasoning_effort: runtime.reasoning_effort,
    });
    useChannelStore.getState().setSelection(runtime.ai_channel_id, runtime.model);
    if (runtime.reasoning_effort) {
      useUiStore.getState().setComposerThinkingLevel(runtime.reasoning_effort);
    }
    return;
  }
  const request_id = crypto.randomUUID();
  const pending: PendingSessionConfiguration = {
    request_id,
    ai_channel_id: runtime.ai_channel_id,
    model: runtime.model,
  };
  useSessionStore.getState().setPendingConfiguration(sessionId, pending);
  try {
    const result = await updateNativeSessionConfiguration({
      session_record_id: sessionId,
      ai_channel_id: runtime.ai_channel_id,
      model: runtime.model,
      reasoning_effort: runtime.reasoning_effort,
      request_id,
    });
    useSessionStore.getState().onConfiguration(result);
    if (result.runtime && useSessionStore.getState().selectedSessionId === sessionId) {
      useChannelStore.getState().setSelection(result.runtime.ai_channel_id, result.runtime.model);
      if (result.runtime.reasoning_effort) {
        useUiStore.getState().setComposerThinkingLevel(result.runtime.reasoning_effort);
      }
    }
    return result;
  } catch (reason) {
    const current = useSessionStore.getState().pendingConfigurationBySession[sessionId];
    if (current?.request_id === request_id) {
      useSessionStore.getState().clearPendingConfiguration(sessionId);
    }
    if (isConfigurationSuperseded(reason)) return;
    throw reason;
  }
}

export async function changeSessionConfiguration(
  sessionId: string | null,
  changes: Partial<NativeSessionRuntime>,
): Promise<NativeSessionConfigurationEvent | void> {
  if (!sessionId) {
    if (changes.ai_channel_id && changes.model) {
      useChannelStore.getState().setSelection(changes.ai_channel_id, changes.model);
    }
    return;
  }
  if (isModelRuntimeChange(changes)) {
    return changeSessionModel(sessionId, changes);
  }
  await finishIdleSession(sessionId);
  applyLocalConfiguration(sessionId, changes);
}
