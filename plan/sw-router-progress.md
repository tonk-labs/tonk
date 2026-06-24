# SW router — autonomous progress (Stages 2–3 landed; Stage 4 prepared)

Branch: `feat/sw-router-next` (off `feat/router`). Stages 2 and 3 are committed
and verified at the build/test level. Stage 4 (the production cutover) is left
for browser verification — see "Why I stopped before Stage 4" below.

## What landed (committed)

- **Stage 2** (`09904212`): Level 0 routing moved to `tonk-schema` (`parse_space`,
  `resolve_path`, `RouteTarget`), shared by worker + UI. `HostContext {path,hash}`
  concept keyed on the per-tab host-id entity. The SW resolves the document path
  from `Referer`, maps it to a `(repo, branch)`, and stamps the context on that
  branch's overlay (the `state:here` pattern keyed per tab).
- **Stage 3a** (`59b08774`): routes-as-concepts in `core.yaml` — the `route`
  command (write API), the durable `router/route` table, the two translation
  rules (assert + per-path eviction), the `router/active` indirection concept +
  delegating view, `route/default` and `route/board` page models with views, and
  a seeded default+board route table.
- **Stage 3b** (`e38c6edf`): `RouterRoute` / `RouterActive` Rust concepts. The SW
  now matches the remaining (Level 1) path against the branch's `router/route`
  table with `matchit` and stamps the matched model as `router/active` on the
  host-id entity.

All three are **additive**: the Leptos router still drives production, so the app
boots exactly as before. The data-driven path runs alongside it.

## Browser-verified (Stage 3 end to end)

Ran the app (`nix develop .#default --command dev:web`, port 8080) and drove it
with Chrome DevTools MCP. Confirmed on a real space
(`did:key:z6MkpYAk…`/`main`):

- `HostContext` stamps on the space branch overlay, keyed by host-id entities,
  with `path: /space/z6MkpYAk…`.
- `router/active` stamps `model: tonk:route/default` for the `/` remaining path,
  and `model: tonk:route/board` when the path is `/space/…/board`.
- The seeded `router/route` table is present and queryable.

**Bug found + fixed in the process (commit bcb6abef):** a service worker reads
`request.headers`, which never includes `Referer` (the browser exposes it only
as the `request.referrer` property). PR #527 had dropped `x-tonk-path` in favour
of `Referer`, so the SW never saw the path and every stamp landed on the profile
branch with an empty path. Restored the explicit `X-Tonk-Path` header (host and
guest both know their document path). Re-verified green.

## Stage 4 progress — sealed /space renders through router/active (one bug left)

Wired the data-driven shell for the sealed `/space` route and browser-verified
most of the chain. Decisions used: the guest proxy fills `entity` from the
host's session id; route views always thread `entity={this}`.

What works (browser-confirmed):
- The guest proxy `<tonk-host>` fills `entity` on `[data-tonk-entity="session"]`
  from the host's session id (passed in the bridge context as `session`, since
  the guest's data queries are issued by the host's `<tonk-host>` carrying
  `X-Tonk-Session: host:<uuid>` — NOT the guest's own `guest:…`). The sealed
  content binds `<tonk-display model=tonk:router/active data-tonk-entity=session>`.
- `router/active` resolves to `tonk:route/default` on the bound `host:<uuid>`.
- The SW stamps both `router/active {model}` and `route/path {path}` on the
  host-id entity. Direct query AND SSE-subscribe both return the `route/default`
  instance (`path: /`) when pinned to the host-id.

