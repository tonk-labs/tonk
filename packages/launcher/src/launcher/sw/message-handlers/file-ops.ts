import { getTonk } from '../state';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';

export async function handleReadFile(message: {
  id: string;
  path: string;
}): Promise<void> {
  logger.debug('Reading file', { path: message.path });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const documentData = await tonkInstance.tonk.readFile(message.path);
    logger.debug('File read successfully', { path: message.path });

    postResponse({
      type: 'readFile',
      id: message.id,
      success: true,
      data: documentData,
    });
  } catch (error) {
    logger.error('Failed to read file', {
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
}

export async function handleWriteFile(message: {
  id: string;
  path: string;
  create?: boolean;
  content: {
    bytes?: Uint8Array;
    content: unknown;
  };
}): Promise<void> {
  logger.debug('Writing file', {
    path: message.path,
    create: message.create,
    hasBytes: !!message.content.bytes,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    // Cast content to any to satisfy TonkCore's JsonValue type
    const content = message.content.content as Parameters<
      typeof tonkInstance.tonk.createFile
    >[1];

    if (message.create) {
      logger.debug('Creating new file', { path: message.path });
      if (message.content.bytes) {
        // Create file with bytes
        await tonkInstance.tonk.createFileWithBytes(
          message.path,
          content,
          message.content.bytes
        );
      } else {
        // Create file with content only
        await tonkInstance.tonk.createFile(message.path, content);
      }
    } else {
      logger.debug('Setting existing file', { path: message.path });
      if (message.content.bytes) {
        // Set file with bytes
        await tonkInstance.tonk.setFileWithBytes(
          message.path,
          content,
          message.content.bytes
        );
      } else {
        // Set file with content only
        await tonkInstance.tonk.setFile(message.path, content);
      }
    }
    logger.debug('File write completed', { path: message.path });
    postResponse({
      type: 'writeFile',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to write file', {
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
}

export async function handleDeleteFile(message: {
  id: string;
  path: string;
}): Promise<void> {
  logger.debug('Deleting file', { path: message.path });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    await tonkInstance.tonk.deleteFile(message.path);
    logger.debug('File deleted successfully', { path: message.path });
    postResponse({
      type: 'deleteFile',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to delete file', {
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
}

export async function handleRename(message: {
  id: string;
  oldPath: string;
  newPath: string;
}): Promise<void> {
  logger.debug('Renaming file or directory', {
    oldPath: message.oldPath,
    newPath: message.newPath,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    await tonkInstance.tonk.rename(message.oldPath, message.newPath);
    logger.debug('Rename completed', {
      oldPath: message.oldPath,
      newPath: message.newPath,
    });
    postResponse({
      type: 'rename',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to rename', {
      oldPath: message.oldPath,
      newPath: message.newPath,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'rename',
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handleExists(message: {
  id: string;
  path: string;
}): Promise<void> {
  logger.debug('Checking file existence', { path: message.path });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const exists = await tonkInstance.tonk.exists(message.path);
    logger.debug('File existence check completed', {
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
    logger.error('Failed to check file existence', {
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
}

export async function handleUpdateFile(message: {
  id: string;
  path: string;
  content: unknown;
}): Promise<void> {
  logger.debug('Updating file with smart diff', { path: message.path });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    // Cast content to any to satisfy TonkCore's JsonValue type
    const content = message.content as Parameters<
      typeof tonkInstance.tonk.updateFile
    >[1];

    const result = await tonkInstance.tonk.updateFile(message.path, content);
    logger.debug('File update completed', {
      path: message.path,
      changed: result,
    });
    postResponse({
      type: 'updateFile',
      id: message.id,
      success: true,
      data: result,
    });
  } catch (error) {
    logger.error('Failed to update file', {
      path: message.path,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'updateFile',
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handlePatchFile(message: {
  id: string;
  path: string;
  jsonPath: string[];
  value: unknown;
}): Promise<void> {
  logger.debug('Patching file', {
    path: message.path,
    jsonPath: message.jsonPath,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    // Cast value to the expected type for patchFile
    const value = message.value as Parameters<
      typeof tonkInstance.tonk.patchFile
    >[2];

    const result = await tonkInstance.tonk.patchFile(
      message.path,
      message.jsonPath,
      value
    );

    logger.debug('File patch completed', { path: message.path, result });
    postResponse({
      type: 'patchFile',
      id: message.id,
      success: true,
      data: result,
    });
  } catch (error) {
    logger.error('Failed to patch file', {
      path: message.path,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'patchFile',
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}
