import type { Manifest } from "@tonk/core/slim";
import { getBundleState, getLastActiveBundleId } from "../state";
import { logger } from "../utils/logging";
import { postResponse } from "../utils/response";
// Bundle operations
import {
  handleForkToBytes,
  handleLoadBundle,
  handleToBytes,
  handleUnloadBundle,
} from "./bundle-ops";

// Directory operations
import { handleListDirectory } from "./directory-ops";
// File operations
import {
  handleDeleteFile,
  handleExists,
  handlePatchFile,
  handleReadFile,
  handleRename,
  handleUpdateFile,
  handleWriteFile,
} from "./file-ops";
// Init operations
import {
  handleGetManifest,
  handleGetServerUrl,
  handleInit,
  handleInitializeFromBytes,
  handleInitializeFromUrl,
  handlePing,
  handleSetAppSlug,
} from "./init-ops";
// Watch operations
import {
  handleUnwatchDirectory,
  handleUnwatchFile,
  handleWatchDirectory,
  handleWatchFile,
} from "./watch-ops";

// Message type - we use unknown and cast in handlers
type Message = Record<string, unknown>;

// Operations allowed when bundle is not initialized
const allowedWhenUninitialized = [
  "init",
  "loadBundle",
  "unloadBundle",
  "initializeFromUrl",
  "initializeFromBytes",
  "getServerUrl",
  "ping",
  "setAppSlug",
];

export async function handleMessage(message: Message): Promise<void> {
  const msgType = message.type as string;
  const msgId = message.id as string | undefined;
  const launcherBundleId = message.launcherBundleId as string | undefined;

  logger.debug("Received message", {
    type: msgType,
    id: msgId || "N/A",
    launcherBundleId: launcherBundleId || "N/A",
  });

  // Handle ping separately (doesn't require bundle context)
  if (msgType === "ping") {
    await handlePing();
    return;
  }

  // Handle setAppSlug separately (requires launcherBundleId)
  if (msgType === "setAppSlug") {
    if (!launcherBundleId) {
      logger.warn("setAppSlug missing launcherBundleId");
      return;
    }
    await handleSetAppSlug({
      launcherBundleId,
      slug: message.slug as string,
    });
    return;
  }

  // For operations requiring bundle context, check if bundle is initialized
  if (!allowedWhenUninitialized.includes(msgType)) {
    // Determine which bundle to use
    const bundleId = launcherBundleId || getLastActiveBundleId();

    if (!bundleId) {
      logger.warn("Operation attempted without bundle context", {
        type: msgType,
      });
      if (msgId) {
        postResponse({
          type: msgType,
          id: msgId,
          success: false,
          error: "No bundle context. Please load a bundle first.",
        });
      }
      return;
    }

    const state = getBundleState(bundleId);
    if (!state || state.status !== "active") {
      logger.warn("Operation attempted before bundle initialization", {
        type: msgType,
        launcherBundleId: bundleId,
        status: state?.status || "none",
      });
      if (msgId) {
        postResponse({
          type: msgType,
          id: msgId,
          success: false,
          error: "Bundle not initialized. Please load a bundle first.",
        });
      }
      return;
    }
  }

  // Resolve the effective bundle ID for operations that need it
  const effectiveBundleId = launcherBundleId || getLastActiveBundleId() || "";

  // Route to appropriate handler
  switch (msgType) {
    // Init operations
    case "init":
      await handleInit({
        ...(msgId !== undefined && { id: msgId }),
        manifest: message.manifest as ArrayBuffer,
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "initializeFromUrl":
      await handleInitializeFromUrl({
        ...(msgId !== undefined && { id: msgId }),
        ...(message.manifestUrl !== undefined && {
          manifestUrl: message.manifestUrl as string,
        }),
        ...(message.wasmUrl !== undefined && {
          wasmUrl: message.wasmUrl as string,
        }),
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "initializeFromBytes":
      await handleInitializeFromBytes({
        ...(msgId !== undefined && { id: msgId }),
        bundleBytes: message.bundleBytes as ArrayBuffer,
        ...(message.serverUrl !== undefined && {
          serverUrl: message.serverUrl as string,
        }),
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "getServerUrl":
      await handleGetServerUrl({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "getManifest":
      await handleGetManifest({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    // Bundle operations
    case "loadBundle":
      await handleLoadBundle({
        ...(msgId !== undefined && { id: msgId }),
        bundleBytes: message.bundleBytes as ArrayBuffer,
        ...(message.serverUrl !== undefined && {
          serverUrl: message.serverUrl as string,
        }),
        ...(message.manifest !== undefined && {
          manifest: message.manifest as Manifest,
        }),
        launcherBundleId: message.launcherBundleId as string,
      });
      break;

    case "unloadBundle":
      await handleUnloadBundle({
        ...(msgId !== undefined && { id: msgId }),
        launcherBundleId: message.launcherBundleId as string,
      });
      break;

    case "toBytes":
      await handleToBytes({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "forkToBytes":
      await handleForkToBytes({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    // File operations
    case "readFile":
      await handleReadFile({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "writeFile":
      await handleWriteFile({
        id: msgId as string,
        path: message.path as string,
        ...(message.create !== undefined && {
          create: message.create as boolean,
        }),
        content: message.content as { bytes?: Uint8Array; content: unknown },
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "deleteFile":
      await handleDeleteFile({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "rename":
      await handleRename({
        id: msgId as string,
        oldPath: message.oldPath as string,
        newPath: message.newPath as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "exists":
      await handleExists({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "patchFile":
      await handlePatchFile({
        id: msgId as string,
        path: message.path as string,
        jsonPath: message.jsonPath as string[],
        value: message.value,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "updateFile":
      await handleUpdateFile({
        id: msgId as string,
        path: message.path as string,
        content: message.content,
        launcherBundleId: effectiveBundleId,
      });
      break;

    // Directory operations
    case "listDirectory":
      await handleListDirectory({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    // Watch operations
    case "watchFile":
      await handleWatchFile({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "unwatchFile":
      await handleUnwatchFile({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "watchDirectory":
      await handleWatchDirectory({
        id: msgId as string,
        path: message.path as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    case "unwatchDirectory":
      await handleUnwatchDirectory({
        id: msgId as string,
        launcherBundleId: effectiveBundleId,
      });
      break;

    default:
      logger.warn("Unknown message type", { type: msgType });
      if (msgId) {
        postResponse({
          type: msgType,
          id: msgId,
          success: false,
          error: `Unknown message type: ${msgType}`,
        });
      }
  }
}
