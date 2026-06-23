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

## Stage 4 — ready to execute (needs a browser at each step)

The seam is in place: `tonk_host::bridge::session_id()` is public and returns
the same `host:<uuid>` the SW receives, so the shell can bind
`entity={session_id}`.

1. **Prove Stage 3 first** (the gate). With the app running, navigate to
   `/space/{home}/board`, then read the host-id entity on the space's `main`
   overlay: confirm a `router/active` fact pointing at `tonk:route/board`. Mount
   `<tonk-display model=router/active entity={host-id}>` on a scratch route and
   confirm it renders the board. Fire a `route` command and confirm the table
   updates (re-request the path, see the new match).
2. **Switch the shell.** Replace the `<Router>/<Routes>` block in
   `launcher.rs` with `<tonk-display model=router/active entity={host-id}>`
   (host-id from `tonk_host::bridge::session_id()`).
3. **Move routes to `route!`.** `/` and `/join` → profile-side `router/route`
   instances (profile.yaml); per-space `/board` `/inspector` `/{entity}@{model}`
   → `router/route` in core.yaml. Level 0 already owns the `/space/{seg}` prefix.
   The SW must also match+stamp `router/active` for the **profile** target (Stage
   3b only does it for spaces — extend `stamp_host_context`).
4. **Nav glue.** Intercept same-origin link clicks + `popstate`, forward to the
   SW via the existing `navigate` channel (now page→SW). pushState generates no
   FetchEvent, so the host must forward it; the SW re-stamps `router/active`, and
   the subscribed shell display switches with no reload.
5. **Delete `leptos_router`** from `tonk-ui/Cargo.toml` (5 source refs, in
   `launcher.rs`, `display.rs`, `space_sealed.rs`).

## Stage 5+ (after the cutover)

- **Reduce the relay host to the minimum** (task): with routing in the SW, the
  host carries only headers + nav forwarding. Strip any host-side routing
  intelligence.
- **Containment** (plan Stage 5): tie each `/api` request to its host-id's
  Level-0 `(repo, branch)` via the document path; reject cross-target requests.
- **Live router** (the deferred optimization): subscribe to `router/route` per
  branch and cache/rebuild the `matchit` router, plus `router/conflict` overlay
  facts for discoverable conflicts.
