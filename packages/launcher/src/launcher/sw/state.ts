import type { Manifest, TonkCore } from '@tonk/core/slim';
import type { ActiveBundleState, BundleState, BundleStateMap, WatcherEntry } from './types';
import { logger } from './utils/logging';

// Global state - Map of bundle states keyed by launcherBundleId
const bundleStates: BundleStateMap = new Map();

// Track the last active bundle ID for backwards compatibility and cache restore
let lastActiveBundleId: string | null = null;

// Track the auto-initialization promise so fetch handler can wait for it
let initializationPromise: Promise<void> | null = null;

// Get initialization promise
export function getInitializationPromise(): Promise<void> | null {
  return initializationPromise;
}

// Set initialization promise
export function setInitializationPromise(promise: Promise<void> | null): void {
  initializationPromise = promise;
}

// Get last active bundle ID
export function getLastActiveBundleId(): string | null {
  return lastActiveBundleId;
}

// Set last active bundle ID
export function setLastActiveBundleId(id: string | null): void {
  lastActiveBundleId = id;
  logger.debug('Last active bundle ID updated', { id });
}

// Get all bundle IDs currently loaded
export function getAllBundleIds(): string[] {
  return Array.from(bundleStates.keys());
}

// Get bundle state by launcherBundleId
export function getBundleState(launcherBundleId: string): BundleState | undefined {
  return bundleStates.get(launcherBundleId);
}

// Set bundle state
export function setBundleState(launcherBundleId: string, state: BundleState): void {
  const oldState = bundleStates.get(launcherBundleId);

  logger.debug('Setting bundle state', {
    launcherBundleId,
    oldStatus: oldState?.status ?? 'none',
    newStatus: state.status,
  });

  // Cleanup old state if it was active
  if (oldState?.status === 'active') {
    cleanupActiveState(oldState);
  }

  bundleStates.set(launcherBundleId, state);
}

// Remove bundle state (for unloading)
export function removeBundleState(launcherBundleId: string): boolean {
  const state = bundleStates.get(launcherBundleId);

  if (!state) {
    logger.debug('Bundle state not found for removal', { launcherBundleId });
    return false;
  }

  logger.debug('Removing bundle state', {
    launcherBundleId,
    status: state.status,
  });

  // Cleanup if active
  if (state.status === 'active') {
    cleanupActiveState(state);
  }

  bundleStates.delete(launcherBundleId);

  // Clear last active if this was it
  if (lastActiveBundleId === launcherBundleId) {
    lastActiveBundleId = null;
  }

  return true;
}

// Get TonkCore for a specific bundle
export function getTonkForBundle(
  launcherBundleId: string
): { tonk: TonkCore; manifest: Manifest } | null {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    return { tonk: state.tonk, manifest: state.manifest };
  }
  return null;
}

// Get active bundle state for a specific bundle
export function getActiveBundleState(launcherBundleId: string): ActiveBundleState | null {
  const state = bundleStates.get(launcherBundleId);
  return state?.status === 'active' ? state : null;
}

// Get active bundle state (uses lastActiveBundleId if no ID provided)
// This provides backwards compatibility for code that doesn't have bundle context
export function getActiveBundle(launcherBundleId?: string): ActiveBundleState | null {
  const id = launcherBundleId ?? lastActiveBundleId;
  if (!id) return null;
  return getActiveBundleState(id);
}

// Get TonkCore instance (uses lastActiveBundleId if no ID provided)
// This provides backwards compatibility
export function getTonk(launcherBundleId?: string): { tonk: TonkCore; manifest: Manifest } | null {
  const id = launcherBundleId ?? lastActiveBundleId;
  if (!id) return null;
  return getTonkForBundle(id);
}

// Get current app slug for a bundle
export function getAppSlug(launcherBundleId?: string): string | null {
  const id = launcherBundleId ?? lastActiveBundleId;
  if (!id) return null;

  const state = bundleStates.get(id);
  if (state?.status === 'active') {
    return state.appSlug;
  }
  return null;
}

// Legacy getState function - returns state for lastActiveBundleId or 'idle'
export function getState(): BundleState | { status: 'idle' } {
  if (!lastActiveBundleId) {
    return { status: 'idle' };
  }
  return bundleStates.get(lastActiveBundleId) ?? { status: 'idle' };
}

