import "./types"; // Import for global type declarations
import type { Manifest } from "@tonk/core/slim";
import { Bundle, TonkCore } from "@tonk/core/slim";
import {
  clearAllCache,
  persistBundleBytes,
  persistNamespace,
  persistWsUrl,
  restoreAppSlug,
  restoreBundleBytes,
  restoreNamespace,
  restoreWsUrl,
} from "./cache";
import { startHealthMonitoring } from "./connection";
import { getState, transitionTo } from "./state";
import { logger } from "./utils/logging";
import { getWsUrlFromManifest } from "./utils/network";
import { ensureWasmInitialized } from "./wasm-init";

declare const TONK_SERVER_URL: string;

// Helper to wait for PathIndex to sync from remote
export async function waitForPathIndexSync(tonk: TonkCore): Promise<void> {
  logger.debug("Waiting for PathIndex to sync from remote...");

  return new Promise((resolve) => {
    let syncDetected = false;
    let watcherHandle: { stop: () => void } | null = null;

    // Covers case where we're first client or PathIndex is already current
    const timeout = setTimeout(() => {
      if (watcherHandle) {
        try {
          watcherHandle.stop();
        } catch (error) {
          logger.warn("Error stopping PathIndex watcher on timeout", {
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
      if (!syncDetected) {
        logger.debug("No PathIndex changes detected after 1s - proceeding");
        resolve();
      }
    }, 1000);

    // Watch root directory (which watches the PathIndex document)
    tonk
      .watchDirectory("/", (documentData: unknown) => {
        if (!syncDetected) {
          syncDetected = true;
          logger.debug("PathIndex synced from remote", {
            documentType: (documentData as { type?: string }).type,
          });
          clearTimeout(timeout);

          // Stop watching
          if (watcherHandle) {
            try {
              watcherHandle.stop();
            } catch (error) {
              logger.warn("Error stopping PathIndex watcher after sync", {
                error: error instanceof Error ? error.message : String(error),
              });
            }
          }

          resolve();
        }
      })
      .then((watcher: { stop: () => void; document_id?: string }) => {
        watcherHandle = watcher;
        logger.debug("PathIndex watcher established", {
          watcherId: watcher?.document_id || "unknown",
        });
      })
      .catch((error: Error) => {
        logger.error("Failed to establish PathIndex watcher", {
          error: error.message,
        });
        // Still resolve to allow initialization to proceed
        clearTimeout(timeout);
        resolve();
      });
  });
}

// Auto-initialize Tonk from cache if available
export async function autoInitializeFromCache(): Promise<void> {
  try {
    // Try to restore appSlug, bundle, wsUrl, and namespace from cache
    const restoredSlug = await restoreAppSlug();
    const bundleBytes = await restoreBundleBytes();
    const restoredWsUrl = await restoreWsUrl();
    const restoredNamespace = await restoreNamespace();

    if (!restoredSlug || !bundleBytes) {
      logger.debug(
        "No cached state found, waiting for initialization message",
        {
          hasSlug: !!restoredSlug,
          hasBundle: !!bundleBytes,
        },
      );
      return;
    }

    // Found cached state - auto-initialize
    logger.info("Auto-initializing from cache", {
      slug: restoredSlug,
      bundleSize: bundleBytes.length,
      namespace: restoredNamespace,
    });

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    // Create bundle and manifest
    const bundle = await Bundle.fromBytes(bundleBytes);
    const manifest = await bundle.getManifest();
    logger.debug("Bundle and manifest restored from cache", {
      rootId: manifest.rootId,
    });

    // Create TonkCore instance with restored namespace for IndexedDB isolation
    const storageConfig = restoredNamespace
      ? { type: "indexeddb" as const, namespace: restoredNamespace }
      : { type: "indexeddb" as const };
    const tonk = await TonkCore.fromBytes(bundleBytes, {
      storage: storageConfig,
    });
    logger.debug("TonkCore created from cached bundle", {
      namespace: restoredNamespace,
    });

    // Connect to websocket
    // Use restored WS URL if available, otherwise check manifest, then fallback to build-time config
    const wsUrl = restoredWsUrl || getWsUrlFromManifest(manifest);

    // Ensure we persist the URL if we fell back to default
    if (!restoredWsUrl && wsUrl) {
      await persistWsUrl(wsUrl);
    }

    logger.debug("Connecting to websocket...", {
      wsUrl,
      localRootId: manifest.rootId,
    });
    await tonk.connectWebsocket(wsUrl);
    logger.debug("Websocket connected");

    // Wait for PathIndex to sync from remote
    await waitForPathIndexSync(tonk);
    logger.info("Auto-initialization complete");

    // Transition to active state
    transitionTo({
      status: "active",
      bundleId: manifest.rootId,
      ...(restoredNamespace && { launcherBundleId: restoredNamespace }),
      tonk,
      manifest,
      appSlug: restoredSlug,
      wsUrl,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    startHealthMonitoring();
  } catch (error) {
    logger.error("Auto-initialization failed", {
      error: error instanceof Error ? error.message : String(error),
    });

    // Reset to idle state
    transitionTo({ status: "idle" });

    // Clear cache
    await clearAllCache();

    // Notify clients that manual initialization is needed
    const swSelf = self as unknown as ServiceWorkerGlobalScope;
    const allClients = await swSelf.clients.matchAll();

    allClients.forEach((client: Client) => {
      client.postMessage({
        type: "needsReinit",
        appSlug: null,
        reason: "Auto-initialization failed",
      });
    });
  }
}

// Load a new bundle
export async function loadBundle(
  bundleBytes: Uint8Array,
  serverUrl: string,
  _messageId?: string,
  cachedManifest?: Manifest,
  launcherBundleId?: string,
): Promise<{ success: boolean; skipped?: boolean; error?: string }> {
  logger.debug("Loading new bundle", {
    byteLength: bundleBytes.length,
    serverUrl,
    hasCachedManifest: !!cachedManifest,
    launcherBundleId,
  });

  try {
    const state = getState();

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    // Get manifest - use cached version if available to skip redundant Bundle.fromBytes
    let manifest: Manifest;

    if (cachedManifest) {
      logger.info("Using cached manifest, skipping Bundle.fromBytes", {
        rootId: cachedManifest.rootId,
      });
      manifest = cachedManifest;
    } else {
      logger.debug("No cached manifest, parsing bundle", {
        byteLength: bundleBytes.length,
      });
      const bundle = await Bundle.fromBytes(bundleBytes);
      manifest = await bundle.getManifest();
      bundle.free();
      logger.debug("Bundle manifest extracted", { rootId: manifest.rootId });
    }

    // Check if we already have this bundle loaded by comparing launcherBundleId
    // We use launcherBundleId (IndexedDB key) instead of rootId (manifest hash) because
    // multiple bundles can share the same rootId if they have the same code but different data
    if (
      state.status === "active" &&
      launcherBundleId &&
      state.launcherBundleId === launcherBundleId
    ) {
      logger.debug(
        "Bundle already loaded with same launcherBundleId, skipping reload",
        {
          launcherBundleId,
          rootId: manifest.rootId,
        },
      );
      return { success: true, skipped: true };
    }

    // Create new TonkCore instance with namespace for IndexedDB isolation
    // Using launcherBundleId ensures each tonk gets its own separate database
    logger.debug("Creating new TonkCore from bundle bytes", {
      launcherBundleId,
    });
    const loadStorageConfig = launcherBundleId
      ? { type: "indexeddb" as const, namespace: launcherBundleId }
      : { type: "indexeddb" as const };
    const newTonk = await TonkCore.fromBytes(bundleBytes, {
      storage: loadStorageConfig,
    });
    logger.debug("New TonkCore created successfully", {
      rootId: manifest.rootId,
      namespace: launcherBundleId,
    });

    // Get the websocket URL - check manifest first, then URL params, then server URL
    let wsUrl = getWsUrlFromManifest(manifest, serverUrl);

    // URL params can still override if present
    const urlParams = new URLSearchParams(self.location.search);
    const bundleParam = urlParams.get("bundle");

    if (bundleParam) {
      try {
        const decoded = atob(bundleParam);
        const bundleConfig = JSON.parse(decoded);
        if (bundleConfig.wsUrl) {
          wsUrl = bundleConfig.wsUrl;
        }
      } catch (error) {
        logger.warn("Could not parse bundle config for wsUrl", {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    logger.debug("Determined websocket URL", {
      wsUrl,
      serverUrl,
    });

    // Connect to websocket
    await persistWsUrl(wsUrl);

    logger.debug("Connecting new tonk to websocket", {
      wsUrl,
      localRootId: manifest.rootId,
    });

    if (wsUrl) {
      await newTonk.connectWebsocket(wsUrl);
      logger.debug("Websocket connection established");

      // Wait for PathIndex to sync from remote
      await waitForPathIndexSync(newTonk);
      logger.debug("PathIndex sync complete after loadBundle");
    }

    // Get current app slug if we have one, or use a default
    const currentState = getState();
    const appSlug =
      currentState.status === "active"
        ? currentState.appSlug
        : manifest.entrypoints?.[0] || "app";

    // Transition to active state (this will cleanup old state)
    transitionTo({
      status: "active",
      bundleId: manifest.rootId,
      ...(launcherBundleId && { launcherBundleId }),
      tonk: newTonk,
      manifest,
      appSlug,
      wsUrl,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    startHealthMonitoring();

    // Persist bundle bytes and namespace to survive service worker restarts
    await persistBundleBytes(bundleBytes);
    if (launcherBundleId) {
      await persistNamespace(launcherBundleId);
    }
    logger.debug("Bundle bytes and namespace persisted to cache", {
      namespace: launcherBundleId,
    });

    logger.info("Bundle loaded successfully", { rootId: manifest.rootId });
    return { success: true };
  } catch (error) {
    logger.error("Failed to load bundle", {
      error: error instanceof Error ? error.message : String(error),
    });

    transitionTo({
      status: "error",
      error: error instanceof Error ? error : new Error(String(error)),
    });

    return {
      success: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}
