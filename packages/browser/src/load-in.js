// Simple script to receive file contents from Electron main process via IPC
import { configureSyncEngine, writeDoc, readDoc, ls, rm } from '@tonk/keepsync';
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

let syncEngine = null;
async function init() {
  const wsUrl = `http://localhost:7777/sync`;
  const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);

  syncEngine = await configureSyncEngine({
    url: `http://localhost:7777`,
    network: [wsAdapter],
  });
  await syncEngine.whenReady();

  console.log('KeepSync Engine', syncEngine);
  return syncEngine;
}

// Function to restore apps from keepsync
async function restoreAppsFromKeepsync() {
  if (!syncEngine) {
    console.warn('KeepSync engine not initialized, cannot restore apps');
    return [];
  }

  try {
    console.log('Restoring apps from keepsync...');

    // List all directories under /apps/
    const appsDir = await ls('/apps/');
    if (!appsDir || !appsDir.children) {
      console.log('No apps found in keepsync');
      return [];
    }

    const restoredApps = [];

    // Process each app directory
    for (const appRef of appsDir.children) {
      if (appRef.type !== 'dir') continue;

      const appName = appRef.name;

      try {
        // Try to read app metadata
        const metadataPath = `/apps/${appName}/.metadata`;
        const metadata = await readDoc(metadataPath);

        if (metadata) {
          console.log(`Restored app: ${appName}`, metadata);
          restoredApps.push(metadata);
        } else {
          // If no metadata exists, create basic metadata from directory
          console.log(`Creating basic metadata for app: ${appName}`);
          const basicMetadata = {
            id: `app_restored_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`,
            name: appName,
            fileName: `${appName}.tonk`,
            filePath: 'restored from keepsync',
            manifest: { name: appName },
            installedAt: Date.now(),
            lastAccessed: Date.now(),
            restoredFromKeepsync: true,
          };

          // Save the basic metadata to keepsync
          await writeDoc(metadataPath, basicMetadata);
          restoredApps.push(basicMetadata);
        }
      } catch (error) {
        console.error(`Error restoring app ${appName}:`, error);
      }
    }

    console.log(`Restored ${restoredApps.length} apps from keepsync`);
    return restoredApps.sort(
      (a, b) => (b.lastAccessed || 0) - (a.lastAccessed || 0)
    );
  } catch (error) {
    console.error('Error restoring apps from keepsync:', error);
    return [];
  }
}

// Function to save app metadata to keepsync
async function saveAppMetadataToKeepsync(appMetadata) {
  if (!syncEngine) {
    console.warn('KeepSync engine not initialized, cannot save app metadata');
    return;
  }

  try {
    const metadataPath = `/apps/${appMetadata.name}/.metadata`;
    await writeDoc(metadataPath, appMetadata);
    console.log(`Saved app metadata to: ${metadataPath}`);
  } catch (error) {
    console.error('Error saving app metadata to keepsync:', error);
  }
}

// Function to update app last accessed time
async function updateAppLastAccessed(appName) {
  if (!syncEngine) return;

  try {
    const metadataPath = `/apps/${appName}/.metadata`;
    const metadata = await readDoc(metadataPath);
    if (metadata) {
      metadata.lastAccessed = Date.now();
      await writeDoc(metadataPath, metadata);
    }
  } catch (error) {
    console.error('Error updating app last accessed time:', error);
  }
}

// Function to delete app from keepsync
async function deleteAppFromKeepsync(appName) {
  if (!syncEngine) return;

  try {
    // Delete the entire app directory using rm
    const appPath = `/apps/${appName}`;
    const deleted = await rm(appPath);

    if (deleted) {
      console.log(`Deleted app directory: ${appPath}`);
    } else {
      console.warn(`App directory not found: ${appPath}`);
    }
  } catch (error) {
    console.error('Error deleting app from keepsync:', error);
  }
}

// Make functions globally available
window.restoreAppsFromKeepsync = restoreAppsFromKeepsync;
window.saveAppMetadataToKeepsync = saveAppMetadataToKeepsync;
window.updateAppLastAccessed = updateAppLastAccessed;
window.deleteAppFromKeepsync = deleteAppFromKeepsync;

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

      // Save app metadata to keepsync
      await saveAppMetadataToKeepsync(appMetadata);

      // Also save to IndexedDB as fallback/cache (check if available)
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
