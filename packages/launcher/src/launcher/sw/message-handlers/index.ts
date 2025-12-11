import type { Manifest } from '@tonk/core/slim';
import { getState } from '../state';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';
// Bundle operations
import { handleForkToBytes, handleLoadBundle, handleToBytes } from './bundle-ops';

// Directory operations
import { handleListDirectory } from './directory-ops';
// File operations
import {
  handleDeleteFile,
  handleExists,
  handlePatchFile,
  handleReadFile,
  handleRename,
  handleUpdateFile,
  handleWriteFile,
} from './file-ops';
// Init operations
import {
  handleGetManifest,
  handleGetServerUrl,
  handleInit,
  handleInitializeFromBytes,
  handleInitializeFromUrl,
  handlePing,
  handleSetAppSlug,
} from './init-ops';
// Watch operations
import {
  handleUnwatchDirectory,
  handleUnwatchFile,
  handleWatchDirectory,
  handleWatchFile,
} from './watch-ops';

// Message type - we use unknown and cast in handlers
type Message = Record<string, unknown>;

// Operations allowed when tonk is not initialized
const allowedWhenUninitialized = [
  'init',
  'loadBundle',
  'initializeFromUrl',
  'initializeFromBytes',
  'getServerUrl',
  'ping',
  'setAppSlug',
];

export async function handleMessage(message: Message): Promise<void> {
  const msgType = message.type as string;
  const msgId = message.id as string | undefined;

  logger.debug('Received message', {
    type: msgType,
    id: msgId || 'N/A',
  });

  const state = getState();

  // Handle setAppSlug separately (doesn't require active state)
  if (msgType === 'setAppSlug') {
    await handleSetAppSlug({ slug: message.slug as string });
    return;
  }

  // Handle ping separately
  if (msgType === 'ping') {
    await handlePing();
    return;
  }

  // Check if operation is allowed when not active
  if (state.status !== 'active' && !allowedWhenUninitialized.includes(msgType)) {
    logger.warn('Operation attempted before VFS initialization', {
      type: msgType,
      status: state.status,
    });
    if (msgId) {
      postResponse({
        type: msgType,
        id: msgId,
        success: false,
        error: 'VFS not initialized. Please load a bundle first.',
      });
    }
    return;
  }

  // Route to appropriate handler
  switch (msgType) {
    // Init operations
    case 'init':
      await handleInit({
        ...(msgId !== undefined && { id: msgId }),
        manifest: message.manifest as ArrayBuffer,
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
      });
      break;

    case 'initializeFromUrl':
      await handleInitializeFromUrl({
        ...(msgId !== undefined && { id: msgId }),
        ...(message.manifestUrl !== undefined && {
          manifestUrl: message.manifestUrl as string,
        }),
        ...(message.wasmUrl !== undefined && {
          wasmUrl: message.wasmUrl as string,
        }),
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
      });
      break;

    case 'initializeFromBytes':
      await handleInitializeFromBytes({
        ...(msgId !== undefined && { id: msgId }),
        bundleBytes: message.bundleBytes as ArrayBuffer,
        ...(message.serverUrl !== undefined && {
          serverUrl: message.serverUrl as string,
        }),
        ...(message.wsUrl !== undefined && { wsUrl: message.wsUrl as string }),
      });
      break;

    case 'getServerUrl':
      await handleGetServerUrl({ id: msgId as string });
      break;

    case 'getManifest':
      await handleGetManifest({ id: msgId as string });
      break;

    // Bundle operations
    case 'loadBundle':
      await handleLoadBundle({
        ...(msgId !== undefined && { id: msgId }),
        bundleBytes: message.bundleBytes as ArrayBuffer,
        ...(message.serverUrl !== undefined && {
          serverUrl: message.serverUrl as string,
        }),
        ...(message.manifest !== undefined && {
          manifest: message.manifest as Manifest,
        }),
      });
      break;

    case 'toBytes':
      await handleToBytes({ id: msgId as string });
      break;

    case 'forkToBytes':
      await handleForkToBytes({ id: msgId as string });
      break;

    // File operations
    case 'readFile':
      await handleReadFile({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    case 'writeFile':
      await handleWriteFile({
        id: msgId as string,
        path: message.path as string,
        ...(message.create !== undefined && {
          create: message.create as boolean,
        }),
        content: message.content as { bytes?: Uint8Array; content: unknown },
      });
      break;

    case 'deleteFile':
      await handleDeleteFile({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    case 'rename':
      await handleRename({
        id: msgId as string,
        oldPath: message.oldPath as string,
        newPath: message.newPath as string,
      });
      break;

    case 'exists':
      await handleExists({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    case 'patchFile':
      await handlePatchFile({
        id: msgId as string,
        path: message.path as string,
        jsonPath: message.jsonPath as string[],
        value: message.value,
      });
      break;

    case 'updateFile':
      await handleUpdateFile({
        id: msgId as string,
        path: message.path as string,
        content: message.content,
      });
      break;

    // Directory operations
    case 'listDirectory':
      await handleListDirectory({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    // Watch operations
    case 'watchFile':
      await handleWatchFile({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    case 'unwatchFile':
      await handleUnwatchFile({ id: msgId as string });
      break;

    case 'watchDirectory':
      await handleWatchDirectory({
        id: msgId as string,
        path: message.path as string,
      });
      break;

    case 'unwatchDirectory':
      await handleUnwatchDirectory({ id: msgId as string });
      break;

    default:
      logger.warn('Unknown message type', { type: msgType });
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
