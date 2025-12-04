import {
  TonkCore,
  Manifest,
  Bundle,
  initializeTonk,
} from '@tonk/core/slim';
import { log } from './logging';
import { SERVER_URL, PATH_INDEX_SYNC_TIMEOUT } from './constants';
import {
  getTonkState,
  setTonkState,
  setAppSlug,
  setWsUrl,
  getAppSlug,
} from './state';
import {
  persistAppSlug,
  restoreAppSlug,
  persistBundleBytes,
  restoreBundleBytes,
} from './persistence';
import { startHealthMonitoring } from './websocket-manager';
import { postResponse } from './message-utils';
import type { TonkState } from './types';

// Configuration for TonkCore initialization
export interface InitConfig {
  bundleBytes: Uint8Array;
  serverUrl?: string;
  wsUrl?: string;
}

// Helper to wait for PathIndex to sync from remote
export async function waitForPathIndexSync(tonk: TonkCore): Promise<void> {
  log('info', 'Waiting for PathIndex to sync from remote...');

  return new Promise(resolve => {
    let syncDetected = false;
    let watcherHandle: { stop: () => void } | null = null;

    // Covers case where we're first client or PathIndex is already current
    const timeout = setTimeout(() => {
      if (watcherHandle) {
        try {
          watcherHandle.stop();
        } catch (error) {
          log('warn', 'Error stopping PathIndex watcher on timeout', {
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
      if (!syncDetected) {
        log(
          'info',
          'No PathIndex changes detected after timeout - proceeding (first client or already synced)'
        );
        resolve();
      }
    }, PATH_INDEX_SYNC_TIMEOUT);

    // Watch root directory (which watches the PathIndex document)
    tonk
      .watchDirectory('/', (documentData: unknown) => {
        if (!syncDetected) {
          syncDetected = true;
          log('info', 'PathIndex synced from remote - changes detected!', {
            documentType: (documentData as { type?: string }).type,
          });
          clearTimeout(timeout);

          // Stop watching
          if (watcherHandle) {
            try {
              watcherHandle.stop();
            } catch (error) {
              log('warn', 'Error stopping PathIndex watcher after sync', {
                error: error instanceof Error ? error.message : String(error),
              });
            }
          }

          resolve();
        }
      })
      .then((watcher: { stop: () => void; document_id?: string }) => {
        watcherHandle = watcher;
        log('info', 'PathIndex watcher established', {
          watcherId: watcher?.document_id || 'unknown',
        });
      })
      .catch((error: Error) => {
        log('error', 'Failed to establish PathIndex watcher', {
          error: error.message,
        });
        // Still resolve to allow initialization to proceed
        clearTimeout(timeout);
        resolve();
      });
  });
}

// Unified TonkCore initialization function
// This replaces the 4 duplicated init paths in the original service worker
export async function initializeTonkCore(config: InitConfig): Promise<{
  tonk: TonkCore;
  manifest: Manifest;
}> {
  const { bundleBytes, serverUrl = SERVER_URL, wsUrl: wsUrlInit } = config;
  const currentState = getTonkState();

  // Initialize WASM if not already done
  if (currentState.status === 'uninitialized' || currentState.status === 'failed') {
    log('info', 'Initializing WASM');
    const wasmUrl = `${serverUrl}/tonk_core_bg.wasm`;
    const cacheBustUrl = `${wasmUrl}?t=${Date.now()}`;
    log('info', 'Fetching WASM from', { cacheBustUrl });
    await initializeTonk({ wasmPath: cacheBustUrl });
    log('info', 'WASM initialization completed');
  }

  // Create bundle and manifest
  log('info', 'Creating bundle from bytes', { byteLength: bundleBytes.length });
  const bundle = await Bundle.fromBytes(bundleBytes);
  const manifest = await bundle.getManifest();
  log('info', 'Bundle and manifest created successfully', {
    rootId: manifest.rootId,
  });

  // Create TonkCore instance
  log('info', 'Creating TonkCore from bundle bytes');
  const tonk = await TonkCore.fromBytes(bundleBytes, {
    storage: { type: 'indexeddb' },
  });
  log('info', 'TonkCore created successfully');

  // Determine WebSocket URL
  const wsUrlLocal = wsUrlInit || serverUrl.replace(/^http/, 'ws');

  // Connect to WebSocket
  setWsUrl(wsUrlLocal);
  log('info', 'Connecting to WebSocket', {
    wsUrl: wsUrlLocal,
    localRootId: manifest.rootId,
  });

  await tonk.connectWebsocket(wsUrlLocal);
  log('info', 'WebSocket connection established');
  startHealthMonitoring();

  // Wait for PathIndex to sync from remote
  await waitForPathIndexSync(tonk);
  log('info', 'PathIndex sync complete');

  // Update state
  const newState: TonkState = { status: 'ready', tonk, manifest };
  setTonkState(newState);
  log('info', 'Tonk state updated to ready');

  // Persist bundle bytes for restart recovery
  await persistBundleBytes(bundleBytes);
  log('info', 'Bundle bytes persisted');

  return { tonk, manifest };
}

// Auto-initialize from cached state on service worker startup
export async function autoInitializeFromCache(): Promise<void> {
  try {
    // Try to restore appSlug and bundle from cache
    const restoredSlug = await restoreAppSlug();
    const bundleBytes = await restoreBundleBytes();

    if (!restoredSlug || !bundleBytes) {
      log('info', 'No cached state found, waiting for initialization message', {
        hasSlug: !!restoredSlug,
        hasBundle: !!bundleBytes,
      });
      console.log('🔄 SERVICE WORKER: No cached state - waiting for bundle');
      return;
    }

    // Found cached state - auto-initialize
    setAppSlug(restoredSlug);
    log('info', 'Found cached state, auto-initializing', {
      slug: restoredSlug,
      bundleSize: bundleBytes.length,
    });
    console.log('🔄 SERVICE WORKER: Auto-initializing from cache...', restoredSlug);

    // Use unified initialization
    await initializeTonkCore({
      bundleBytes,
      serverUrl: SERVER_URL,
    });

    console.log('✅ SERVICE WORKER: Auto-initialized successfully');
  } catch (error) {
    log('error', 'Auto-initialization failed', {
      error: error instanceof Error ? error.message : String(error),
    });
    console.error('❌ SERVICE WORKER: Auto-initialization failed:', error);

    // Reset to uninitialized state
    setTonkState({ status: 'uninitialized' });
    setAppSlug(null);

    // Notify clients that manual initialization is needed
    const allClients = await (self as unknown as ServiceWorkerGlobalScope).clients.matchAll();

    allClients.forEach(client => {
      client.postMessage({
        type: 'needsReinit',
        appSlug: null,
        reason: 'Auto-initialization failed',
      });
    });
  }
}

// Initialize from URL (fetch manifest from remote)
export async function initializeFromUrl(config: {
  manifestUrl?: string;
  wasmUrl?: string;
  wsUrl?: string;
}): Promise<{ tonk: TonkCore; manifest: Manifest }> {
  const manifestUrl = config.manifestUrl || `${SERVER_URL}/.manifest.tonk`;

  log('info', 'Fetching manifest from URL', { manifestUrl });
  const manifestResponse = await fetch(manifestUrl);
  const bundleBytes = new Uint8Array(await manifestResponse.arrayBuffer());
  log('info', 'Manifest bytes loaded', { byteLength: bundleBytes.length });

  return initializeTonkCore({
    bundleBytes,
    serverUrl: SERVER_URL,
    wsUrl: config.wsUrl,
  });
}
