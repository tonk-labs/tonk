import { Bundle, DocumentData } from '@tonk/core/slim';
import { log } from './logging';
import { getTonk, addWatcher, removeWatcher, getWatcher } from './state';
import { postResponse } from './message-utils';
import type { DocumentContent } from './types';

// Read file handler
export async function handleReadFile(id: string, path: string): Promise<void> {
  log('info', 'Reading file', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const documentData = await tonkInstance.tonk.readFile(path);
    log('info', 'File read successfully', {
      path,
      documentType: documentData.type,
      hasBytes: !!documentData.bytes,
      contentType: typeof documentData.content,
    });

    await postResponse({
      type: 'readFile',
      id,
      success: true,
      data: documentData,
    });
  } catch (error) {
    log('error', 'Failed to read file', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'readFile',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Write file handler
export async function handleWriteFile(
  id: string,
  path: string,
  content: DocumentContent,
  create: boolean
): Promise<void> {
  log('info', 'Writing file', {
    path,
    id,
    create,
    hasBytes: !!content.bytes,
    contentType: typeof content.content,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    if (create) {
      log('info', 'Creating new file', { path });
      if (content.bytes) {
        await tonkInstance.tonk.createFileWithBytes(
          path,
          content.content,
          content.bytes
        );
      } else {
        await tonkInstance.tonk.createFile(path, content.content);
      }
    } else {
      log('info', 'Updating existing file', { path });
      if (content.bytes) {
        await tonkInstance.tonk.updateFileWithBytes(
          path,
          content.content,
          content.bytes
        );
      } else {
        await tonkInstance.tonk.updateFile(path, content.content);
      }
    }

    log('info', 'File write completed successfully', { path });
    await postResponse({
      type: 'writeFile',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to write file', {
      path,
      create,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'writeFile',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Delete file handler
export async function handleDeleteFile(id: string, path: string): Promise<void> {
  log('info', 'Deleting file', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    await tonkInstance.tonk.deleteFile(path);
    log('info', 'File deleted successfully', { path });
    await postResponse({
      type: 'deleteFile',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to delete file', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'deleteFile',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Rename handler
export async function handleRename(
  id: string,
  oldPath: string,
  newPath: string
): Promise<void> {
  log('info', 'Renaming file or directory', { oldPath, newPath, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    await tonkInstance.tonk.rename(oldPath, newPath);
    log('info', 'Rename completed successfully', { oldPath, newPath });
    await postResponse({
      type: 'rename',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to rename', {
      oldPath,
      newPath,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'rename',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// List directory handler
export async function handleListDirectory(id: string, path: string): Promise<void> {
  log('info', 'Listing directory', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const files = await tonkInstance.tonk.listDirectory(path);
    log('info', 'Directory listed successfully', {
      path,
      fileCount: Array.isArray(files) ? files.length : 'unknown',
    });
    await postResponse({
      type: 'listDirectory',
      id,
      success: true,
      data: files,
    });
  } catch (error) {
    log('error', 'Failed to list directory', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'listDirectory',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Exists handler
export async function handleExists(id: string, path: string): Promise<void> {
  log('info', 'Checking file existence', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const exists = await tonkInstance.tonk.exists(path);
    log('info', 'File existence check completed', { path, exists });
    await postResponse({
      type: 'exists',
      id,
      success: true,
      data: exists,
    });
  } catch (error) {
    log('error', 'Failed to check file existence', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'exists',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Watch file handler
export async function handleWatchFile(id: string, path: string): Promise<void> {
  log('info', 'Starting file watch', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const watcher = await tonkInstance.tonk.watchFile(
      path,
      (documentData: DocumentData) => {
        log('info', 'File change detected', {
          watchId: id,
          path,
          documentType: documentData.type,
          hasBytes: !!documentData.bytes,
        });

        postResponse({
          type: 'fileChanged',
          watchId: id,
          documentData,
        });
      }
    );

    addWatcher(id, watcher!);
    log('info', 'File watch started successfully', {
      path,
      watchId: id,
    });
    await postResponse({
      type: 'watchFile',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to start file watch', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'watchFile',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Unwatch file handler
export async function handleUnwatchFile(id: string): Promise<void> {
  log('info', 'Stopping file watch', { watchId: id });
  try {
    const watcher = getWatcher(id);
    if (watcher) {
      log('info', 'Found watcher, stopping it', { watchId: id });
      watcher.stop();
      removeWatcher(id);
      log('info', 'File watch stopped successfully', { watchId: id });
    } else {
      log('warn', 'No watcher found for ID', { watchId: id });
    }
    await postResponse({
      type: 'unwatchFile',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to stop file watch', {
      watchId: id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'unwatchFile',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Watch directory handler
export async function handleWatchDirectory(id: string, path: string): Promise<void> {
  log('info', 'Starting directory watch', { path, id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const watcher = await tonkInstance.tonk.watchDirectory(
      path,
      (changeData: unknown) => {
        log('info', 'Directory change detected', { watchId: id, path });

        postResponse({
          type: 'directoryChanged',
          watchId: id,
          path,
          changeData,
        });
      }
    );

    addWatcher(id, watcher!);
    log('info', 'Directory watch started successfully', {
      path,
      watchId: id,
    });
    await postResponse({
      type: 'watchDirectory',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to start directory watch', {
      path,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'watchDirectory',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Unwatch directory handler
export async function handleUnwatchDirectory(id: string): Promise<void> {
  log('info', 'Stopping directory watch', { watchId: id });
  try {
    const watcher = getWatcher(id);
    if (watcher) {
      log('info', 'Found directory watcher, stopping it', { watchId: id });
      watcher.stop();
      removeWatcher(id);
      log('info', 'Directory watch stopped successfully', { watchId: id });
    } else {
      log('warn', 'No directory watcher found for ID', { watchId: id });
    }
    await postResponse({
      type: 'unwatchDirectory',
      id,
      success: true,
    });
  } catch (error) {
    log('error', 'Failed to stop directory watch', {
      watchId: id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'unwatchDirectory',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// ToBytes handler
export async function handleToBytes(id: string): Promise<void> {
  log('info', 'Converting tonk to bytes', { id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const bytes = await tonkInstance.tonk.toBytes();
    const rootId = tonkInstance.manifest.rootId;
    log('info', 'Tonk converted to bytes successfully', {
      id,
      byteLength: bytes.length,
      rootId,
    });
    await postResponse({
      type: 'toBytes',
      id,
      success: true,
      data: bytes,
      rootId,
    });
  } catch (error) {
    log('error', 'Failed to convert tonk to bytes', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'toBytes',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// ForkToBytes handler
export async function handleForkToBytes(id: string): Promise<void> {
  log('info', 'Forking tonk to bytes', { id });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const bytes = await tonkInstance.tonk.forkToBytes();

    // Create a new bundle from the forked bytes to get the new rootId
    const forkedBundle = await Bundle.fromBytes(bytes);
    const forkedManifest = await forkedBundle.getManifest();
    const rootId = forkedManifest.rootId;

    log('info', 'Tonk forked to bytes successfully', {
      id,
      byteLength: bytes.length,
      rootId,
    });
    await postResponse({
      type: 'forkToBytes',
      id,
      success: true,
      data: bytes,
      rootId,
    });
  } catch (error) {
    log('error', 'Failed to fork tonk to bytes', {
      id,
      error: error instanceof Error ? error.message : String(error),
    });
    await postResponse({
      type: 'forkToBytes',
      id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}
