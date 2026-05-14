// Listing of published artifacts on a branch — used by the
// EntityPicker's autosuggest. An "artifact" here means an entity
// that has both a `dialog.meta/name` claim (so it has a label
// the user can recognise) *and* a `text/html` claim (so it
// renders inside an iframe). The intersection mirrors what
// `tmp/launcher.yaml` does inside the in-portal launcher app:
// without the HTML filter the list also surfaces concept
// attributes (`attribute`, `attribute/as`, …) and other named
// entities that aren't useful targets for a portal tile.
//
// Cached in-process so opening the picker on a freshly-rendered
// tile doesn't refetch on every keystroke.

export type Artifact = {
  name: string;
  entity: string;
};

type CacheEntry = { time: number; artifacts: Artifact[] };
const cache = new Map<string, CacheEntry>();
const TTL_MS = 5_000;

type ClaimRow = { the: string; of: string; is: unknown };

async function fetchClaims(repo: string, branch: string, attribute: string): Promise<ClaimRow[]> {
  const url =
    `/api/repository/${encodeURIComponent(repo)}` +
    `/branch/${encodeURIComponent(branch)}` +
    `/claim/select?the=${encodeURIComponent(attribute)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`claim select '${attribute}' failed (${res.status})`);
  const body = (await res.json()) as { claims: ClaimRow[] };
  return body.claims ?? [];
}

export async function listArtifacts(
  repo: string,
  branch: string = "main",
): Promise<Artifact[]> {
  const key = `${repo}::${branch}`;
  const now = Date.now();
  const cached = cache.get(key);
  if (cached && now - cached.time < TTL_MS) return cached.artifacts;

  // Two parallel selects: every name claim, every html claim.
  // We intersect on `of` so the returned list is only entities
  // that are both named *and* renderable as a tile body.
  const [names, htmls] = await Promise.all([
    fetchClaims(repo, branch, "dialog.meta/name"),
    fetchClaims(repo, branch, "text/html"),
  ]);

  const renderable = new Set(htmls.map((c) => c.of));
  const seen = new Set<string>();
  const artifacts: Artifact[] = [];
  for (const c of names) {
    if (!renderable.has(c.of)) continue;
    if (seen.has(c.of)) continue;
    if (typeof c.is !== "string") continue;
    seen.add(c.of);
    artifacts.push({ name: c.is, entity: c.of });
  }
  artifacts.sort((a, b) => a.name.localeCompare(b.name));

  cache.set(key, { time: now, artifacts });
  return artifacts;
}

export function invalidateArtifacts(repo?: string, branch?: string) {
  if (repo == null) {
    cache.clear();
    return;
  }
  cache.delete(`${repo}::${branch ?? "main"}`);
}