// Cleanup active state resources
function cleanupActiveState(activeState: ActiveBundleState): void {
  logger.debug('Cleaning up active bundle state', {
    launcherBundleId: activeState.launcherBundleId,
    watcherCount: activeState.watchers.size,
    hasHealthCheck: !!activeState.healthCheckInterval,
  });

  // Clear health check interval
  if (activeState.healthCheckInterval) {
    clearInterval(activeState.healthCheckInterval);
  }

  // Stop all watchers
  activeState.watchers.forEach((entry, id) => {
    try {
      entry.watcher.stop();
      logger.debug('Stopped watcher', { id });
    } catch (error) {
      logger.warn('Error stopping watcher', {
        id,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  });

  // Disconnect WebSocket if available
  try {
    // @ts-expect-error - disconnectWebsocket may not be in type definitions
    if (typeof activeState.tonk.disconnectWebsocket === 'function') {
      // @ts-expect-error - calling potentially undefined method
      activeState.tonk.disconnectWebsocket();
      logger.debug('Disconnected WebSocket');
    }
  } catch (error) {
    logger.warn('Error disconnecting WebSocket', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Set app slug on a bundle's active state
export function setAppSlug(launcherBundleId: string, slug: string): boolean {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    bundleStates.set(launcherBundleId, { ...state, appSlug: slug });
    return true;
  }
  return false;
}

// Update connection health on a bundle's active state
export function setConnectionHealth(launcherBundleId: string, healthy: boolean): void {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    bundleStates.set(launcherBundleId, {
      ...state,
      connectionHealthy: healthy,
    });
  }
}

// Increment reconnect attempts for a bundle
export function incrementReconnectAttempts(launcherBundleId: string): number {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    const newAttempts = state.reconnectAttempts + 1;
    bundleStates.set(launcherBundleId, {
      ...state,
      reconnectAttempts: newAttempts,
    });
    return newAttempts;
  }
  return 0;
}

// Reset reconnect attempts for a bundle
export function resetReconnectAttempts(launcherBundleId: string): void {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    bundleStates.set(launcherBundleId, { ...state, reconnectAttempts: 0 });
  }
}

// Set health check interval for a bundle
export function setHealthCheckInterval(launcherBundleId: string, interval: number | null): void {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    bundleStates.set(launcherBundleId, {
      ...state,
      healthCheckInterval: interval,
    });
  }
}

// Add a watcher to a bundle
// Add a watcher to a bundle with client ID for routing callbacks
export function addWatcher(
  launcherBundleId: string,
  watcherId: string,
  watcher: { stop: () => void },
  clientId: string
): void {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    state.watchers.set(watcherId, { watcher, clientId });
  }
}

// Remove a watcher from a bundle
export function removeWatcher(launcherBundleId: string, watcherId: string): boolean {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    const entry = state.watchers.get(watcherId);
    if (entry) {
      try {
        entry.watcher.stop();
      } catch (error) {
        logger.warn('Error stopping watcher on remove', {
          watcherId,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      state.watchers.delete(watcherId);
      return true;
    }
  }
  return false;
}

// Get watcher entry by ID from a bundle (includes clientId for routing)
export function getWatcherEntry(
  launcherBundleId: string,
  watcherId: string
): WatcherEntry | undefined {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    return state.watchers.get(watcherId);
  }
  return undefined;
}

// Get client ID for a watcher (for routing async callbacks)
export function getWatcherClientId(
  launcherBundleId: string,
  watcherId: string
): string | undefined {
  const entry = getWatcherEntry(launcherBundleId, watcherId);
  return entry?.clientId;
}

// Get all watcher entries for a bundle (for reestablishing after reconnect)
export function getWatcherEntries(launcherBundleId: string): [string, WatcherEntry][] {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status === 'active') {
    return Array.from(state.watchers.entries());
  }
  return [];
}

// Remove all watchers for a specific client ID (for cleanup when client disconnects)
export function removeWatchersByClientId(launcherBundleId: string, clientId: string): number {
  const state = bundleStates.get(launcherBundleId);
  if (state?.status !== 'active') {
    return 0;
  }

  const watchersToRemove: string[] = [];

  // Find all watchers for this client
  for (const [watcherId, entry] of state.watchers.entries()) {
    if (entry.clientId === clientId) {
      watchersToRemove.push(watcherId);
    }
  }

  // Remove and stop each watcher
  for (const watcherId of watchersToRemove) {
    const entry = state.watchers.get(watcherId);
    if (entry) {
      try {
        entry.watcher.stop();
        logger.debug('Stopped stale watcher for disconnected client', {
          watcherId,
          clientId,
        });
      } catch (error) {
        logger.warn('Error stopping stale watcher', {
          watcherId,
          clientId,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      state.watchers.delete(watcherId);
    }
  }

  if (watchersToRemove.length > 0) {
    logger.info('Cleaned up stale watchers for disconnected client', {
      clientId,
      launcherBundleId,
      count: watchersToRemove.length,
    });
  }

  return watchersToRemove.length;
}
