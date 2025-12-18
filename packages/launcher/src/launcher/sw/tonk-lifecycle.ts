import './types'; // Import for global type declarations
import type { Manifest } from '@tonk/core/slim';
import { Bundle, TonkCore } from '@tonk/core/slim';
import {
  clearAllCache,
  persistBundleBytes,
  persistLastActiveBundleId,
  persistNamespace,
  persistWsUrl,
  restoreAppSlug,
  restoreBundleBytes,
  restoreLastActiveBundleId,
  restoreNamespace,
  restoreWsUrl,
} from './cache';
import { startHealthMonitoring } from './connection';
import {
  getBundleState,
  getLastActiveBundleId,
  removeBundleState,
  setBundleState,
  setLastActiveBundleId,
} from './state';
import { logger } from './utils/logging';
import { getWsUrlFromManifest } from './utils/network';
import { ensureWasmInitialized } from './wasm-init';

// Helper to wait for PathIndex to sync from remote
export async function waitForPathIndexSync(tonk: TonkCore): Promise<void> {
  logger.debug('Waiting for PathIndex to sync from remote...');

  return new Promise(resolve => {
    let syncDetected = false;
    let watcherHandle: { stop: () => void } | null = null;

    // Covers case where we're first client or PathIndex is already current
    const timeout = setTimeout(() => {
      if (watcherHandle) {
        try {
          watcherHandle.stop();
        } catch (error) {
          logger.warn('Error stopping PathIndex watcher on timeout', {
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
      if (!syncDetected) {
        logger.debug('No PathIndex changes detected after 1s - proceeding');
        resolve();
      }
    }, 1000);

    // Watch root directory (which watches the PathIndex document)
    tonk
      .watchDirectory('/', (documentData: unknown) => {
        if (!syncDetected) {
          syncDetected = true;
          logger.debug('PathIndex synced from remote', {
            documentType: (documentData as { type?: string }).type,
          });
          clearTimeout(timeout);

          // Stop watching
          if (watcherHandle) {
            try {
              watcherHandle.stop();
            } catch (error) {
              logger.warn('Error stopping PathIndex watcher after sync', {
                error: error instanceof Error ? error.message : String(error),
              });
            }
          }

          resolve();
        }
      })
      .then((watcher: { stop: () => void; document_id?: string }) => {
        watcherHandle = watcher;
        logger.debug('PathIndex watcher established', {
          watcherId: watcher?.document_id || 'unknown',
        });
      })
      .catch((error: Error) => {
        logger.error('Failed to establish PathIndex watcher', {
          error: error.message,
        });
        // Still resolve to allow initialization to proceed
        clearTimeout(timeout);
        resolve();
      });
  });
}

// Auto-initialize Tonk from cache if available (only restores last active bundle)
export async function autoInitializeFromCache(): Promise<void> {
  try {
    // Try to restore last active bundle ID first
    const restoredBundleId = await restoreLastActiveBundleId();
    const restoredSlug = await restoreAppSlug();
    const bundleBytes = await restoreBundleBytes();
    const restoredWsUrl = await restoreWsUrl();
    const restoredNamespace = await restoreNamespace();

    if (!restoredSlug || !bundleBytes) {
      logger.debug(
        'No cached state found, waiting for initialization message',
        {
          hasSlug: !!restoredSlug,
          hasBundle: !!bundleBytes,
        }
      );
      return;
    }

    // Use restored bundle ID or namespace as the launcher bundle ID
    const launcherBundleId = restoredBundleId || restoredNamespace;
    if (!launcherBundleId) {
      logger.debug(
        'No launcher bundle ID found in cache, waiting for initialization message'
      );
      return;
    }

    // Found cached state - auto-initialize
    logger.info('Auto-initializing from cache', {
      slug: restoredSlug,
      bundleSize: bundleBytes.length,
      launcherBundleId,
    });

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    // Create bundle and manifest
    const bundle = await Bundle.fromBytes(bundleBytes);
    const manifest = await bundle.getManifest();
    logger.debug('Bundle and manifest restored from cache', {
      rootId: manifest.rootId,
    });

    // Create TonkCore instance with namespace for IndexedDB isolation
    const storageConfig = {
      type: 'indexeddb' as const,
      namespace: launcherBundleId,
    };
    const tonk = await TonkCore.fromBytes(bundleBytes, {
      storage: storageConfig,
    });
    logger.debug('TonkCore created from cached bundle', {
      namespace: launcherBundleId,
    });

    // Connect to websocket
    const wsUrl = restoredWsUrl || getWsUrlFromManifest(manifest);

    // Ensure we persist the URL if we fell back to default
    if (!restoredWsUrl && wsUrl) {
      await persistWsUrl(wsUrl);
    }

    logger.debug('Connecting to websocket...', {
      wsUrl,
      localRootId: manifest.rootId,
    });
    await tonk.connectWebsocket(wsUrl);
    logger.debug('Websocket connected');

    // Wait for PathIndex to sync from remote
    await waitForPathIndexSync(tonk);
    logger.info('Auto-initialization complete');

    // Set bundle state in map
    setBundleState(launcherBundleId, {
      status: 'active',
      bundleId: manifest.rootId,
      launcherBundleId,
      tonk,
      manifest,
      appSlug: restoredSlug,
      wsUrl,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    // Set as last active
    setLastActiveBundleId(launcherBundleId);

    startHealthMonitoring(launcherBundleId);
  } catch (error) {
    logger.error('Auto-initialization failed', {
      error: error instanceof Error ? error.message : String(error),
    });

    // Clear cache
    await clearAllCache();

    // Notify clients that manual initialization is needed
    const swSelf = self as unknown as ServiceWorkerGlobalScope;
    const allClients = await swSelf.clients.matchAll();

    allClients.forEach((client: Client) => {
      client.postMessage({
        type: 'needsReinit',
        appSlug: null,
        reason: 'Auto-initialization failed',
      });
    });
  }
}

// Load a new bundle (or return existing if already loaded)
export async function loadBundle(
  bundleBytes: Uint8Array,
  serverUrl: string,
  launcherBundleId: string,
  _messageId?: string,
  cachedManifest?: Manifest
): Promise<{ success: boolean; skipped?: boolean; error?: string }> {
  logger.debug('Loading bundle', {
    byteLength: bundleBytes.length,
    serverUrl,
    hasCachedManifest: !!cachedManifest,
    launcherBundleId,
  });

  try {
    // Check if this bundle is already loaded
    const existingState = getBundleState(launcherBundleId);
    if (existingState?.status === 'active') {
      logger.debug('Bundle already loaded, skipping reload', {
        launcherBundleId,
        bundleId: existingState.bundleId,
      });
      // Update last active
      setLastActiveBundleId(launcherBundleId);
      await persistLastActiveBundleId(launcherBundleId);
      return { success: true, skipped: true };
    }

    // If bundle is currently loading, wait for it
    if (existingState?.status === 'loading') {
      logger.debug('Bundle is currently loading, waiting...', {
        launcherBundleId,
      });
      await existingState.promise;
      setLastActiveBundleId(launcherBundleId);
      await persistLastActiveBundleId(launcherBundleId);
      return { success: true, skipped: true };
    }

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    // Get manifest - use cached version if available to skip redundant Bundle.fromBytes
    let manifest: Manifest;

    if (cachedManifest) {
      logger.info('Using cached manifest, skipping Bundle.fromBytes', {
        rootId: cachedManifest.rootId,
      });
      manifest = cachedManifest;
    } else {
      logger.debug('No cached manifest, parsing bundle', {
        byteLength: bundleBytes.length,
      });
      const bundle = await Bundle.fromBytes(bundleBytes);
      manifest = await bundle.getManifest();
      bundle.free();
      logger.debug('Bundle manifest extracted', { rootId: manifest.rootId });
    }

    // Create loading state with promise
    let resolveLoading: () => void;
    const loadingPromise = new Promise<void>(resolve => {
      resolveLoading = resolve;
    });

    setBundleState(launcherBundleId, {
      status: 'loading',
      launcherBundleId,
      bundleId: manifest.rootId,
      promise: loadingPromise,
    });

    // Create new TonkCore instance with namespace for IndexedDB isolation
    logger.debug('Creating new TonkCore from bundle bytes', {
      launcherBundleId,
    });
    const loadStorageConfig = {
      type: 'indexeddb' as const,
      namespace: launcherBundleId,
    };
    const newTonk = await TonkCore.fromBytes(bundleBytes, {
      storage: loadStorageConfig,
    });
    logger.debug('New TonkCore created successfully', {
      rootId: manifest.rootId,
      namespace: launcherBundleId,
    });

    // Get the websocket URL - check manifest first, then URL params, then server URL
    let wsUrl = getWsUrlFromManifest(manifest, serverUrl);

    // URL params can still override if present
    const urlParams = new URLSearchParams(self.location.search);
    const bundleParam = urlParams.get('bundle');

    if (bundleParam) {
      try {
        const decoded = atob(bundleParam);
        const bundleConfig = JSON.parse(decoded);
        if (bundleConfig.wsUrl) {
          wsUrl = bundleConfig.wsUrl;
        }
      } catch (error) {
        logger.warn('Could not parse bundle config for wsUrl', {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    logger.debug('Determined websocket URL', {
      wsUrl,
      serverUrl,
    });

    // Connect to websocket
    await persistWsUrl(wsUrl);

    logger.debug('Connecting new tonk to websocket', {
      wsUrl,
      localRootId: manifest.rootId,
    });

    if (wsUrl) {
      await newTonk.connectWebsocket(wsUrl);
      logger.debug('Websocket connection established');

      // Wait for PathIndex to sync from remote
      await waitForPathIndexSync(newTonk);
      logger.debug('PathIndex sync complete after loadBundle');
    }

    // Get app slug from manifest or use default
    const appSlug = manifest.entrypoints?.[0] || 'app';

    // Set active state in map
    setBundleState(launcherBundleId, {
      status: 'active',
      bundleId: manifest.rootId,
      launcherBundleId,
      tonk: newTonk,
      manifest,
      appSlug,
      wsUrl,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    // Set as last active and persist
    setLastActiveBundleId(launcherBundleId);
    await persistLastActiveBundleId(launcherBundleId);

    startHealthMonitoring(launcherBundleId);

    // Persist bundle bytes and namespace to survive service worker restarts
    await persistBundleBytes(bundleBytes);
    await persistNamespace(launcherBundleId);
    logger.debug('Bundle bytes and namespace persisted to cache', {
      namespace: launcherBundleId,
    });

    // Resolve the loading promise
    resolveLoading!();

    logger.info('Bundle loaded successfully', {
      rootId: manifest.rootId,
      launcherBundleId,
    });
    return { success: true };
  } catch (error) {
    logger.error('Failed to load bundle', {
      error: error instanceof Error ? error.message : String(error),
      launcherBundleId,
    });

    setBundleState(launcherBundleId, {
      status: 'error',
      launcherBundleId,
      error: error instanceof Error ? error : new Error(String(error)),
    });

    return {
      success: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

// Unload a bundle (cleanup resources and remove from map)
export async function unloadBundle(launcherBundleId: string): Promise<boolean> {
  logger.debug('Unloading bundle', { launcherBundleId });

  const removed = removeBundleState(launcherBundleId);

  if (removed) {
    logger.info('Bundle unloaded successfully', { launcherBundleId });

    // If this was the last active bundle, clear cache
    if (getLastActiveBundleId() === null) {
      await clearAllCache();
    }
  } else {
    logger.warn('Bundle not found for unload', { launcherBundleId });
  }

  return removed;
}
