# Offline support

> Status: the cache/update/rollback mechanics below are superseded by
> `plan/service-worker-lifecycle-consolidation.md`. The current runtime seals a
> complete immutable generation during install, retains every adopted
> generation needed by an older controller, and crosses generations only
> through the page-directed claim/reload flow. It does not stale-revalidate or
> lazy-fill a generation, delete older generation caches during ordinary
> operation, or offer code rollback before the separate IndexedDB data
> compatibility policy is settled. The offline goals and compatibility warning
> remain applicable.

A Service Worker layer that caches the app shell so it loads
instantly and keeps working offline, with each release stored in
its own named cache so users can opt into switching forward and
roll back if a new version misbehaves.

## Why

The site already registers a Service Worker but doesn't cache
anything. Every load goes to the network; offline drops the app.
We also have no story for how a release reaches users, who right
now just see whatever the host last deployed on the next hard
reload.

## Constraints

- Static Trunk-built Rust/WASM SPA. No backend routing layer;
  version selection has to be client-side.
- Trunk hashes asset URLs, so cached entries are immutable by
  URL. A new build only invalidates the entry-point HTML and the
  manifest; everything else changes URL when its content changes.
- User data lives in IndexedDB and syncs opportunistically to S3.
  The SW only cares about the app shell, but rollback semantics
  intersect with IDB (see milestone 3).

## Milestones

Each milestone lands a usable improvement on its own. Don't ship
the next one without confirming the previous holds in production.

### Milestone 1 — Cache-on-demand

**Goal:** the app keeps working offline once the user has loaded
it at least once. No version awareness, no banners.

- SW intercepts same-origin GETs.
- Strategy: stale-while-revalidate against a single named cache.
  Hit the cache first, fall back to network on miss, write the
  network response back into the cache.
- `index.html` and `/` get a network-first variant so the user
  picks up new builds on their next online navigation. Hashed
  assets the new HTML references will fetch and cache themselves.
- Cache name is just a constant; no per-version partitioning yet.
- HTTP cache headers must align with this strategy: `index.html`,
  the SW source, and the future `version.json` are `no-cache`;
  hashed assets are `immutable`. Without this the browser HTTP
  cache fights the SW cache. Configured via whatever the host
  provides (a `_headers` file, a worker, etc.).

Done when: load the app, kill the network, reload, app comes up.

### Milestone 2 — Opt-in updates per version

**Goal:** when a new release deploys, the user sees a banner and
chooses when to switch. No surprise reloads.

- Build emits `dist/version.json` listing `{ version, builtAt,
  assets[] }`. `version` is the git commit SHA when available
  (CI exports `GIT_COMMIT_SHA`); falls back to a content hash of
  the asset set.
- SW stores each version's assets in its own `app-<version>`
  cache, plus a `meta` cache holding a `current` pointer
  (`{ version, installedAt }`).
- Fetch handler reads `current` from `meta`, opens the matching
  `app-<version>`, serves from there. Misses lazy-fill into the
  current version's cache.
- Periodic `/version.json` poll from a registered client (every
  5min and on visibility/focus). When `version !== current`, SW
  pre-installs the new version's assets via
  `cache.addAll(manifest.assets)` and posts `update-available` to
  all clients.
- Client renders a small banner ("Version X available — switch?").
  Accepting sends `switch-to`; SW updates `current` in `meta` and
  broadcasts `switched`; client reloads.
- GC: any `app-<version>` cache whose version isn't `current`
  gets deleted on switch.

Done when: deploy twice in a row, banner appears in the first
deployment after the second goes live, accepting the banner
reloads on the new version.

### Milestone 3 — Rollback to previous version

**Goal:** if the new version misbehaves, the user can return to
what was working before. Only ship after the data-compatibility
question below has an answer.

- `meta` cache gains a `previous` pointer (`{ version,
  installedAt, previousSince }`). On `switch-to`, the
  outgoing `current` becomes `previous`.
- Client surfaces a "Rollback to version X" affordance whenever
  `previous` is non-null. Sends `rollback`; SW swaps the two
  pointers and broadcasts `rolled-back`.
- GC retains `previous` for `RETENTION_MS` (default 7 days) past
  `previousSince`, then deletes its cache. Triggered on switch,
  rollback, and a hourly client-driven `gc` message — SWs can be
  killed at any time, so no SW-side timer.

**Blocker — data compatibility.** The SW can swap code
atomically but can't undo IDB writes. If V2 changed the schema
or wrote records V1 can't read, rollback is either lossy or
broken. Pick one before exposing the rollback button:

1. Enforce additive-only schema changes by convention. Cheap,
   relies on review discipline.
2. Tag every IDB record with the producing app version; V1 skips
   records it doesn't understand.
3. Snapshot IDB before any V2 migration; rollback restores the
   snapshot. Expensive in storage, cleanest semantics.
4. V2 sets a flag when it performs an irreversible migration; SW
   refuses rollback while the flag is set and the client hides
   the button.

The plan defaults to option 4 as a safe interim — rollback
silently disabled in any case the new version touched data — and
revisits if a stricter contract becomes worth the cost.

Done when: switch forward, observe `previous` retained, roll back,
observe original behavior; after retention window elapses, GC
deletes the previous version's cache.

## Out of scope

- Server-side rollback. The host may have its own; we don't
  depend on it.
- N-version history. Only `current` + `previous`.
- Reviving an evicted version by re-fetching its assets from a
  stable per-deployment URL. Possible if the host exposes one,
  but cross-origin and not worth the complexity in v1.
- Forcing reloads. Both forward switch and rollback are user-
  initiated.
- Background Sync / pre-warming newly detected versions. Adds
  complexity without changing the user-visible flow much.

## Open decisions

- `RETENTION_MS` default — 7 days unless someone has a reason.
- Reload behavior on switch — auto-reload all clients vs. show
  "reload to apply" prompt per-tab. Default to the prompt; tabs
  with unsaved state shouldn't reload behind the user's back.
- `navigator.storage.persist()` — call on first install (Firefox
  prompts the user; Chromium silently grants for installed PWAs).
  If the Firefox prompt is too noisy, gate it behind an explicit
  user action instead.
