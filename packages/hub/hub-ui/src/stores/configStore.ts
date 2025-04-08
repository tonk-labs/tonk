import { create } from "zustand";
import { Config } from "../types";
import { getConfig } from "../ipc/config";

interface ConfigState {
  config: Config | null;
  isLoading: boolean;
  isInitialized: boolean;
  loadConfig: () => Promise<void>;
  setConfig: (config: Config) => void;
}

export const useConfigStore = create<ConfigState>((set) => ({
  config: null,
  isLoading: false,
  isInitialized: false,
  loadConfig: async () => {
    set({ isLoading: true });
    try {
      const config = await getConfig();
      set({ config, isInitialized: !!config?.homePath });
    } catch (error) {
      console.error("Failed to load config:", error);
    } finally {
      set({ isLoading: false });
    }
  },
  setConfig: (config) => set({ config, isInitialized: true }),
}));
