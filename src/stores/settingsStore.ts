import { create } from "zustand";

import { getNativeSettings, getNetworkSettings, getQuickPrompts } from "@/lib/backend";
import type { NativeSettings, NetworkSettings, QuickPrompt } from "@/lib/types";

interface SettingsState {
  native: NativeSettings | null;
  network: NetworkSettings | null;
  quickPrompts: QuickPrompt[];
  load: () => Promise<void>;
  setNative: (native: NativeSettings) => void;
  setNetwork: (network: NetworkSettings) => void;
  setQuickPrompts: (prompts: QuickPrompt[]) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  native: null,
  network: null,
  quickPrompts: [],
  load: async () => {
    const [native, network, quickPrompts] = await Promise.all([
      getNativeSettings(),
      getNetworkSettings(),
      getQuickPrompts(),
    ]);
    set({ native, network, quickPrompts });
  },
  setNative: (native) => set({ native }),
  setNetwork: (network) => set({ network }),
  setQuickPrompts: (quickPrompts) => set({ quickPrompts }),
}));
