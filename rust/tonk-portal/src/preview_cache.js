// Called only by the authenticated portal dispatcher. `context` and `host`
// come from the host, never the request. Nested hosts repeat this check.
const KEY = 'tonk-space-previews-v1';
const MAX_IMAGE = 48000;
const TTL = 24 * 60 * 60 * 1000;
let scope;
let entries = {};

export async function previewRequest(request, context, host) {
  if (!host.isConnected) return null;
  const profile = context.with?.includes('@profile:');
  if (!request || typeof request !== 'object') return null;
  if (!profile && !context.preview) return null;
  if (request.action === 'put') {
    if (!profile && request.repo !== context.preview) return null;
    if (context.repo && !profile && (context.repo !== context.preview || context.branch !== 'main')) return null;
    if (typeof request.repo !== 'string' || !request.repo.startsWith('did:')) return null;
    if (typeof request.image !== 'string' || request.image.length > MAX_IMAGE ||
        !/^data:image\/webp;base64,[A-Za-z0-9+/=]+$/.test(request.image)) return null;
  } else if (request.action === 'get') {
    if (!profile || !Array.isArray(request.repos) || request.repos.length > 200) return null;
  } else return null;

  // The opaque guest has no storage. Relay through the existing authenticated
  // data port until the real top document owns the cache.
  if (window.parent !== window) return window.tonk.preview(request);
  if (!profile) return null;
  if (scope !== context.with) {
    scope = context.with;
    entries = {};
    try {
      const saved = JSON.parse(sessionStorage.getItem(KEY));
      if (saved?.scope === scope) entries = saved.entries || {};
      else sessionStorage.removeItem(KEY);
    } catch (_) { /* unavailable storage is a normal memory-only cache */ }
  }
  const now = Date.now();
  for (const [repo, entry] of Object.entries(entries)) {
    if (!entry || now - entry.at > TTL || typeof entry.image !== 'string') delete entries[repo];
  }
  if (request.action === 'get') {
    return Object.fromEntries(request.repos.filter(repo => typeof repo === 'string' &&
      Object.hasOwn(entries, repo)).map(repo => [repo, entries[repo].image]));
  }
  if (entries[request.repo] && now - entries[request.repo].at < 60000) return null;
  entries[request.repo] = { at: now, image: request.image };
  const ordered = Object.keys(entries).sort((a, b) => entries[b].at - entries[a].at);
  for (const repo of ordered.slice(32)) delete entries[repo];
  try { sessionStorage.setItem(KEY, JSON.stringify({ scope, entries })); } catch (_) { /* best effort */ }
  return null;
}
