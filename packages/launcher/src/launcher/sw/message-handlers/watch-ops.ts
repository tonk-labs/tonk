import type { DocumentData } from '@tonk/core/slim';
import { addWatcher, getTonk, getWatcher, removeWatcher } from '../state';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';

export async function handleWatchFile(message: { id: string; path: string }): Promise<void> {
  logger.debug('Starting file watch', {
    path: message.path,
    watchId: message.id,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const watcher = await tonkInstance.tonk.watchFile(
      message.path,
      (documentData: DocumentData) => {
        logger.debug('File change detected', {
          watchId: message.id,
          path: message.path,
        });

        postResponse({
          type: 'fileChanged',
          watchId: message.id,
          documentData: documentData,
        });
      }
    );

    if (watcher) {
      addWatcher(message.id, watcher);
    }
    logger.debug('File watch started', {
      path: message.path,
      watchId: message.id,
    });
    postResponse({
      type: 'watchFile',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to start file watch', {
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
}

export async function handleUnwatchFile(message: { id: string }): Promise<void> {
  logger.debug('Stopping file watch', { watchId: message.id });
  try {
    const watcher = getWatcher(message.id);
    if (watcher) {
      logger.debug('Found watcher, stopping it', { watchId: message.id });
      removeWatcher(message.id);
      logger.debug('File watch stopped', { watchId: message.id });
    } else {
      logger.debug('No watcher found for ID', { watchId: message.id });
    }
    postResponse({
      type: 'unwatchFile',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to stop file watch', {
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
}

export async function handleWatchDirectory(message: { id: string; path: string }): Promise<void> {
  logger.debug('Starting directory watch', {
    path: message.path,
    watchId: message.id,
  });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const watcher = await tonkInstance.tonk.watchDirectory(message.path, (changeData: unknown) => {
      logger.debug('Directory change detected', {
        watchId: message.id,
        path: message.path,
      });

      postResponse({
        type: 'directoryChanged',
        watchId: message.id,
        path: message.path,
        changeData,
      });
    });

    if (watcher) {
      addWatcher(message.id, watcher);
    }
    logger.debug('Directory watch started', {
      path: message.path,
      watchId: message.id,
    });
    postResponse({
      type: 'watchDirectory',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to start directory watch', {
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
}

export async function handleUnwatchDirectory(message: { id: string }): Promise<void> {
  logger.debug('Stopping directory watch', { watchId: message.id });
  try {
    const watcher = getWatcher(message.id);
    if (watcher) {
      logger.debug('Found directory watcher, stopping it', {
        watchId: message.id,
      });
      removeWatcher(message.id);
      logger.debug('Directory watch stopped', { watchId: message.id });
    } else {
      logger.debug('No directory watcher found for ID', {
        watchId: message.id,
      });
    }
    postResponse({
      type: 'unwatchDirectory',
      id: message.id,
      success: true,
    });
  } catch (error) {
    logger.error('Failed to stop directory watch', {
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
}
