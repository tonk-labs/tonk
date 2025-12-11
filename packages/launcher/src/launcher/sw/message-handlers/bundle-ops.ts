import type { Manifest } from '@tonk/core/slim';
import { Bundle } from '@tonk/core/slim';
import { getTonk } from '../state';
import { loadBundle } from '../tonk-lifecycle';
import { logger } from '../utils/logging';
import { postResponse } from '../utils/response';

declare const TONK_SERVER_URL: string;

export async function handleLoadBundle(message: {
  id?: string;
  bundleBytes: ArrayBuffer;
  serverUrl?: string;
  manifest?: Manifest;
}): Promise<void> {
  logger.debug('Loading new bundle', {
    byteLength: message.bundleBytes.byteLength,
    serverUrl: message.serverUrl,
    hasCachedManifest: !!message.manifest,
  });

  const serverUrl = message.serverUrl || TONK_SERVER_URL;
  const bundleBytes = new Uint8Array(message.bundleBytes);

  const result = await loadBundle(bundleBytes, serverUrl, message.id, message.manifest);

  postResponse({
    type: 'loadBundle',
    id: message.id,
    success: result.success,
    skipped: result.skipped,
    error: result.error,
  });
}

export async function handleToBytes(message: { id: string }): Promise<void> {
  logger.debug('Converting tonk to bytes');
  try {
    const tonkInstance = getTonk();
    if (!tonkInstance) {
      throw new Error('Tonk not initialized');
    }

    const bytes = await tonkInstance.tonk.toBytes();
    const rootId = tonkInstance.manifest.rootId;
    logger.debug('Tonk converted to bytes', {
      byteLength: bytes.length,
      rootId,
    });
    postResponse({
      type: 'toBytes',
      id: message.id,
      success: true,
      data: bytes,
      rootId,
    });
  } catch (error) {
    logger.error('Failed to convert tonk to bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'toBytes',
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function handleForkToBytes(message: { id: string }): Promise<void> {
  logger.debug('Forking tonk to bytes');
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

    logger.debug('Tonk forked to bytes', { byteLength: bytes.length, rootId });
    postResponse({
      type: 'forkToBytes',
      id: message.id,
      success: true,
      data: bytes,
      rootId,
    });
  } catch (error) {
    logger.error('Failed to fork tonk to bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
    postResponse({
      type: 'forkToBytes',
      id: message.id,
      success: false,
      error: error instanceof Error ? error.message : String(error),
    });
  }
}
