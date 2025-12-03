import { StoreBuilder } from '../../../lib/storeBuilder';

interface MembersBarState {
  isOpen: boolean;
}

const initialState: MembersBarState = {
  isOpen: true,
};

export const membersBarStore = StoreBuilder(initialState, {
  name: 'members-bar-state',
  version: 1,
  // biome-ignore lint/suspicious/noExplicitAny: Zustand persist storage type mismatch
  storage: localStorage as any,
});

export const useMembersBarStore = membersBarStore.useStore;

const createMembersBarActions = () => {
  return {
    toggle: () => {
      membersBarStore.set((state) => {
        state.isOpen = !state.isOpen;
      });
    },
  };
};

export const useMembersBar = membersBarStore.createFactory(createMembersBarActions());
