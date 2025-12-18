import type { DocumentData } from '@tonk/core/slim';
import {
  addWatcher,
  getTonkForBundle,
  getWatcherEntry,
  removeWatcher,
  removeWatchersByClientId,
} from '../state';
import { logger } from '../utils/logging';
import { postResponse, postResponseToClientId } from '../utils/response';

export async function handleWatchFile(
  message: {
    id: string;
    path: string;
    launcherBundleId: string;
  },
  sourceClient: Client
): Promise<void> {
  logger.debug('Starting file watch', {
    path: message.path,
    watchId: message.id,
    launcherBundleId: message.launcherBundleId,
    clientId: sourceClient.id,
  });
  try {
    const tonkInstance = getTonkForBundle(message.launcherBundleId);
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    // Store the client ID so we can route callbacks to the correct client
    const clientId = sourceClient.id;

    const watcher = await tonkInstance.tonk.watchFile(
      message.path,
      async (documentData: DocumentData) => {
        logger.debug('File change detected', {
          watchId: message.id,
          path: message.path,
          clientId,
        });

        // Send notification only to the client that registered this watcher
        const delivered = await postResponseToClientId(
          {
            type: 'fileChanged',
            watchId: message.id,
            documentData: documentData,
          },
          clientId
        );

        // If client no longer exists, clean up all watchers for that client
        if (!delivered) {
          removeWatchersByClientId(message.launcherBundleId, clientId);
        }
      }
    );

    if (watcher) {
      addWatcher(message.launcherBundleId, message.id, watcher, clientId);
    }
    logger.debug('File watch started', {
      path: message.path,
      watchId: message.id,
    });
    postResponse(
      {
        type: 'watchFile',
        id: message.id,
        success: true,
      },
      sourceClient
    );
  } catch (error) {
    logger.error('Failed to start file watch', {
      path: message.path,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse(
      {
        type: 'watchFile',
        id: message.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      sourceClient
    );
  }
}

export async function handleUnwatchFile(
  message: {
    id: string;
    launcherBundleId: string;
  },
  sourceClient: Client
): Promise<void> {
  logger.debug('Stopping file watch', {
    watchId: message.id,
    launcherBundleId: message.launcherBundleId,
  });
  try {
    const entry = getWatcherEntry(message.launcherBundleId, message.id);
    if (entry) {
      logger.debug('Found watcher, stopping it', { watchId: message.id });
      removeWatcher(message.launcherBundleId, message.id);
      logger.debug('File watch stopped', { watchId: message.id });
    } else {
      logger.debug('No watcher found for ID', { watchId: message.id });
    }
    postResponse(
      {
        type: 'unwatchFile',
        id: message.id,
        success: true,
      },
      sourceClient
    );
  } catch (error) {
    logger.error('Failed to stop file watch', {
      watchId: message.id,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse(
      {
        type: 'unwatchFile',
        id: message.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      sourceClient
    );
  }
}

export async function handleWatchDirectory(
  message: {
    id: string;
    path: string;
    launcherBundleId: string;
  },
  sourceClient: Client
): Promise<void> {
  logger.debug('Starting directory watch', {
    path: message.path,
    watchId: message.id,
    launcherBundleId: message.launcherBundleId,
    clientId: sourceClient.id,
  });
  try {
    const tonkInstance = getTonkForBundle(message.launcherBundleId);
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    // Store the client ID so we can route callbacks to the correct client
    const clientId = sourceClient.id;

    const watcher = await tonkInstance.tonk.watchDirectory(
      message.path,
      async (changeData: unknown) => {
        logger.debug('Directory change detected', {
          watchId: message.id,
          path: message.path,
          clientId,
        });

        // Send notification only to the client that registered this watcher
        const delivered = await postResponseToClientId(
          {
            type: 'directoryChanged',
            watchId: message.id,
            path: message.path,
            changeData,
          },
          clientId
        );

        // If client no longer exists, clean up all watchers for that client
        if (!delivered) {
          removeWatchersByClientId(message.launcherBundleId, clientId);
        }
      }
    );

    if (watcher) {
      addWatcher(message.launcherBundleId, message.id, watcher, clientId);
    }
    logger.debug('Directory watch started', {
      path: message.path,
      watchId: message.id,
    });
    postResponse(
      {
        type: 'watchDirectory',
        id: message.id,
        success: true,
      },
      sourceClient
    );
  } catch (error) {
    logger.error('Failed to start directory watch', {
      path: message.path,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse(
      {
        type: 'watchDirectory',
        id: message.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      sourceClient
    );
  }
}

export async function handleUnwatchDirectory(
  message: {
    id: string;
    launcherBundleId: string;
  },
  sourceClient: Client
): Promise<void> {
  logger.debug('Stopping directory watch', {
    watchId: message.id,
    launcherBundleId: message.launcherBundleId,
  });
  try {
    const entry = getWatcherEntry(message.launcherBundleId, message.id);
    if (entry) {
      logger.debug('Found directory watcher, stopping it', {
        watchId: message.id,
      });
      removeWatcher(message.launcherBundleId, message.id);
      logger.debug('Directory watch stopped', { watchId: message.id });
    } else {
      logger.debug('No directory watcher found for ID', {
        watchId: message.id,
      });
    }
    postResponse(
      {
        type: 'unwatchDirectory',
        id: message.id,
        success: true,
      },
      sourceClient
    );
  } catch (error) {
    logger.error('Failed to stop directory watch', {
      watchId: message.id,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse(
      {
        type: 'unwatchDirectory',
        id: message.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      sourceClient
    );
  }
}
