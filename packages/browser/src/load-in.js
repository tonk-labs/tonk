// Simple script to receive file contents from Electron main process via IPC
import { configureSyncEngine, writeDoc } from '@tonk/keepsync';
import { BrowserWebSocketClientAdapter } from '@automerge/automerge-repo-network-websocket';
import { parseBundle } from '@tonk/spec';

console.log('Load-in script initialized');

function emitProgressEvent(message, percentage) {
  window.dispatchEvent(
    new CustomEvent('loading-progress', {
      detail: { message, percentage },
    })
  );
}

async function init() {
  const wsUrl = `http://localhost:7777/sync`;
  const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);

  const engine = await configureSyncEngine({
    url: `http://localhost:7777`,
    network: [wsAdapter],
  });
  await engine.whenReady();

  console.log('KeepSync Engine', engine);
  return engine;
}

// Listen for file data from main process
if (window.electronAPI && window.electronAPI.onFileReceived) {
  window.electronAPI.onFileReceived(fileData => {
    console.log('File received via IPC:', fileData.fileName);
    console.log('File path:', fileData.filePath);
    console.log(
      'File content length:',
      fileData.content ? fileData.content.length : 'undefined'
    );

    // Handle the received file data
    handleFileReceived(fileData);
  });
} else {
  console.error('Electron API or onFileReceived listener not available');
}

// Function to handle received file data
async function handleFileReceived(fileData) {
  if (fileData.success) {
    try {
      emitProgressEvent('Loading app bundle...', 10);

      // fileData.content is now a Uint8Array from IPC, convert to ArrayBuffer for parseBundle
      const arrayBuffer = fileData.content.buffer.slice(
        fileData.content.byteOffset,
        fileData.content.byteOffset + fileData.content.byteLength
      );
      console.log(
        'Parsing bundle from ArrayBuffer, size:',
        arrayBuffer.byteLength
      );

      emitProgressEvent('Parsing app bundle...', 25);
      const parsed = await parseBundle(arrayBuffer);
      console.log('Bundle parsed successfully:', parsed);

      // Get app name from manifest, fallback to filename
      const appName =
        parsed.manifest.name ||
        fileData.fileName.replace('.tonk', '') ||
        'unnamed-app';
      const appId = `app_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`;

      console.log(`Installing app: ${appName} with ID: ${appId}`);

      emitProgressEvent('Preparing app installation...', 40);

      // Create app metadata
      const appMetadata = {
        id: appId,
        name: appName,
        fileName: fileData.fileName,
        filePath: fileData.filePath,
        manifest: parsed.manifest,
        installedAt: Date.now(),
        lastAccessed: Date.now(),
      };

      emitProgressEvent('Syncing app files...', 50);

      // Sync files to Keepsync using apps/[app-name]/ structure
      const totalFiles = parsed.manifest.files.length;
      let syncedFiles = 0;

      const filePromises = parsed.manifest.files.map(async ({ path }) => {
        const data = await parsed.getFileData(path);
        const fileMetadata = parsed.getFile(path);
        if (data) {
          const text = new globalThis.TextDecoder().decode(data);
          const appPath = `/apps/${appName}${path}`;
          await writeDoc(appPath, {
            path: appPath,
            mimeType: fileMetadata.contentType,
            content: text,
          });
          console.log(`Synced: ${appPath}`);

          syncedFiles++;
          const syncProgress = 50 + (syncedFiles / totalFiles) * 40; // 50% to 90%
          emitProgressEvent(
            `Syncing files... (${syncedFiles}/${totalFiles})`,
            syncProgress
          );
        }
      });

      await Promise.all(filePromises);

      emitProgressEvent('Saving app metadata...', 95);

      // Save app metadata to IndexedDB (check if available)
      if (window.appStorage && window.appStorage.saveApp) {
        await window.appStorage.saveApp(appMetadata);
      }

      emitProgressEvent('Installation complete!', 100);

      // Small delay to show completion before hiding
      setTimeout(() => {
        // Dispatch event to notify the main UI
        window.dispatchEvent(
          new CustomEvent('app-installed', {
            detail: appMetadata,
          })
        );
      }, 500);

      console.log(`App "${appName}" installed successfully`);
    } catch (error) {
      console.error('Error parsing bundle:', error);
      window.currentFile = fileData;
      window.currentBundle = null;

      // Dispatch error event
      window.dispatchEvent(
        new CustomEvent('app-install-error', {
          detail: { error: error.message },
        })
      );
    }
  } else {
    console.error('Error in received file:', fileData.error);
    window.currentFile = null;
    window.currentBundle = null;

    // Dispatch error event
    window.dispatchEvent(
      new CustomEvent('app-install-error', {
        detail: { error: fileData.error },
      })
    );
  }
}

init();
