declare const TONK_SERVER_URL: string;

/**
 * Gets the WebSocket URL from manifest networkUris or falls back to provided/default URL
 */
export function getWsUrlFromManifest(
  manifest: { networkUris?: string[] },
  fallbackUrl: string = TONK_SERVER_URL
): string {
  if (manifest.networkUris && manifest.networkUris.length > 0) {
    const networkUri = manifest.networkUris[0];
    return networkUri.replace(/^http/, 'ws');
  }
  return fallbackUrl.replace(/^http/, 'ws');
}
