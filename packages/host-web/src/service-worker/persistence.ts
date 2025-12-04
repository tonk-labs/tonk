import { log } from './logging';
import { CACHE_NAME, APP_SLUG_URL, BUNDLE_BYTES_URL } from './constants';

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

    log('info', 'AppSlug persisted to cache', { slug });
  } catch (error) {
    log('error', 'Failed to persist appSlug', {
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
    log('error', 'Failed to restore appSlug', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

export async function persistBundleBytes(bytes: Uint8Array | null): Promise<void> {
  try {
    const cache = await caches.open(CACHE_NAME);

    if (bytes === null) {
      await cache.delete(BUNDLE_BYTES_URL);
    } else {
      // Convert Uint8Array to Blob for Response
      const blob = new Blob([bytes], {
        type: 'application/octet-stream',
      });
      const response = new Response(blob, {
        headers: { 'Content-Type': 'application/octet-stream' },
      });
      await cache.put(BUNDLE_BYTES_URL, response);
    }

    log('info', 'Bundle bytes persisted to cache', {
      size: bytes ? bytes.length : 0,
    });
  } catch (error) {
    log('error', 'Failed to persist bundle bytes', {
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
    log('error', 'Failed to restore bundle bytes', {
      error: error instanceof Error ? error.message : String(error),
    });
    return null;
  }
}

export async function clearPersistedState(): Promise<void> {
  await Promise.all([persistAppSlug(null), persistBundleBytes(null)]);
}
