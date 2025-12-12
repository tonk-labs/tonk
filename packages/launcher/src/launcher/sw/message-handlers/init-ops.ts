import { Bundle, TonkCore } from "@tonk/core/slim";
import {
  persistAppSlug,
  persistBundleBytes,
  persistLastActiveBundleId,
  persistNamespace,
  persistWsUrl,
} from "../cache";
import { startHealthMonitoring } from "../connection";
import {
  getBundleState,
  getTonkForBundle,
  setAppSlug,
  setBundleState,
  setLastActiveBundleId,
} from "../state";
import { waitForPathIndexSync } from "../tonk-lifecycle";
import { logger } from "../utils/logging";
import { getWsUrlFromManifest } from "../utils/network";
import { postResponse } from "../utils/response";
import { ensureWasmInitialized } from "../wasm-init";

declare const TONK_SERVER_URL: string;

export async function handleInit(message: {
  id?: string;
  manifest: ArrayBuffer;
  wsUrl?: string;
  launcherBundleId: string;
}): Promise<void> {
  logger.debug("Handling VFS init message", {
    manifestSize: message.manifest.byteLength,
    wsUrl: message.wsUrl,
    launcherBundleId: message.launcherBundleId,
  });

  try {
    const state = getBundleState(message.launcherBundleId);

    // Check if we already have a Tonk instance for this bundle
    if (state?.status === "active") {
      logger.debug("Tonk already initialized for bundle", {
        launcherBundleId: message.launcherBundleId,
      });
      postResponse({
        type: "init",
        success: true,
      });
      return;
    }

    // If Tonk is still loading, wait for it
    if (state?.status === "loading") {
      logger.debug("Tonk is loading, waiting for completion", {
        launcherBundleId: message.launcherBundleId,
      });
      try {
        await state.promise;
        logger.debug("Tonk loading completed");
        postResponse({
          type: "init",
          success: true,
        });
      } catch (error) {
        logger.error("Tonk loading failed", {
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: "init",
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      return;
    }

    // If Tonk failed to initialize, respond with error
    if (state?.status === "error") {
      logger.error("Tonk initialization failed previously", {
        error: state.error.message,
      });
      postResponse({
        type: "init",
        success: false,
        error: state.error.message,
      });
      return;
    }

    // If uninitialized, this shouldn't happen in normal flow
    logger.warn("Tonk is uninitialized, this is unexpected", {
      launcherBundleId: message.launcherBundleId,
    });
    postResponse({
      type: "init",
      success: false,
      error: "Tonk not initialized",
    });
  } catch (error) {
    logger.error("Failed to handle init message", {
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: "init",
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handleInitializeFromUrl(message: {
  id?: string;
  manifestUrl?: string;
  wasmUrl?: string;
  wsUrl?: string;
  launcherBundleId: string;
}): Promise<void> {
  logger.debug("Initializing from URL", {
    manifestUrl: message.manifestUrl,
    wasmUrl: message.wasmUrl,
    launcherBundleId: message.launcherBundleId,
  });

  try {
    // Extract manifest URL from message, with default
    const manifestUrl =
      message.manifestUrl || `${TONK_SERVER_URL}/.manifest.tonk`;

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    // Fetch the manifest data
    logger.debug("Fetching manifest from URL", { manifestUrl });
    const manifestResponse = await fetch(manifestUrl);
    const manifestBytes = new Uint8Array(await manifestResponse.arrayBuffer());
    logger.debug("Manifest bytes loaded", { byteLength: manifestBytes.length });

    // Create bundle and manifest
    const bundle = await Bundle.fromBytes(manifestBytes);
    const manifest = await bundle.getManifest();
    logger.debug("Bundle and manifest created");

    // Derive wsUrl: explicit param > manifest networkUris > build-time default
    const wsUrlInit = message.wsUrl || getWsUrlFromManifest(manifest);

    // Create TonkCore instance with namespace for IndexedDB isolation
    logger.debug("Creating TonkCore from manifest bytes");
    const tonk = await TonkCore.fromBytes(manifestBytes, {
      storage: { type: "indexeddb", namespace: message.launcherBundleId },
    });
    logger.debug("TonkCore created");

    // Connect to websocket
    await persistWsUrl(wsUrlInit);

    logger.debug("Connecting to websocket", { wsUrl: wsUrlInit });
    await tonk.connectWebsocket(wsUrlInit);
    logger.debug("Websocket connection established");

    // Wait for PathIndex to sync from remote
    await waitForPathIndexSync(tonk);
    logger.debug("PathIndex sync complete");

    // Set bundle state
    setBundleState(message.launcherBundleId, {
      status: "active",
      bundleId: manifest.rootId,
      launcherBundleId: message.launcherBundleId,
      tonk,
      manifest,
      appSlug: manifest.entrypoints?.[0] || "app",
      wsUrl: wsUrlInit,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    setLastActiveBundleId(message.launcherBundleId);
    startHealthMonitoring(message.launcherBundleId);

    // Persist bundle bytes to survive service worker restarts
    await persistBundleBytes(manifestBytes);
    await persistNamespace(message.launcherBundleId);
    await persistLastActiveBundleId(message.launcherBundleId);
    logger.debug("Bundle bytes persisted");

    logger.info("Initialized from URL successfully");
    postResponse({
      type: "initializeFromUrl",
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error("Failed to initialize from URL", {
      error: error instanceof Error ? error.message : String(error),
    });

    setBundleState(message.launcherBundleId, {
      status: "error",
      launcherBundleId: message.launcherBundleId,
      error: error instanceof Error ? error : new Error(String(error)),
    });

    postResponse({
      type: "initializeFromUrl",
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handleInitializeFromBytes(message: {
  id?: string;
  bundleBytes: ArrayBuffer;
  serverUrl?: string;
  wsUrl?: string;
  launcherBundleId: string;
}): Promise<void> {
  logger.debug("Initializing from bytes", {
    byteLength: message.bundleBytes.byteLength,
    serverUrl: message.serverUrl,
    launcherBundleId: message.launcherBundleId,
  });

  try {
    const serverUrl = message.serverUrl || TONK_SERVER_URL;

    logger.debug("Using server URL", { serverUrl });

    // Initialize WASM (singleton - safe to call multiple times)
    await ensureWasmInitialized();

    const bundleBytes = new Uint8Array(message.bundleBytes);
    logger.debug("Creating bundle from bytes", {
      byteLength: bundleBytes.length,
    });

    const bundle = await Bundle.fromBytes(bundleBytes);
    const manifest = await bundle.getManifest();
    logger.debug("Bundle and manifest created", { rootId: manifest.rootId });

    // Derive wsUrl: explicit param > manifest networkUris > server URL fallback
    const wsUrlInit =
      message.wsUrl || getWsUrlFromManifest(manifest, serverUrl);

    logger.debug("Creating TonkCore from bundle bytes");
    const tonk = await TonkCore.fromBytes(bundleBytes, {
      storage: { type: "indexeddb", namespace: message.launcherBundleId },
    });
    logger.debug("TonkCore created");

    logger.debug("Connecting to websocket", { wsUrl: wsUrlInit });
    if (wsUrlInit) {
      await tonk.connectWebsocket(wsUrlInit);
      logger.debug("Websocket connection established");

      // Wait for PathIndex to sync from remote
      await waitForPathIndexSync(tonk);
      logger.debug("PathIndex sync complete");
    }

    // Set bundle state
    setBundleState(message.launcherBundleId, {
      status: "active",
      bundleId: manifest.rootId,
      launcherBundleId: message.launcherBundleId,
      tonk,
      manifest,
      appSlug: manifest.entrypoints?.[0] || "app",
      wsUrl: wsUrlInit,
      healthCheckInterval: null,
      watchers: new Map(),
      connectionHealthy: true,
      reconnectAttempts: 0,
    });

    setLastActiveBundleId(message.launcherBundleId);
    startHealthMonitoring(message.launcherBundleId);

    await persistBundleBytes(bundleBytes);
    await persistNamespace(message.launcherBundleId);
    await persistLastActiveBundleId(message.launcherBundleId);
    logger.debug("Bundle bytes persisted");

    logger.info("Initialized from bytes successfully");
    postResponse({
      type: "initializeFromBytes",
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error("Failed to initialize from bytes", {
      error: error instanceof Error ? error.message : String(error),
    });

    setBundleState(message.launcherBundleId, {
      status: "error",
      launcherBundleId: message.launcherBundleId,
      error: error instanceof Error ? error : new Error(String(error)),
    });

    postResponse({
      type: "initializeFromBytes",
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handleGetServerUrl(message: {
  id: string;
  launcherBundleId: string;
}): Promise<void> {
  logger.debug("Getting server URL");
  postResponse({
    type: "getServerUrl",
    id: message.id,
    success: true,
    data: TONK_SERVER_URL,
  });
}

export async function handleGetManifest(message: {
  id: string;
  launcherBundleId: string;
}): Promise<void> {
  logger.debug("Getting manifest", {
    launcherBundleId: message.launcherBundleId,
  });
  try {
    const tonkInstance = getTonkForBundle(message.launcherBundleId);
    if (!tonkInstance) {
      throw new Error("Tonk not initialized");
    }

    postResponse({
      type: "getManifest",
      id: message.id,
      success: true,
      data: tonkInstance.manifest,
    });
  } catch (error) {
    logger.error("Failed to get manifest", {
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: "getManifest",
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handlePing(): Promise<void> {
  logger.debug("Ping received");
  postResponse({
    type: "ready",
    needsBundle: true,
  });
}

export async function handleSetAppSlug(message: {
  launcherBundleId: string;
  slug: string;
}): Promise<void> {
  setAppSlug(message.launcherBundleId, message.slug);

  // Persist to cache so it survives service worker restarts
  await persistAppSlug(message.slug);

  logger.debug("App slug set and persisted", {
    slug: message.slug,
    launcherBundleId: message.launcherBundleId,
  });
}
