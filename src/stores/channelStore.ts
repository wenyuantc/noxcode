import { create } from "zustand";

import { listAiChannels, listModelCatalog } from "@/lib/backend";
import type { AiChannel, ModelCatalogEntry } from "@/lib/types";

const ACTIVE_KEY = "noxcode:active-model";

interface StoredSelection {
  channelId: string;
  modelId: string;
}

interface ChannelState {
  channels: AiChannel[];
  catalog: ModelCatalogEntry[];
  activeChannelId: string | null;
  activeModelId: string | null;
  load: () => Promise<void>;
  setSelection: (channelId: string, modelId: string) => void;
}

function readStored(): StoredSelection | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(ACTIVE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as StoredSelection;
    if (parsed.channelId && parsed.modelId) return parsed;
  } catch {
    return null;
  }
  return null;
}

function persistSelection(channelId: string | null, modelId: string | null) {
  if (typeof window === "undefined") return;
  if (!channelId || !modelId) {
    window.localStorage.removeItem(ACTIVE_KEY);
    return;
  }
  window.localStorage.setItem(ACTIVE_KEY, JSON.stringify({ channelId, modelId }));
}

function resolveSelection(
  channels: AiChannel[],
  preferred: { channelId: string | null; modelId: string | null },
): { channelId: string | null; modelId: string | null } {
  const enabled = channels.filter((channel) => channel.enabled);
  const channel = enabled.find((item) => item.id === preferred.channelId) ?? enabled[0] ?? null;
  if (!channel) return { channelId: null, modelId: null };
  const modelId =
    channel.models.find((item) => item.id === preferred.modelId)?.id ??
    channel.models[0]?.id ??
    null;
  return { channelId: channel.id, modelId };
}

export const useChannelStore = create<ChannelState>((set, get) => ({
  channels: [],
  catalog: [],
  activeChannelId: readStored()?.channelId ?? null,
  activeModelId: readStored()?.modelId ?? null,
  load: async () => {
    const [channels, catalog] = await Promise.all([listAiChannels(), listModelCatalog()]);
    const next = resolveSelection(channels, {
      channelId: get().activeChannelId,
      modelId: get().activeModelId,
    });
    persistSelection(next.channelId, next.modelId);
    set({ channels, catalog, activeChannelId: next.channelId, activeModelId: next.modelId });
  },
  setSelection: (channelId, modelId) => {
    persistSelection(channelId, modelId);
    set({ activeChannelId: channelId, activeModelId: modelId });
  },
}));
