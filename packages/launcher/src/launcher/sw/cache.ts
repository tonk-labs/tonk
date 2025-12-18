import { logger } from './utils/logging';

// Cache API constants for persisting state across service worker restarts
// v3: Updated for multi-bundle isolation with /space/<launcherBundleId>/<appSlug>/ URL structure
export const CACHE_NAME = 'tonk-sw-state-v3';
export const APP_SLUG_URL = '/tonk-state/appSlug';
export const BUNDLE_BYTES_URL = '/tonk-state/bundleBytes';
export const WS_URL_KEY = '/tonk-state/wsUrl';
export const NAMESPACE_KEY = '/tonk-state/namespace';
export const LAST_ACTIVE_BUNDLE_ID_KEY = '/tonk-state/lastActiveBundleId';

export async function persistAppSlug(slug: string | null): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (slug === null) {
      await cache.delete(APP_SLUG_URL);
    } else {
      const response = new Response(JSON.stringify({ slug }), {
        headers: { 'Content-Type': 'application/json' },
      });
      await cache.put(APP_SLUG_URL, response);
    }

    logger.debug('AppSlug persisted to cache', { slug });
  } catch (error) {
    logger.error('Failed to persist appSlug', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function restoreAppSlug(): Promise<string | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(APP_SLUG_URL);

    if (!response) {
      return null;
    }

    const data = await response.json();
    return data.slug || null;
  } catch (error) {
    logger.error('Failed to restore appSlug', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

export async function persistBundleBytes(
  bytes: Uint8Array | null
): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (bytes === null) {
      await cache.delete(BUNDLE_BYTES_URL);
    } else {
      // Convert Uint8Array to Blob for Response
      const blob = new Blob([bytes as unknown as BlobPart], {
        type: 'application/octet-stream',
      });
      const response = new Response(blob, {
        headers: { 'Content-Type': 'application/octet-stream' },
      });
      await cache.put(BUNDLE_BYTES_URL, response);
    }

    logger.debug('Bundle bytes persisted to cache', {
      size: bytes ? bytes.length : 0,
    });
  } catch (error) {
    logger.error('Failed to persist bundle bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function restoreBundleBytes(): Promise<Uint8Array | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(BUNDLE_BYTES_URL);

    if (!response) {
      return null;
    }

    const arrayBuffer = await response.arrayBuffer();
    return new Uint8Array(arrayBuffer);
  } catch (error) {
    logger.error('Failed to restore bundle bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

export async function persistWsUrl(url: string | null): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (url === null) {
      await cache.delete(WS_URL_KEY);
    } else {
      const response = new Response(JSON.stringify({ url }), {
        headers: { 'Content-Type': 'application/json' },
      });
      await cache.put(WS_URL_KEY, response);
    }

    logger.debug('WS URL persisted to cache', { url });
  } catch (error) {
    logger.error('Failed to persist WS URL', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function restoreWsUrl(): Promise<string | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(WS_URL_KEY);

    if (!response) {
      return null;
    }

    const data = await response.json();
    return data.url || null;
  } catch (error) {
    logger.error('Failed to restore WS URL', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

export async function persistNamespace(
  namespace: string | null
): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (namespace === null) {
      await cache.delete(NAMESPACE_KEY);
    } else {
      const response = new Response(JSON.stringify({ namespace }), {
        headers: { 'Content-Type': 'application/json' },
      });
      await cache.put(NAMESPACE_KEY, response);
    }

    logger.debug('Namespace persisted to cache', { namespace });
  } catch (error) {
    logger.error('Failed to persist namespace', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

export async function restoreNamespace(): Promise<string | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(NAMESPACE_KEY);

    if (!response) {
      return null;
    }

    const data = await response.json();
    return data.namespace || null;
  } catch (error) {
    logger.error('Failed to restore namespace', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

// Persist last active bundle ID
export async function persistLastActiveBundleId(
  id: string | null
): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (id === null) {
      await cache.delete(LAST_ACTIVE_BUNDLE_ID_KEY);
    } else {
      const response = new Response(JSON.stringify({ id }), {
        headers: { 'Content-Type': 'application/json' },
      });
      await cache.put(LAST_ACTIVE_BUNDLE_ID_KEY, response);
    }

    logger.debug('Last active bundle ID persisted to cache', { id });
  } catch (error) {
    logger.error('Failed to persist last active bundle ID', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

// Restore last active bundle ID
export async function restoreLastActiveBundleId(): Promise<string | null> {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(LAST_ACTIVE_BUNDLE_ID_KEY);

    if (!response) {
      return null;
    }

    const data = await response.json();
    return data.id || null;
  } catch (error) {
    logger.error('Failed to restore last active bundle ID', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

// Clear all cached state
export async function clearAllCache(): Promise<void> {
  await Promise.all([
    persistAppSlug(null),
    persistBundleBytes(null),
    persistWsUrl(null),
    persistNamespace(null),
    persistLastActiveBundleId(null),
  ]);
}
