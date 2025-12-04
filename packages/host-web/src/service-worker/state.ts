import type { TonkCore, Manifest } from '@tonk/core/slim';
import type { TonkState, WatcherHandle } from './types';

// Global state variables
let tonkState: TonkState = { status: 'uninitialized' };
let appSlug: string | null = null;
let wsUrl: string | null = null;
let healthCheckInterval: number | null = null;
let connectionHealthy = true;
let reconnectAttempts = 0;

// Watcher storage
const watchers = new Map<string, WatcherHandle>();

// State getters
export function getTonkState(): TonkState {
  return tonkState;
}

export function getTonk(): { tonk: TonkCore; manifest: Manifest } | null {
  return tonkState.status === 'ready' ? tonkState : null;
}

export function getAppSlug(): string | null {
  return appSlug;
}

export function getWsUrl(): string | null {
  return wsUrl;
}

export function getHealthCheckInterval(): number | null {
  return healthCheckInterval;
}

export function isConnectionHealthy(): boolean {
  return connectionHealthy;
}

export function getReconnectAttempts(): number {
  return reconnectAttempts;
}

export function getWatchers(): Map<string, WatcherHandle> {
  return watchers;
}

// State setters
export function setTonkState(state: TonkState): void {
  tonkState = state;
}

export function setAppSlug(slug: string | null): void {
  appSlug = slug;
}

export function setWsUrl(url: string | null): void {
  wsUrl = url;
}

export function setHealthCheckInterval(interval: number | null): void {
  healthCheckInterval = interval;
}

export function setConnectionHealthy(healthy: boolean): void {
  connectionHealthy = healthy;
}

export function setReconnectAttempts(attempts: number): void {
  reconnectAttempts = attempts;
}

export function incrementReconnectAttempts(): number {
  reconnectAttempts++;
  return reconnectAttempts;
}

export function resetReconnectAttempts(): void {
  reconnectAttempts = 0;
}

// Watcher management
export function addWatcher(id: string, watcher: WatcherHandle): void {
  watchers.set(id, watcher);
}

export function removeWatcher(id: string): WatcherHandle | undefined {
  const watcher = watchers.get(id);
  if (watcher) {
    watchers.delete(id);
  }
  return watcher;
}

export function getWatcher(id: string): WatcherHandle | undefined {
  return watchers.get(id);
}

export function clearHealthCheckInterval(): void {
  if (healthCheckInterval !== null) {
    clearInterval(healthCheckInterval);
    healthCheckInterval = null;
  }
}
