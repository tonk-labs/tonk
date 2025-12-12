// Determine VFS path from URL
// URL structure: /space/<space-name>/path/to/file
// VFS path structure: <space-name>/path/to/file
export function determinePath(url: URL, spaceName: string | null): string {
  console.log("determinePath START", {
    url: url.href,
    pathname: url.pathname,
    spaceName: spaceName || "none",
  });

  // If no spaceName is set, we can't determine the path
  if (!spaceName) {
    console.error("determinePath - NO SPACE NAME SET");
    throw new Error(`No space name available for ${url.pathname}`);
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
  // After stripping /space/, we get <space-name>/path/to/file
  const segments = strippedPath.replace(/^\/+/, "").split("/").filter(Boolean);

  console.log("determinePath - segments", {
    scopePath,
    strippedPath,
    segments: [...segments],
    firstSegment: segments[0] || "none",
  });

  // Check if the spaceName is already at the start (it should be for /space/<space-name>/... URLs)
  let pathSegments = segments;
  if (segments[0] === spaceName) {
    // SpaceName is already present, remove it from segments
    pathSegments = segments.slice(1);
    console.log(
      "determinePath - spaceName already present, using remaining segments",
      {
        pathSegments: [...pathSegments],
      },
    );
  } else {
    console.log(
      "determinePath - spaceName not present, using all segments as path",
      {
        pathSegments: [...pathSegments],
      },
    );
  }

  // If no segments left or path ends with slash, default to index.html
  if (pathSegments.length === 0 || url.pathname.endsWith("/")) {
    const result = `${spaceName}/index.html`;
    console.log("determinePath - defaulting to index.html", { result });
    return result;
  }

  // Regular file path
  const result = `${spaceName}/${pathSegments.join("/")}`;
  console.log("determinePath - returning file path", { result });
  return result;
}
