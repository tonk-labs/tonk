import { TonkCore } from '@tonk/core';

// DocumentWatcher type
interface DocumentWatcher {
  stop: () => void;
}

import type { VFSWorkerMessage, VFSWorkerResponse } from './types';

console.log('[Worker] Worker script loading...');

// Debug logging flag - set to true to enable comprehensive logging
const DEBUG_LOGGING = false;

// Logger utility
function log(
  level: 'info' | 'warn' | 'debug' | 'error',
  message: string,
  data?: Record<string, unknown>
) {
  if (!DEBUG_LOGGING) return;

  const timestamp = new Date().toISOString();
  const prefix = `[VFS Worker ${timestamp}] ${level.toUpperCase()}:`;

  if (data !== undefined) {
    console[level](prefix, message, data);
  } else {
    console[level](prefix, message);
  }
}

// Worker state
let tonk: TonkCore | null = null;
const watchers = new Map<string, DocumentWatcher>();

// Helper to post messages back to main thread
function postResponse(response: VFSWorkerResponse) {
  log('info', 'Posting response to main thread', {
    type: response.type,
    success: 'success' in response ? response.success : 'N/A',
  });
  self.postMessage(response);
}

// Initialize TonkCore
async function initializeTonk(manifest: ArrayBuffer, wsUrl: string) {
  log('info', 'Starting TonkCore initialization', {
    manifestSize: manifest.byteLength,
    wsUrl,
  });

  try {
    log('info', 'About to create Uint8Array from manifest');
    const bytes = new Uint8Array(manifest);
    log(
      'info',
      'Successfully created Uint8Array, about to call TonkCore.fromBytes'
    );
    log('info', 'Creating TonkCore from bytes', {
      bytesLength: bytes.length,
      manifestByteLength: manifest.byteLength,
      firstFewBytes: Array.from(bytes.slice(0, 10)),
      lastFewBytes: Array.from(bytes.slice(-10)),
    });

    tonk = await TonkCore.fromBytes(bytes, {
      storage: { type: 'indexeddb' },
    });
    log('info', 'TonkCore created successfully');

    log('info', 'Connecting to websocket', { wsUrl });
    await tonk.connectWebsocket(wsUrl);
    log('info', 'Websocket connection initiated');

    // Wait for connection
    let retries = 50;
    log('info', 'Testing connection with directory listing', {
      maxRetries: retries,
    });

    while (retries > 0) {
      // Check if connected (you may need to adjust this based on TonkCore API)
      try {
        await tonk.listDirectory('/');
        log('info', 'Connection test successful', {
          retriesRemaining: retries,
        });
        break;
      } catch (testError) {
        log('warn', 'Connection test failed, retrying', {
          retriesRemaining: retries - 1,
          error:
            testError instanceof Error ? testError.message : String(testError),
        });
        await new Promise(resolve => setTimeout(resolve, 100));
        retries--;
      }
    }

    if (retries === 0) {
      log('error', 'Connection test failed after all retries');
      throw new Error('Failed to connect to websocket');
    }

    log('info', 'TonkCore initialization completed successfully');
    postResponse({ type: 'init', success: true });
  } catch (error) {
    log('error', 'TonkCore initialization failed', {
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'init',
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle file operations
async function handleMessage(message: VFSWorkerMessage) {
  log('info', 'Worker received message', {
    type: message.type,
    id: 'id' in message ? message.id : 'N/A',
    hasManifest: message.type === 'init' ? 'manifest' in message : false,
  });

  if (!tonk && message.type !== 'init') {
    log('error', 'Operation attempted before VFS initialization', {
      type: message.type,
    });
    if ('id' in message) {
      postResponse({
        type: message.type,
        id: message.id,
        success: false,
        error: 'VFS not initialized',
      });
    }
    return;
  }

  switch (message.type) {
    case 'init':
      log('info', 'Received init message, calling initializeTonk', {
        manifestSize: message.manifest.byteLength,
        wsUrl: message.wsUrl,
      });
      await initializeTonk(message.manifest, message.wsUrl);
      break;

    case 'readFile':
      log('info', 'Reading file', { path: message.path, id: message.id });
      try {
        const documentData = await tonk!.readFile(message.path);
        log('info', 'File read successfully', {
          path: message.path,
          documentType: documentData.type,
          hasBytes: !!documentData.bytes,
          contentType: typeof documentData.content,
        });

        postResponse({
          type: 'readFile',
          id: message.id,
          success: true,
          data: documentData,
        });
      } catch (error) {
        log('error', 'Failed to read file', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'readFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'writeFile':
      log('info', 'Writing file', {
        path: message.path,
        id: message.id,
        create: message.create,
        hasBytes: !!message.content.bytes,
        contentType: typeof message.content.content,
      });
      try {
        if (message.create) {
          log('info', 'Creating new file', { path: message.path });
          if (message.content.bytes) {
            // Create file with bytes
            await tonk!.createFileWithBytes(
              message.path,
              message.content.content,
              message.content.bytes
            );
          } else {
            // Create file with content only
            await tonk!.createFile(message.path, message.content.content);
          }
        } else {
          log('info', 'Updating existing file', { path: message.path });
          if (message.content.bytes) {
            // Update file with bytes
            await tonk!.updateFileWithBytes(
              message.path,
              message.content.content,
              message.content.bytes
            );
          } else {
            // Update file with content only
            await tonk!.updateFile(message.path, message.content.content);
          }
        }
        log('info', 'File write completed successfully', {
          path: message.path,
        });
        postResponse({
          type: 'writeFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to write file', {
          path: message.path,
          create: message.create,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'writeFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'deleteFile':
      log('info', 'Deleting file', { path: message.path, id: message.id });
      try {
        await tonk!.deleteFile(message.path);
        log('info', 'File deleted successfully', { path: message.path });
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to delete file', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'listDirectory':
      log('info', 'Listing directory', { path: message.path, id: message.id });
      try {
        const files = await tonk!.listDirectory(message.path);
        log('info', 'Directory listed successfully', {
          path: message.path,
          fileCount: Array.isArray(files) ? files.length : 'unknown',
        });
        // Pass through the raw response from TonkCore
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: true,
          data: files,
        });
      } catch (error) {
        log('error', 'Failed to list directory', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'exists':
      log('info', 'Checking file existence', {
        path: message.path,
        id: message.id,
      });
      try {
        const exists = await tonk!.exists(message.path);
        log('info', 'File existence check completed', {
          path: message.path,
          exists,
        });
        postResponse({
          type: 'exists',
          id: message.id,
          success: true,
          data: exists,
        });
      } catch (error) {
        log('error', 'Failed to check file existence', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'exists',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'watchFile':
      log('info', 'Starting file watch', {
        path: message.path,
        id: message.id,
      });
      try {
        const watcher = await tonk!.watchFile(message.path, documentData => {
          log('info', 'File change detected', {
            watchId: message.id,
            path: message.path,
            documentType: documentData.type,
            hasBytes: !!documentData.bytes,
          });

          postResponse({
            type: 'fileChanged',
            watchId: message.id,
            documentData: documentData,
          });
        });
        watchers.set(message.id, watcher!);
        log('info', 'File watch started successfully', {
          path: message.path,
          watchId: message.id,
          totalWatchers: watchers.size,
        });
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to start file watch', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'unwatchFile':
      log('info', 'Stopping file watch', { watchId: message.id });
      try {
        const watcher = watchers.get(message.id);
        if (watcher) {
          log('info', 'Found watcher, stopping it', { watchId: message.id });
          watcher.stop();
          watchers.delete(message.id);
          log('info', 'File watch stopped successfully', {
            watchId: message.id,
            remainingWatchers: watchers.size,
          });
        } else {
          log('warn', 'No watcher found for ID', { watchId: message.id });
        }
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to stop file watch', {
          watchId: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'watchDirectory':
      log('info', 'Starting directory watch', {
        path: message.path,
        id: message.id,
      });
      try {
        const watcher = await tonk!.watchDirectory(
          message.path,
          (changeData: any) => {
            log('info', 'Directory change detected', {
              watchId: message.id,
              path: message.path,
            });

            postResponse({
              type: 'directoryChanged',
              watchId: message.id,
              path: message.path,
              changeData,
            });
          }
        );
        watchers.set(message.id, watcher!);
        log('info', 'Directory watch started successfully', {
          path: message.path,
          watchId: message.id,
          totalWatchers: watchers.size,
        });
        postResponse({
          type: 'watchDirectory',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to start directory watch', {
          path: message.path,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'watchDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'unwatchDirectory':
      log('info', 'Stopping directory watch', { watchId: message.id });
      try {
        const watcher = watchers.get(message.id);
        if (watcher) {
          log('info', 'Found directory watcher, stopping it', {
            watchId: message.id,
          });
          watcher.stop();
          watchers.delete(message.id);
          log('info', 'Directory watch stopped successfully', {
            watchId: message.id,
            remainingWatchers: watchers.size,
          });
        } else {
          log('warn', 'No directory watcher found for ID', {
            watchId: message.id,
          });
        }
        postResponse({
          type: 'unwatchDirectory',
          id: message.id,
          success: true,
        });
      } catch (error) {
        log('error', 'Failed to stop directory watch', {
          watchId: message.id,
          error: error instanceof Error ? error.message : String(error),
        });
        postResponse({
          type: 'unwatchDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;
  }
}

// Worker startup
log('info', 'VFS Worker started', { debugLogging: DEBUG_LOGGING });

// Signal to main thread that worker is ready
self.postMessage({ type: 'ready' });

// Listen for messages from main thread
self.addEventListener('message', async event => {
  console.log('[Worker] Raw message received:', event.type, event.data?.type);
  log('info', 'Raw message event received', {
    eventType: event.type,
    messageType: event.data?.type,
    hasData: !!event.data,
  });
  try {
    await handleMessage(event.data as VFSWorkerMessage);
  } catch (error) {
    log('error', 'Error handling message', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
});
