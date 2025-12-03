import { create } from 'zustand';
import type { PersistStorage } from 'zustand/middleware';
import { persist } from 'zustand/middleware';
import { immer } from 'zustand/middleware/immer';
import { sync } from './middleware';

export type PersistConfig = {
  name: string;
  storage?: PersistStorage<unknown>;
  partialize?: <T>(state: T) => object;
  version?: number;
};

// biome-ignore lint/suspicious/noExplicitAny: Generic default type for flexible state types
export type SyncConfig<T = any> = {
  path: string;
  partialize?: (state: T) => Partial<T>;
};

export const StoreBuilder = <T extends object>(
  initialState: T,
  persistConfig?: PersistConfig,
  syncConfig?: SyncConfig
) => {
  type StoreState = T;

  // Create store with VFS sync, localStorage persist, or no persistence
  const useStore = syncConfig
    ? create<StoreState>()(
        sync(
          immer((_set) => ({
            ...initialState,
            // biome-ignore lint/suspicious/noExplicitAny: Middleware type compatibility requires cast
          })) as any,
          {
            path: syncConfig.path,
            partialize: syncConfig.partialize,
          }
        )
      )
    : persistConfig
      ? create<StoreState>()(
          persist(
            immer((_set) => ({
              ...initialState,
              // biome-ignore lint/suspicious/noExplicitAny: Middleware type compatibility requires cast
            })) as any,
            {
              name: persistConfig.name,
              storage: persistConfig.storage,
              partialize: persistConfig.partialize as (state: StoreState) => object,
              version: persistConfig.version,
            }
          )
        )
      : create<StoreState>()(
          immer((_set) => ({
            ...initialState,
            // biome-ignore lint/suspicious/noExplicitAny: Middleware type compatibility requires cast
          })) as any
        );

  const get = () => useStore.getState();

  // Force cast setState to support Immer producers (void return from immer mutations)
  const set = useStore.setState as (
    // biome-ignore lint/suspicious/noConfusingVoidType: Immer producers return void
    partial: T | Partial<T> | ((state: T) => void | T | Partial<T>),
    replace?: boolean | undefined
  ) => void;

  const subscribe = useStore.subscribe;

  /**
   * Creates a factory function that exposes all store state and methods.
   * Facilitates separating actions from the store state, and using the non-reactive get/set instead of `useStore`, as it needs to be explicitly exposed.
   * @returns {set} The store state setter
   * @returns {useStore} The store state React hook
   * @returns {subscribe} The store subscription function
   * @returns {Object} The store state and methods
   */
  const createFactory = <S>(
    args: S
  ): (() => T & S & { set: typeof set; subscribe: typeof subscribe }) => {
    return () => {
      // biome-ignore lint/correctness/useHookAtTopLevel: Factory returns hook to be called at component top level
      const state = useStore();
      return {
        ...state,
        set,
        subscribe,
        ...args,
      };
    };
  };
  return { get, set, useStore, subscribe, createFactory };
};
