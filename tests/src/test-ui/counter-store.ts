import { create } from 'zustand';
import { sync } from './middleware';

interface CounterState {
  counter: number;
  increment: () => void;
}

export const useCounterStore = create<CounterState>()(
  sync(
    set => ({
      counter: 0,
      increment: () => set(state => ({ counter: state.counter + 1 })),
    }),
    { path: '/counter-state.json' }
  )
);
