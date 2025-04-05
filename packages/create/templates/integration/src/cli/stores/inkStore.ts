import { create } from "zustand";
import { Instance } from "ink";

/**
 * Interface defining the state and actions for the Ink render instance store
 */
interface InkState {
  /** The current Ink render instance */
  instance: Instance | null;
  /** Sets the current Ink render instance */
  setInstance: (instance: Instance) => void;
  /** Clears the current Ink render instance */
  clearInstance: () => void;
}

/**
 * Creates a Zustand store for managing the Ink render instance
 */
export const inkStore = create<InkState>((set) => ({
  instance: null,
  setInstance: (instance) => set({ instance }),
  clearInstance: () => set({ instance: null }),
}));

/**
 * Hook version of the store for use in React components
 *
 * @example
 * const { instance, setInstance } = useInkStore()
 * setInstance(render(<App />))
 */
export const useInkStore = inkStore;

export default useInkStore;
