import { create } from 'zustand';
import type { PersistStorage } from 'zustand/middleware';
import { persist } from 'zustand/middleware';
import { immer } from 'zustand/middleware/immer';
import { sync } from './middleware';

export type VFSConfig = {
	type: 'vfs';
	path: string;
};

export type PersistConfig = {
	type: 'persist';
	name: string;
	storage?: PersistStorage<unknown>;
	partialize?: <T>(state: T) => object;
	version?: number;
};

export type StoreConfig = VFSConfig | PersistConfig;

export const StoreBuilder = <T>(
	initialState: T,
	config: StoreConfig,
) => {
	type StoreState = T;

	// Create store with VFS sync or persistence based on config type
	const useStore =
		config.type === 'vfs'
			? create<StoreState>()(
					sync(
						immer(() => ({
							...initialState,
						})),
						{
							path: config.path,
						}
					)
				)
			: create<StoreState>()(
					persist(
						immer(() => ({
							...initialState,
						})),
						{
							name: config.name,
							storage: config.storage,
							partialize: config.partialize as (state: StoreState) => object,
							version: config.version,
						}
					)
				);

	const get = () => useStore.getState();
	const set = useStore.setState;
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
		args: S,
	): (() => T & S & { set: typeof set; subscribe: typeof subscribe }) => {
		return () => {
			return {
				...get(),
				set,
				subscribe,
				...args,
			};
		};
	};
	return { get, set, useStore, subscribe, createFactory };
};
