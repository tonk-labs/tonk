// Bookmark-name → entity-DID resolution against the worker's
// `/resolve/{name}` route. Shared by the picker (UI flow) and the
// postMessage listener (artifact-driven navigation) so both go
// through the same cache and error shape.

const cache = new Map<string, string>();

export async function resolveName(
  repo: string,
  branch: string,
  name: string,
): Promise<string> {
  const key = `${repo}::${branch}::${name}`;
  const cached = cache.get(key);
  if (cached) return cached;

  const url = `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(
    branch,
  )}/resolve/${encodeURIComponent(name)}`;
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(
      res.status === 404
        ? `No entity bookmarked as "${name}" on branch "${branch}"`
        : `resolve failed (${res.status})`,
    );
  }
  const body = (await res.json()) as { entity: string };
  cache.set(key, body.entity);
  return body.entity;
}
