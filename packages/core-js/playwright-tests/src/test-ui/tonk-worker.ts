import { TonkCore } from '@tonk/core';
import type { VFSWorkerMessage, VFSWorkerResponse } from './types';

// Worker state
let tonk: any | null = null;
const watchers = new Map<string, any>();

// Helper to post messages back to main thread
function postResponse(response: VFSWorkerResponse) {
  self.postMessage(response);
}

// Initialize TonkCore
async function initializeTonk(manifest: ArrayBuffer, wsUrl: string) {
  console.log('[Worker] Initializing TonkCore', {
    manifestSize: manifest.byteLength,
    wsUrl,
  });

  try {
    const bytes = new Uint8Array(manifest);
    console.log('[Worker] Creating TonkCore from bytes');

    tonk = await TonkCore.fromBytes(bytes, {
      storage: { type: 'indexeddb' },
    });
    console.log('[Worker] TonkCore created successfully');

    console.log('[Worker] Connecting to websocket:', wsUrl);
    await tonk.connectWebsocket(wsUrl);
    console.log('[Worker] Websocket connection initiated');

    // Wait for connection to be ready
    let retries = 50;
    while (retries > 0) {
      try {
        await tonk.listDirectory('/');
        console.log('[Worker] Connection test successful');
        break;
      } catch (error) {
        await new Promise(resolve => setTimeout(resolve, 100));
        retries--;
      }
    }

    if (retries === 0) {
      throw new Error('Failed to connect to websocket');
    }

    console.log('[Worker] TonkCore initialization completed');
    postResponse({ type: 'init', success: true });
  } catch (error) {
    console.error('[Worker] TonkCore initialization failed:', error);
    postResponse({
      type: 'init',
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Handle file operations
async function handleMessage(message: VFSWorkerMessage) {
  console.log('[Worker] Received message:', message.type);

  if (!tonk && message.type !== 'init') {
    console.error('[Worker] Operation attempted before initialization');
    if ('id' in message) {
      postResponse({
        type: message.type as any,
        id: message.id,
        success: false,
        error: 'VFS not initialized',
      });
    }
    return;
  }

  switch (message.type) {
    case 'init':
      await initializeTonk(message.manifest, message.wsUrl);
      break;

    case 'readFile':
      console.log('[Worker] Reading file:', message.path);
      try {
        const content = await tonk!.readFile(message.path);
        // Return the content as JSON string
        const contentStr = JSON.stringify(content);

        postResponse({
          type: 'readFile',
          id: message.id,
          success: true,
          data: contentStr,
        });
      } catch (error) {
        console.error('[Worker] Failed to read file:', error);
        postResponse({
          type: 'readFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'writeFile':
      console.log('[Worker] Writing file:', message.path);
      try {
        // Parse the content string back to JSON
        const content = JSON.parse(message.content);

        if (message.create) {
          await tonk!.createFile(message.path, content);
        } else {
          await tonk!.updateFile(message.path, content);
        }

        postResponse({
          type: 'writeFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        console.error('[Worker] Failed to write file:', error);
        postResponse({
          type: 'writeFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'deleteFile':
      console.log('[Worker] Deleting file:', message.path);
      try {
        await tonk!.deleteFile(message.path);
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        console.error('[Worker] Failed to delete file:', error);
        postResponse({
          type: 'deleteFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'listDirectory':
      console.log('[Worker] Listing directory:', message.path);
      try {
        const files = await tonk!.listDirectory(message.path);
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: true,
          data: files,
        });
      } catch (error) {
        console.error('[Worker] Failed to list directory:', error);
        postResponse({
          type: 'listDirectory',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'exists':
      console.log('[Worker] Checking existence:', message.path);
      try {
        const exists = await tonk!.exists(message.path);
        postResponse({
          type: 'exists',
          id: message.id,
          success: true,
          data: exists,
        });
      } catch (error) {
        console.error('[Worker] Failed to check existence:', error);
        postResponse({
          type: 'exists',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'watchFile':
      console.log('[Worker] Starting file watch:', message.path);
      try {
        const watcher = await tonk!.watchFile(message.path, (content: any) => {
          console.log('[Worker] File changed:', message.path);
          postResponse({
            type: 'fileChanged',
            watchId: message.id,
            content: JSON.stringify(content),
          });
        });

        watchers.set(message.id, watcher);
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        console.error('[Worker] Failed to watch file:', error);
        postResponse({
          type: 'watchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;

    case 'unwatchFile':
      console.log('[Worker] Stopping file watch:', message.id);
      try {
        const watcher = watchers.get(message.id);
        if (watcher) {
          watcher.stop();
          watchers.delete(message.id);
        }
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: true,
        });
      } catch (error) {
        console.error('[Worker] Failed to stop file watch:', error);
        postResponse({
          type: 'unwatchFile',
          id: message.id,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      break;
  }
}

// Worker startup
console.log('[Worker] Tonk worker started');

// Signal to main thread that worker is ready
self.postMessage({ type: 'ready' });

// Listen for messages from main thread
self.addEventListener('message', async event => {
  try {
    await handleMessage(event.data as VFSWorkerMessage);
  } catch (error) {
    console.error('[Worker] Error handling message:', error);
  }
});

// Export for TypeScript
export {};
