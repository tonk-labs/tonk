// Parse URL to extract launcherBundleId and appSlug
// URL structure: /space/<launcherBundleId>/<appSlug>/path/to/file
export interface ParsedSpaceUrl {
  launcherBundleId: string;
  appSlug: string;
  remainingPath: string;
}

export function parseSpaceUrl(pathname: string): ParsedSpaceUrl | null {
  // Match /space/<launcherBundleId>/<appSlug>/... or /space/<launcherBundleId>/<appSlug>
  const match = pathname.match(/^\/space\/([^/]+)\/([^/]+)(\/.*)?$/);

  if (!match) {
    return null;
  }

  return {
    launcherBundleId: match[1],
    appSlug: match[2],
    remainingPath: match[3] || "/",
  };
}

// Determine VFS path from URL
// URL structure: /space/<launcherBundleId>/<appSlug>/path/to/file
// VFS path structure: <appSlug>/path/to/file
export function determinePath(url: URL, appSlug: string | null): string {
  console.log("determinePath START", {
    url: url.href,
    pathname: url.pathname,
    appSlug: appSlug || "none",
  });

  // If no appSlug is set, we can't determine the path
  if (!appSlug) {
    console.error("determinePath - NO APP SLUG SET");
    throw new Error(`No app slug available for ${url.pathname}`);
  }

  // Strip the scope (/space/) from the pathname
  const scopePath = new URL(
    // @ts-expect-error - self.registration exists in ServiceWorkerGlobalScope
    (self.registration?.scope ?? self.location.href) as string,
  ).pathname;
  const strippedPath = url.pathname.startsWith(scopePath)
    ? url.pathname.slice(scopePath.length)
    : url.pathname;

  // Remove leading slashes and split into segments
  // After stripping /space/, we get <launcherBundleId>/<appSlug>/path/to/file
  const segments = strippedPath.replace(/^\/+/, "").split("/").filter(Boolean);

  console.log("determinePath - segments", {
    scopePath,
    strippedPath,
    segments: [...segments],
    firstSegment: segments[0] || "none",
  });

  // New URL structure: /space/<launcherBundleId>/<appSlug>/...
  // segments[0] = launcherBundleId
  // segments[1] = appSlug
  // segments[2+] = file path within the app

  // Skip launcherBundleId (first segment) and get remaining path
  let pathSegments: string[];
  if (segments.length >= 2 && segments[1] === appSlug) {
    // launcherBundleId is first, appSlug is second - skip first, keep rest
    pathSegments = segments.slice(2);
    console.log(
      "determinePath - new URL structure detected, using path after appSlug",
      {
        launcherBundleId: segments[0],
        appSlug: segments[1],
        pathSegments: [...pathSegments],
      },
    );
  } else if (segments[0] === appSlug) {
    // Old URL structure fallback: /space/<appSlug>/...
    pathSegments = segments.slice(1);
    console.log("determinePath - old URL structure, using path after appSlug", {
      appSlug: segments[0],
      pathSegments: [...pathSegments],
    });
  } else {
    // Unknown structure, use all segments as path
    pathSegments = segments;
    console.log(
      "determinePath - unknown structure, using all segments as path",
      {
        pathSegments: [...pathSegments],
      },
    );
  }

  // If no segments left or path ends with slash, default to index.html
  if (pathSegments.length === 0 || url.pathname.endsWith("/")) {
    const result = `${appSlug}/index.html`;
    console.log("determinePath - defaulting to index.html", { result });
    return result;
  }

  // Regular file path
  const result = `${appSlug}/${pathSegments.join("/")}`;
  console.log("determinePath - returning file path", { result });
  return result;
}
