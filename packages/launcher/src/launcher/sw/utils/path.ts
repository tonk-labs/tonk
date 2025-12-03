// Determine VFS path from URL
export function determinePath(url: URL, appSlug: string | null): string {
  console.log('determinePath START', {
    url: url.href,
    pathname: url.pathname,
    appSlug: appSlug || 'none',
  });

  // If no appSlug is set, we can't determine the path
  if (!appSlug) {
    console.error('determinePath - NO APP SLUG SET');
    throw new Error(`No app slug available for ${url.pathname}`);
  }

  // Strip the scope from the pathname
  const scopePath = new URL(
    // @ts-expect-error - self.registration exists in ServiceWorkerGlobalScope
    (self.registration?.scope ?? self.location.href) as string
  ).pathname;
  const strippedPath = url.pathname.startsWith(scopePath)
    ? url.pathname.slice(scopePath.length)
    : url.pathname;

  // Remove leading slashes and split into segments
  const segments = strippedPath.replace(/^\/+/, '').split('/').filter(Boolean);

  console.log('determinePath - segments', {
    scopePath,
    strippedPath,
    segments: [...segments],
    firstSegment: segments[0] || 'none',
  });

  // Check if the appSlug is already at the start
  let pathSegments = segments;
  if (segments[0] === appSlug) {
    // AppSlug is already present, remove it from segments
    pathSegments = segments.slice(1);
    console.log('determinePath - appSlug already present, using remaining segments', {
      pathSegments: [...pathSegments],
    });
  } else {
    console.log('determinePath - appSlug not present, using all segments as path', {
      pathSegments: [...pathSegments],
    });
  }

  // If no segments left or path ends with slash, default to index.html
  if (pathSegments.length === 0 || url.pathname.endsWith('/')) {
    const result = `${appSlug}/index.html`;
    console.log('determinePath - defaulting to index.html', { result });
    return result;
  }

  // Regular file path
  const result = `${appSlug}/${pathSegments.join('/')}`;
  console.log('determinePath - returning file path', { result });
  return result;
}
