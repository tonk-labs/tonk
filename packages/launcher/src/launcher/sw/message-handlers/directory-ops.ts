import { getTonkForBundle } from '../state';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';

export async function handleListDirectory(
  message: {
    id: string;
    path: string;
    launcherBundleId: string;
  },
  sourceClient: Client
): Promise<void> {
  logger.debug('Listing directory', {
    path: message.path,
    launcherBundleId: message.launcherBundleId,
  });
  try {
    const tonkInstance = getTonkForBundle(message.launcherBundleId);
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const files = await tonkInstance.tonk.listDirectory(message.path);
    logger.debug('Directory listed', {
      path: message.path,
      fileCount: Array.isArray(files) ? files.length : 'unknown',
    });
    // Pass through the raw response from TonkCore
    postResponse(
      {
        type: 'listDirectory',
        id: message.id,
        success: true,
        data: files,
      },
      sourceClient
    );
  } catch (error) {
    logger.error('Failed to list directory', {
      path: message.path,
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse(
      {
        type: 'listDirectory',
        id: message.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
      },
      sourceClient
    );
  }
}
