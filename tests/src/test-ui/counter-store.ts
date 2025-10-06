import { create } from 'zustand';
import { sync } from './middleware';

interface CounterState {
  counter: number;
  largeData?: any;
  increment: () => void;
  setData: (data: any) => void;
}

export const useCounterStore = create<CounterState>()(
  sync(
    set => ({
      counter: 0,
      largeData: undefined,
      increment: () => set(state => ({ counter: state.counter + 1 })),
      setData: (data: any) => set({ largeData: data }),
    }),
    { path: '/counter-state.json' }
  )
);
