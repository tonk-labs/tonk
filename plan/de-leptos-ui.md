# De-Leptos the tonk-ui shell

Goal: remove the `leptos`, `leptos_router`, and `leptos_use` dependencies
from `tonk-ui` entirely, replacing the SPA framework with the same
custom-element + reactor-subscription substrate the rest of the system
already uses. The app must boot and pass its integration tests at every
stage — no big-bang rewrite.

Leptos is confined to one crate (`rust/tonk-ui`). It does three load-bearing
jobs there: the **URL router**, the **reactive system**, and the
**component/shell model**. This plan tackles them in dependency order,
thinnest-first, so each step lands a working app.

## What Leptos is actually doing (from inventory)

Router (`leptos_router`, only in `launcher.rs`):
- One `<Router><Routes>` with 6 patterns → 5 views, one `<ParentRoute>`
  chrome shell with `<Outlet/>`.
- Params `:space`, `:entity`, `:board`, `*subject`, `""`, read in 3 views
  via `#[derive(Params)]` + `use_params`.
- `use_location` (read-only) in the toolbar for the active-space indicator.
- A `<Routes fallback>` not-found closure.
- ZERO programmatic navigation (no `use_navigate`/`<Navigate>`/`<Redirect>`).
  Navigation is plain `<a href>` links rendered by directory views, plus the
  new SW→client navigate message (already shipped).

Reactivity:
- `mount_to_body(TonkShell)` (bin/ui.rs) — the one root mount.
- `mount_to(...)` inside `<tonk-inspector>` (inspector.rs) — a second mount.
- `TonkShell` owns shared context: `HostId`, `ProfileResource`,
  `LastJoinOutcome`, plus a `watch::<Notification>("/api/profile")` +
  `Effect` that refetches the profile on broadcast.
- `sync_controller.rs` — background sync, built on `leptos_use` timers/
  listeners and the reactive owner for teardown (shallow: signals read
  untracked, no Effects).
- `watch.rs` — a `BroadcastChannel` → `ReadSignal` bridge; one consumer.
- Heavy reactivity: `TonkDisplayView` (resource + imperative DOM Effect +
  name resolve), `TonkSpaceViewer` (Suspense/ErrorBoundary/Either + iframe),
  `TonkInspector`/`InspectorCell` (notebook eval engine).

Components, by weight:
- THIN (already just custom-element mounts): `TonkHub`, `TonkJoin`,
  `TonkBoardView`, `ChromeShell`.
- THICK: `TonkShell` (infra), `TonkToolbar` (sidebar list), `TonkDisplayView`,
  `TonkSpaceViewer`, `TonkInspector`, `InspectorCell`.

## The replacement substrate (already in the codebase)

- **Custom elements** (`custom-elements` crate) — used everywhere already
  (`<tonk-host>`, `<tonk-display>`, `<tonk-board>`, `<tonk-page>`, the
  workspace elements). Route views become elements or `<tonk-display>`
  directory views.
- **`<tonk-display>` directory views** — already render `TonkHub`/`TonkJoin`
  entirely; the model lives in `core.yaml`/`profile.yaml`.
- **Reactor subscriptions via `<tonk-host>`** — the worker→page push channel
  (`tonk-subscribe`/SSE) that replaces `watch.rs` + the profile `Effect`.
- **SW→client `postMessage`** — already added for navigate; the general
  worker→page command channel.
- **`broadcast.rs`** — already leptos-free; survives unchanged.

## Router replacement: a `<tonk-route>` matcher (no leptos_router)

The router is the spine, so it comes first behind the thin-view prep. The
inventory makes this small: 6 static patterns, params read in 3 views, no
programmatic nav.

Approach: a minimal client-side `<tonk-router>` custom element (or a plain
mount function) that:
1. Reads `window.location.pathname` and listens for `popstate` +
   intercepts same-origin `<a>` clicks (history pushState) — the bits
   `leptos_router` did invisibly.
2. Matches the 6 patterns in definition order (a tiny ordered table; the
   `*subject` wildcard last). Extracts named params into a struct.
3. Mounts the matching view element, setting params as attributes — exactly
   the pattern the route views already want (they only use params to set
   attributes on `<tonk-display>`/`<tonk-board>`).
4. Renders the fallback for no match.

This is the deferred `<tonk-router>`/`route!:` idea, but kept CLIENT-side and
minimal for now (no SW-side `route!` facts yet — that generalization is a
separate later step, noted at the end). Param-as-attribute means the thick
param-reading logic (`parse_space`/`parse_subject`/name-resolve) moves out of
Leptos `Signal::derive` closures and into the element's `attributeChangedCallback`
or a small parse step — the parsers themselves (`route.rs`) are already
leptos-free.

## Stages (each ends with a booting app + green tests)

### Stage 0 — Inventory + plan (this document). DONE.

### Stage 1 — Thin views to elements/views, behind the still-Leptos router
Convert the THIN views so they no longer need Leptos, while `leptos_router`
still drives them:
- `TonkHub`, `TonkJoin` — already pure `<tonk-display>` mounts; collapse each
  to a bare element/template the router can mount with no `#[component]`.
- `TonkBoardView` — replace the 4 `Signal::derive_local` param plumbing with
  a `<tonk-board>` whose attributes come from the router's param-attributes.
- `ChromeShell` — becomes a static element wrapping `<tonk-toolbar>` + a
  slot; the `data-last-join-outcome` reactive attr is dropped (LastJoinOutcome
  is stale/unused per inventory — verify and remove).
