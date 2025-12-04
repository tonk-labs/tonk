import { log } from './logging';
import { SERVER_URL } from './constants';
import {
  getTonkState,
  setTonkState,
  getTonk,
  setAppSlug,
  getAppSlug,
} from './state';
import { persistAppSlug } from './persistence';
import { postResponse } from './message-utils';
import { initializeTonkCore, initializeFromUrl } from './lifecycle';
import {
  handleReadFile,
  handleWriteFile,
  handleDeleteFile,
  handleRename,
  handleListDirectory,
  handleExists,
  handleWatchFile,
  handleUnwatchFile,
  handleWatchDirectory,
  handleUnwatchDirectory,
  handleToBytes,
  handleForkToBytes,
} from './vfs-handlers';
import type { VFSWorkerMessage } from './types';

// Message types that can be handled when tonk is not ready
const ALLOWED_WHEN_UNINITIALIZED = [
  'init',
  'loadBundle',
  'initializeFromUrl',
  'initializeFromBytes',
  'getServerUrl',
  'setAppSlug',
];

// Handle init message
async function handleInit(message: { manifest: ArrayBuffer; wsUrl: string }): Promise<void> {
  log('info', 'Handling VFS init message', {
    manifestSize: message.manifest.byteLength,
    wsUrl: message.wsUrl,
  });

  try {
    const state = getTonkState();

    // Check if we already have a Tonk instance
    if (state.status === 'ready') {
      log('info', 'Tonk already initialized, responding with success');
      await postResponse({ type: 'init', success: true });
      return;
    }

    // If Tonk is still loading, wait for it
    if (state.status === 'loading') {
      log('info', 'Tonk is loading, waiting for completion');
      try {
        await state.promise;
        log('info', 'Tonk loading completed, responding with success');
        await postResponse({ type: 'init', success: true });
      } catch (error) {
        log('error', 'Tonk loading failed', {
          error: error instanceof Error ? error.message : String(error),
        });
        await postResponse({
          type: 'init',
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      return;
    }

    // If Tonk failed to initialize, respond with error
    if (state.status === 'failed') {
      log('error', 'Tonk initialization failed previously', {
        error: state.error.message,
      });
      await postResponse({
        type: 'init',
        success: false,
        error: state.error.message,
      });
      return;
    }

    // If uninitialized, this shouldn't happen in normal flow
    log('warn', 'Tonk is uninitialized, this is unexpected');
    await postResponse({
      type: 'init',
      success: false,
      error: 'Tonk not initialized',
    });
  } catch (error) {
    log('error', 'Failed to handle init message', {
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'init',
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle loadBundle message
async function handleLoadBundle(
  id: string,
  bundleBytes: ArrayBuffer,
  serverUrl?: string
): Promise<void> {
  log('info', 'Loading new bundle', {
    id,
    byteLength: bundleBytes.byteLength,
    serverUrl,
  });

  try {
    const bytes = new Uint8Array(bundleBytes);
    await initializeTonkCore({
      bundleBytes: bytes,
      serverUrl: serverUrl || SERVER_URL,
    });

    await postResponse({
      type: 'loadBundle',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to load bundle', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'loadBundle',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle initializeFromUrl message
async function handleInitializeFromUrl(
  id: string,
  manifestUrl?: string,
  wasmUrl?: string,
  wsUrl?: string
): Promise<void> {
  log('info', 'Initializing from URL', { id, manifestUrl, wasmUrl });

  try {
    await initializeFromUrl({ manifestUrl, wasmUrl, wsUrl });
    await postResponse({
      type: 'initializeFromUrl',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to initialize from URL', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    setTonkState({ status: 'failed', error: error as Error });
    await postResponse({
      type: 'initializeFromUrl',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle initializeFromBytes message
async function handleInitializeFromBytes(
  id: string,
  bundleBytes: ArrayBuffer,
  serverUrl?: string,
  wsUrl?: string
): Promise<void> {
  log('info', 'Initializing from bytes', {
    id,
    byteLength: bundleBytes.byteLength,
    serverUrl,
    wsUrl,
  });

  try {
    const bytes = new Uint8Array(bundleBytes);
    await initializeTonkCore({
      bundleBytes: bytes,
      serverUrl: serverUrl || SERVER_URL,
      wsUrl,
    });

    await postResponse({
      type: 'initializeFromBytes',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to initialize from bytes', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    setTonkState({ status: 'failed', error: error as Error });
    await postResponse({
      type: 'initializeFromBytes',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle getServerUrl message
async function handleGetServerUrl(id: string): Promise<void> {
  log('info', 'Getting server URL', { id });
  await postResponse({
    type: 'getServerUrl',
    id,
    success: true,
    data: SERVER_URL,
  });
}

// Handle getManifest message
async function handleGetManifest(id: string): Promise<void> {
  log('info', 'Getting manifest', { id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    await postResponse({
      type: 'getManifest',
      id,
      success: true,
      data: tonkInstance.manifest,
    });
  } catch (error) {
    log('error', 'Failed to get manifest', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'getManifest',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle setAppSlug message
async function handleSetAppSlug(slug: string): Promise<void> {
  setAppSlug(slug);
  await persistAppSlug(slug);
  log('info', 'App slug set and persisted', { slug });
}

// Main message handler
export async function handleMessage(
  message: VFSWorkerMessage | { type: 'setAppSlug'; slug: string }
): Promise<void> {
  log('info', 'Received message', {
    type: message.type,
    id: 'id' in message ? message.id : 'N/A',
  });

  // Handle setAppSlug message separately (no response needed)
  if (message.type === 'setAppSlug') {
    await handleSetAppSlug(message.slug);
    return;
  }

  // Check if operation is allowed when tonk is not ready
  const state = getTonkState();
  if (
    state.status !== 'ready' &&
    !ALLOWED_WHEN_UNINITIALIZED.includes(message.type)
  ) {
    log('error', 'Operation attempted before VFS initialization', {
      type: message.type,
      status: state.status,
    });
    if ('id' in message) {
      await postResponse({
        type: message.type,
        id: message.id,
        success: false,
        error: 'VFS not initialized. Please load a bundle first.',
      } as any);
    }
    return;
  }

  // Route to appropriate handler
  switch (message.type) {
    case 'init':
      await handleInit(message);
      break;

    case 'loadBundle':
      await handleLoadBundle(
        message.id,
        message.bundleBytes,
        (message as any).serverUrl
      );
      break;

    case 'initializeFromUrl':
      await handleInitializeFromUrl(
        message.id,
        message.manifestUrl,
        message.wasmUrl,
        message.wsUrl
      );
      break;

    case 'initializeFromBytes':
      await handleInitializeFromBytes(
        message.id,
        message.bundleBytes,
        message.serverUrl,
        message.wsUrl
      );
      break;

    case 'getServerUrl':
      await handleGetServerUrl(message.id);
      break;

    case 'getManifest':
      await handleGetManifest(message.id);
      break;

    case 'readFile':
      await handleReadFile(message.id, message.path);
      break;

    case 'writeFile':
      await handleWriteFile(
        message.id,
        message.path,
        message.content,
        message.create
      );
      break;

    case 'deleteFile':
      await handleDeleteFile(message.id, message.path);
      break;

    case 'rename':
      await handleRename(message.id, message.oldPath, message.newPath);
      break;

    case 'listDirectory':
      await handleListDirectory(message.id, message.path);
      break;

    case 'exists':
      await handleExists(message.id, message.path);
      break;

    case 'watchFile':
      await handleWatchFile(message.id, message.path);
      break;

    case 'unwatchFile':
      await handleUnwatchFile(message.id);
      break;

    case 'watchDirectory':
      await handleWatchDirectory(message.id, message.path);
      break;

    case 'unwatchDirectory':
      await handleUnwatchDirectory(message.id);
      break;

    case 'toBytes':
      await handleToBytes(message.id);
      break;

    case 'forkToBytes':
      await handleForkToBytes(message.id);
      break;

    default:
      log('warn', 'Unknown message type', { type: (message as any).type });
  }
}