The one remaining bug: the inner `<tonk-display model=tonk:route/default
entity=host:…>` (rendered by `router/active/view`'s delegation) reports
`NoEntity` ("Not found"), even though that exact instance resolves via direct
query and SSE. Both attributes are set on it (the debug box prints them). So it
is a `<tonk-display>` delegation/entity-query gap, not a data-layer problem —
needs logging inside `tonk-display/src/element.rs` `handle_entity_frame` /
`entity_query` to see what query it actually issues for the nested route model.
Everything compiles, the library lowers, and schema tests pass.

## Verified autonomously

- `tonk-schema`, `tonk-worker`, `tonk-ui` compile on `wasm32-unknown-unknown`;
  schema + worker also build native.
- `cargo test -p tonk-schema --lib` — 104 pass (incl. new `space::` tests).
- `cargo test -p tonk-worker --test standard_library` — 3 pass; the library
  (with all the new routing concepts/rules) lowers cleanly, so the repo-creation
  seed won't break.
- Worker wasm tests compile (`--no-run`).
- New code is clippy-clean. (The only clippy errors are the pre-existing
  `dialog-reactor` `Arc`-not-`Send`/`Sync` ones and two pre-existing `router.rs`
  test-code lints — none mine.)

## One simplification vs the plan

The plan's Stage 3 caches a `matchit::Router` per mounted branch and rebuilds it
on each `router/route` subscription frame (live updates with no request). I build
a fresh `matchit::Router` **per request** instead — correct and fully testable,
just not live-without-a-request. The subscription-cached version is a perf
optimization to add once it can be browser-verified.

## Why I stopped before Stage 4

Stage 4 swaps `<TonkLauncher>`'s `<Router>/<Routes>` for the fixed shell
`<tonk-display model=router/active entity={host-id}>`, moves the remaining routes
into `route!` concepts, wires host nav glue (link clicks + `popstate` → SW), and
deletes `leptos_router`. This is the one **production-breaking** step, and the
plan gates it on a "proven host-id-entity seam" (Stage 3 rendering one route end
to end in a browser). That proof needs a running app, which can't be done
unattended. Landing Stage 4 blind risks a non-booting app.

## Stage 4 — refined plan (decision: keep `/` and `/join` special for now)

Stage 3 is proven, so the seam is real. The remaining cutover is the
production-breaking step; the user chose to keep `/` and `/join` as Level-0
specials, so only the per-space render goes data-driven now. The blocker
surfaced while scoping: the sealed `/space/:space` content hardcodes
`<tonk-display model=tonk/space>` (`space_sealed.rs:75`), and to render through
`router/active` the inner display needs `entity={the guest's host-id}` — but the
guest mints its `SESSION` as a private `var` inside the bridge IIFE
(`tonk-portal/src/bridge.rs:120`), exposed only via the `x-tonk-session` header,
not on `window.tonk`. So the cutover needs new plumbing on the working sealed
path. Steps, each browser-verified:

1. **Expose the guest session id.** Add `session: SESSION` to the `window.tonk`
   object (`tonk-portal/src/bridge.rs`, the `var tonk={…}` literal) so guest
   content can reference it. Additive; can't break anything.
2. **Make `<tonk-display>` resolve a dynamic entity** from the bridge context
   (today `entity` is a literal attribute; the guest proxy `tonk-guest/guest_host.rs`
   doesn't annotate displays). Either teach the proxy to fill `entity` from
   `window.tonk.session`, or add a small indirection element. THIS is the real
   work — verify the nested `router/active` → matched-model delegation renders.
3. **Switch the sealed content** (`space_sealed.rs:75`) from
   `<tonk-display model=tonk/space>` to
   `<tonk-display model=tonk:router/active entity=<guest session>>`.
4. **Keep `/` and `/join` on Leptos** (user's call). Only `/space/:space` goes
   data-driven. So `leptos_router` is NOT deleted yet — that waits until `/` and
   `/join` also move to `route!` (a later step). Update task #9 accordingly.
5. **Per-space sub-routes** (`/board`, `/{entity}@{model}`, the `*subject`
   route in `display.rs`): move to `route!` concepts with param binding, and
   retire the Leptos `/space/:space/*subject` route. The artifact routes need
   matchit param capture → facts on the host-id entity → inner view reads them.
6. **Nav glue + live router** as before (host forwards link clicks/popstate to
   the SW; SW re-stamps; subscribed display switches). The current
   per-request matching already updates on the next request; live-without-request
   needs the subscription-cached matchit router.

## Stage 5+ (after the cutover)

- **Reduce the relay host to the minimum** (task): with routing in the SW, the
  host carries only headers + nav forwarding. Strip any host-side routing
  intelligence.
- **Containment** (plan Stage 5): tie each `/api` request to its host-id's
  Level-0 `(repo, branch)` via the document path; reject cross-target requests.
- **Live router** (the deferred optimization): subscribe to `router/route` per
  branch and cache/rebuild the `matchit` router, plus `router/conflict` overlay
  facts for discoverable conflicts.
