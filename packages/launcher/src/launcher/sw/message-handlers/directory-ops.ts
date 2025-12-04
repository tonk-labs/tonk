import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';
import { getTonk } from '../state';

export async function handleListDirectory(message: { id: string; path: string }): Promise<void> {
  logger.debug('Listing directory', { path: message.path });
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const files = await tonkInstance.tonk.listDirectory(message.path);
    logger.debug('Directory listed', {
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
    logger.error('Failed to list directory', {
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
}
