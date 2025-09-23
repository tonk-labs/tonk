import { create } from 'zustand';
import { sync } from '../middleware';

type SimpleStore = {
  count: number;
  increment: () => void;
  decrement: () => void;
  reset: () => void;
};

export const useSimpleStore = create<SimpleStore>()(
  sync(
    set => ({
      count: 0,
      increment: () => set(state => ({ count: state.count + 1 })),
      decrement: () => set(state => ({ count: state.count - 1 })),
      reset: () => set({ count: 0 }),
    }),
    { path: '/simple-store.json' }
  )
);
