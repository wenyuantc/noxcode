import { create } from "zustand";

import {
  getAiSettings,
  getNativeSettings,
  getNetworkSettings,
  getQuickPrompts,
} from "@/lib/backend";
import type { AiSettings, NativeSettings, NetworkSettings, QuickPrompt } from "@/lib/types";

interface SettingsState {
  native: NativeSettings | null;
  network: NetworkSettings | null;
  ai: AiSettings | null;
  quickPrompts: QuickPrompt[];
  load: () => Promise<void>;
  setNative: (native: NativeSettings) => void;
  setNetwork: (network: NetworkSettings) => void;
  setAi: (ai: AiSettings) => void;
  setQuickPrompts: (prompts: QuickPrompt[]) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  native: null,
  network: null,
  ai: null,
  quickPrompts: [],
  load: async () => {
    const [native, network, ai, quickPrompts] = await Promise.all([
      getNativeSettings(),
      getNetworkSettings(),
      getAiSettings(),
      getQuickPrompts(),
    ]);
    set({ native, network, ai, quickPrompts });
  },
  setNative: (native) => set({ native }),
  setNetwork: (network) => set({ network }),
  setAi: (ai) => set({ ai }),
  setQuickPrompts: (quickPrompts) => set({ quickPrompts }),
}));
