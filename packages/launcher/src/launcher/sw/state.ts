import type { TonkCore, Manifest } from '@tonk/core/slim';
import type { BundleState, ActiveBundleState } from './types';
import { logger } from './utils/logging';

// Global state - single source of truth
let state: BundleState = { status: 'idle' };

// Track the auto-initialization promise so fetch handler can wait for it
let initializationPromise: Promise<void> | null = null;

// Get current state
export function getState(): BundleState {
  return state;
}

// Get initialization promise
export function getInitializationPromise(): Promise<void> | null {
  return initializationPromise;
}

// Set initialization promise
export function setInitializationPromise(promise: Promise<void> | null): void {
  initializationPromise = promise;
}

// Get active bundle state (returns null if not active)
export function getActiveBundle(): ActiveBundleState | null {
  return state.status === 'active' ? state : null;
}

// Get TonkCore instance if available (compatibility helper)
export function getTonk(): { tonk: TonkCore; manifest: Manifest } | null {
  if (state.status === 'active') {
    return { tonk: state.tonk, manifest: state.manifest };
  }
  return null;
}

// Get current app slug
export function getAppSlug(): string | null {
  if (state.status === 'active') {
    return state.appSlug;
  }
  return null;
}

// Cleanup active state resources
function cleanupActiveState(activeState: ActiveBundleState): void {
  logger.debug('Cleaning up active bundle state', {
    bundleId: activeState.bundleId,
    watcherCount: activeState.watchers.size,
    hasHealthCheck: !!activeState.healthCheckInterval,
  });

  // Clear health check interval
  if (activeState.healthCheckInterval) {
    clearInterval(activeState.healthCheckInterval);
  }

  // Stop all watchers
  activeState.watchers.forEach((watcher, id) => {
    try {
      watcher.stop();
      logger.debug('Stopped watcher', { id });
    } catch (error) {
      logger.warn('Error stopping watcher', {
        id,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  });

  // Disconnect WebSocket if available
  // Note: TonkCore may not have disconnectWebsocket method exposed
  // This is a placeholder for when it becomes available
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

// Transition to a new state with automatic cleanup
export function transitionTo(newState: BundleState): void {
  const oldState = state;

  logger.debug('State transition', {
    from: oldState.status,
    to: newState.status,
    bundleId: 'bundleId' in newState ? newState.bundleId : undefined,
  });

  // Cleanup old state if it was active
  if (oldState.status === 'active') {
    cleanupActiveState(oldState);
  }

  state = newState;
}

// Set app slug on active state
export function setAppSlug(slug: string): boolean {
  if (state.status === 'active') {
    state = { ...state, appSlug: slug };
    return true;
  }
  return false;
}

// Update connection health on active state
export function setConnectionHealth(healthy: boolean): void {
  if (state.status === 'active') {
    state = { ...state, connectionHealthy: healthy };
  }
}

// Increment reconnect attempts
export function incrementReconnectAttempts(): number {
  if (state.status === 'active') {
    state = { ...state, reconnectAttempts: state.reconnectAttempts + 1 };
    return state.reconnectAttempts;
  }
  return 0;
}

// Reset reconnect attempts
export function resetReconnectAttempts(): void {
  if (state.status === 'active') {
    state = { ...state, reconnectAttempts: 0 };
  }
}

// Set health check interval
export function setHealthCheckInterval(interval: number | null): void {
  if (state.status === 'active') {
    state = { ...state, healthCheckInterval: interval };
  }
}

// Add a watcher
export function addWatcher(id: string, watcher: { stop: () => void }): void {
  if (state.status === 'active') {
    state.watchers.set(id, watcher);
  }
}

// Remove a watcher
export function removeWatcher(id: string): boolean {
  if (state.status === 'active') {
    const watcher = state.watchers.get(id);
    if (watcher) {
      try {
        watcher.stop();
      } catch (error) {
        logger.warn('Error stopping watcher on remove', {
          id,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      state.watchers.delete(id);
      return true;
    }
  }
  return false;
}

// Get watcher by ID
export function getWatcher(id: string): { stop: () => void } | undefined {
  if (state.status === 'active') {
    return state.watchers.get(id);
  }
  return undefined;
}

// Get all watcher entries (for reestablishing after reconnect)
export function getWatcherEntries(): [string, { stop: () => void }][] {
  if (state.status === 'active') {
    return Array.from(state.watchers.entries());
  }
  return [];
}