Verify: routes still resolve, Hub/Join/Board render. Tests green.

### Stage 2 — Replace the router (`leptos_router` → `<tonk-route>`)
Build the minimal matcher above; port the 6-entry table from `launcher.rs`.
Move param extraction to attribute-setting. Replace `use_location` in the
toolbar with reading `location.pathname` + a `popstate`/route-change event.
After this, `leptos_router` is gone from Cargo.toml.
Verify: every route navigates (link clicks + back/forward), params reach the
views, fallback works, the new SW→client navigate still lands. Tests green
(esp. `it_navigates_to_the_default_space`, `it_joins_an_open_invite...`).

### Stage 3 — Replace root mount + shared reactivity (`TonkShell`, watch)
- `watch.rs` → a direct `broadcast::Subscription` (already leptos-free) that
  triggers a profile refetch; drop the `ReadSignal` bridge + the `Effect`.
- `TonkShell` context (`HostId`/`ProfileResource`) → resolve how the toolbar
  and space-viewer get host id / profile without `provide_context`: either a
  small shared `Rc<RefCell>`/global, or have those elements query the worker
  directly through `<tonk-host>` (preferred — removes the context entirely).
- Root mount `mount_to_body(TonkShell)` → register the root custom element
  and let the page host it (the `index.html` already bootstraps `<tonk-host>`
  and the SW); `bin/ui.rs` becomes element registration only.
Verify: app boots from `index.html` with no leptos mount, profile updates
still refresh, toolbar still lists spaces. Tests green.

### Stage 4 — `sync_controller.rs` off leptos_use
Replace the 5 `leptos_use` calls with raw equivalents:
- `use_interval_fn` → `setInterval` (store handle, clear on teardown).
- `use_event_listener` ×3 → `addEventListener` with stored closures.
- `use_debounce_fn` → a small manual debounce (timeout id + reset).
- The reactive-owner auto-teardown becomes explicit teardown tied to the
  owning element's `disconnectedCallback`.
The two signals (`syncing` guard, `source` input) → `Cell<bool>` + a getter.
After this, `leptos_use` is gone from Cargo.toml.
Verify: background sync still ticks (online/visibility/committed/interval),
status-refresh events still fire. Tests green.

### Stage 5 — Thick views: `TonkDisplayView`, `TonkSpaceViewer`
- `TonkDisplayView` — the `@`/`!` segment parser + `Name`-index resolve +
  imperative `create_element` Effect collapse into a custom element (or a
  `<tonk-display>` enhancement) that takes the route params as attributes and
  does the resolve in async on connect; 404/error become element states (the
  `display-reactive-states` design already governs this).
- `TonkSpaceViewer` — the Suspense/ErrorBoundary/Either + iframe become an
  element with loading/ready/error states driven by a `<tonk-host>` query
  for the repository and an HostId read; the iframe `src` gating moves into
  the element.
Verify: `/space/{space}/view/{entity}` viewer + bare display route render,
including not-found/error paths. Tests green.

### Stage 6 — Inspector internals off leptos
`TonkInspector`/`InspectorCell` are already inside the `<tonk-inspector>`
custom element; only their bodies are Leptos. Rebuild the notebook
(`cells`/`<For>`) and the cell (editor + eval engine, many signals/handlers)
on plain DOM + the existing `host_consumer::evaluate` dispatch. This is the
densest step; isolate it last so the rest of the app is already leptos-free.
The shared helpers it imports from `space.rs` (`TransactState`,
`classify_for_dispatch`, `render_transact_state`, `clear_pushed_diagnostics`,
`read_tonk_code_value`) are mostly leptos-free already — verify and relocate.
Verify: inspector cells run, auto-eval-on-clean-diagnostics works, results
render. Tests green.

### Stage 7 — Drop the dependency + cleanup
- Remove `leptos`, `leptos_router`, `leptos_use` from `rust/tonk-ui/Cargo.toml`.
- Replace the remaining trivial uses: `api.rs` `log!`/`window()` →
  `web_sys::window()` + a log macro (`tonk_common::log!`); `bin/ui.rs`
  `mount_*` already gone.
- Delete now-empty `watch.rs` / dead context types (`LastJoinOutcome` if
  confirmed unused).
- `cargo build` + full `test:web` + manual smoke of every route.

## Verification per stage
- `nix develop -c test:web:debug` (the wasm/integration leg) after each stage.
- The integration tests that exercise routing/join specifically:
  `it_navigates_to_the_default_space`, `it_surfaces_externally_created_space...`,
  `it_joins_an_open_invite_via_the_join_route` (launcher.rs).
- Manual smoke: `/`, `/join?...#seed`, `/space/{name}`,
  `/space/{name}/{entity}`, `/space/{space}/view/{entity}`,
  `/space/{space}/board/{board}`, an unknown path (fallback), back/forward.

## Deferred (not in this plan)
- SW-side `<tonk-router>` + `route!:` facts (routing as data on a branch).
  Stage 2 keeps routing client-side and minimal; promoting it into the SW is
  a separate effort once the Leptos router is gone.
- Any redesign of the inspector eval UX beyond a faithful port.

## Risk notes
- Stage 2 (router) and Stage 6 (inspector) are the two real-risk steps.
  Everything else is mechanical. Land them on their own so a regression is
  easy to bisect.
- `display-reactive-states` and `display-route-layout` memories govern the
  display/route element behavior — honor them in Stage 5.
- Keep each stage a separate commit; the app must boot at every commit.
