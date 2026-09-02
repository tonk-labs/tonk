# tonk-ui

The Trunk-built single-page web application that is the Tonk shell.

This crate bundles the browser-facing Tonk app: a Leptos (CSR) front end, the
service worker that backs every `/api/*` request, and the custom elements the UI
hosts. Trunk compiles it to Wasm and assembles `dist/` from [`index.html`](./index.html),
[`Trunk.toml`](./Trunk.toml), and the static assets under [`assets/`](./assets).
The shell mounts into the page, the service worker (`tonk-worker`) installs and
claims the page, and once the worker is controlling, the UI's `/api/*` fetches
route through it.

## Local identity and invite visits

Durable browser operations use a provider-neutral passkey root stored locally as
an exact `root → device` delegation. The top document handles identity-required
messages because passkey ceremonies cannot run in sealed guests. One typed
identity bridge turns Rust DTOs into ordinary camelCase JavaScript objects; the
serialized gate queues concurrent requests, restores focus after cancel or
failure, and replays a successful operation exactly once.

Opening an audience-open invite first installs only bounded guest authority.
“Join this space” explicitly claims to the root. Targeted invites go directly
through the durable root gate. A remote-backed join stages its authority and
content first; the replica becomes visible, guest state is cleared, backup is
started, and navigation occurs only after that stage is usable. Failed,
revoked, wrong-recipient, and unavailable joins leave no visible replica.

## Account route

`/settings` is mounted directly in the top document rather than inside a sealed
`<tonk-site>` guest. WebAuthn must run on the `tonk.network` RP-ID origin, so
`<tonk-account>` owns account creation and passkey self-link there. It reads the
local profile DID from `/api/identify`, sends root-signed ceremony bytes to the
configured account service, then attaches provider metadata through
`/api/account/attach`; it does not replace or own the local root.

The page fetches `GET /.well-known/tonk` once and uses its typed
`accountServiceUrl`. It never infers services from a
hostname or falls back to production for an unknown origin. A `service`
attribute remains an explicit local-test/operator override. Once attached,
background account operations use the persisted provider URL.

## Binaries

The crate produces two Wasm bin targets, both referenced from `index.html` by a
`data-bin` Trunk `rel="rust"` link:

- **`ui`** ([`src/bin/ui.rs`](./src/bin/ui.rs), `data-type="main"`): the page
  entry point. It installs the panic hook, registers every custom element the app
  uses (`tonk-sigil`, `tonk-host`, `tonk-display`, `tonk-board`, `tonk-portal`,
  `tonk-workspace`, the inspector, the `<tonk-code>` JS bundle, and `<tonk-tree>`),
  then mounts the `TonkShell` Leptos component into the document body. Under debug
  builds (`trunk serve`) it also injects the dev-only `hot-swap.js` reload client;
  release builds never load it.
- **`worker`** ([`src/bin/worker.rs`](./src/bin/worker.rs), `data-type="worker"`,
  `data-bindgen-target="web"`): the service-worker module. It exposes an
  `activate()` entry that constructs `tonk_worker::TonkServiceWorker`. The static
  [`assets/service_worker.js`](./assets) imports and drives this Wasm.

## How the SPA and service worker compose

`index.html` starts one memoized registration and update check before the UI
Wasm mounts. The static `#tonk-boot` overlay stays visible until that promise
settles, and the top-document application root is not created while a worker
replacement is in flight. `updateViaCache: "none"` keeps the update job's
script fetches fresh; an explicit `ServiceWorkerRegistration.update()` starts
that job on every user-initiated warm load.

The load lifecycle has four cases:

- A first install explicitly asks the activated worker to claim the current
  document, then continues without reloading.
- An online warm load checks for a newer worker behind the boot overlay.
- A real warm replacement activates through `skipWaiting()`. The update-aware
  page then explicitly asks that successor to claim it and reloads once before
  the application root mounts so the document, shell, and controller agree.
- An offline warm load keeps its existing controller and cached shell. A failed
  update check does not unregister the worker or clear CacheStorage, IndexedDB,
  or other local Tonk state.

The incumbent does not retire merely because `updatefound` reports an
`installing` candidate. It keeps sync and language-server streams operational
until that candidate reaches `installed`, or until a durable waiting successor
is found after restart. A candidate that becomes `redundant` during install
therefore leaves the incumbent fully usable.

Activation alone does not claim already-open documents. Pages cached before
this update protocol therefore remain on their compatible existing controller
until navigation; an update-aware page opts into the new controller only when
it can perform the guarded alignment reload. Activation also retains every
generation-named shell and worker-Wasm cache: an older page or retained worker
may hold the only live reference to an offline generation, so storage pressure
is the only automatic eviction policy until reference-safe cleanup exists.
Each generation cache is sealed after install. A retained controller serves a
cached shell or static asset without revalidation, deletion, or overwrite; a
missing asset returns an actionable `503` online and offline. The build
publisher emits a full-SHA-256 `asset-manifest.json` for the shell, UI, lazy,
and sealed-guest resource graph, excluding mutable deployment controls such as
`version.json` and `kill-switch.json`. Install fetches every response with
`cache: "no-store"`, verifies the manifest, every listed asset, and worker Wasm,
then publishes through nonce-named staging caches. A durable generation marker
records `building`, `publishing`, and finally `adopted`; a same-build retry may
remove only the exact unadopted names recorded by that marker. Stable caches
with no valid adoption provenance fail closed, while adopted final caches are
verified read-only and never replaced. No runtime path backfills or repairs a
retained cache. If storage evicts worker Wasm, the old
worker may boot from a fresh response only when it still matches its stamped
digest, and it does not write those recovered bytes back. In production,
authored `no-store`, `reload`, and `no-cache` request flags cannot bypass the
sealed generation; only the unstamped `dev` worker honors them for hot reload.

The deployed Cloudflare tree is stamped again after the guide and Storybook
are overlaid, so the build identity and manifest describe the bytes actually
served. Each static site's physical `*/index.html` and directory URL are exact
members. A slashless `/guide` or `/storybook` navigation receives a `307` to
the stamped trailing-slash route so relative assets resolve inside that site;
ordinary application routes continue to use the root SPA document. Only exact
stamped top-level paths are eligible for the immutable shell cache.
`/.well-known/tonk`, other unmanifested live edge routes, and requests from a
registered nested client go through Rust/network instead. Client lookup failure
also delegates conservatively rather than cross-serving a top-level asset into
a guest.

Every automatic alignment, update, watchdog-recovery, and development hot-swap
reload also waits for both page-local and origin-global account-setup safety.
`document.documentElement[data-tonk-account-setup-critical]` and the optional
`window.tonkAccountSetupMayReload()` predicate guard the current document; the
predicate must return exactly `true`, and failure defers. Separately, IndexedDB
`tonk-update-safety-v1`, store `holds`, key `account-setup`, contains the
minimal durable hold `{version: 1, kind: "account-setup", operationId,
leasedRevision}` while an Arm may have committed without a durable Stage.
Absence is the only globally safe result. A malformed value, storage error, or
missing Web Locks API fails closed.

Both page handoff and service-worker `clients.claim()` take the exclusive Web
Lock `tonk-update-safety-v1` and re-read the hold before acting. The page invokes
the irreversible claim/reload callback before releasing that lock, closing the
cross-tab check-to-action gap. The `tonk:account-setup-critical-change` DOM
event and `account-setup-hold-changed` message on the same-named
`BroadcastChannel` are advisory wakeups; every retry rechecks authoritative
state. This leaves an Armed/pre-Stage document on its compatible worker until
Stage or an authenticated Inspect has proved recovery durable.

An explicit readiness rejection is not treated as a silent boot stall. Before
returning with the application root unmounted, the UI terminalizes the static
boot shell with “Tonk couldn’t start. Check your connection, then reload. Your
local data is safe.” Terminalization cancels automatic watchdog recovery,
clears its per-tab retry counter, and never reloads, deletes CacheStorage, or
unregisters a worker. The first terminal message wins so a more specific cause
can retain its own recovery guidance; after correcting the cause, the person
chooses when to reload. For a boot that stops making progress without producing
an explicit error, the watchdog performs at most one plain reload. A second
silent stall terminalizes with the same safe-state guidance and leaves every
cache and service-worker registration intact. A deployment withdrawal flag is
also non-destructive: it compares the flag with the immutable generation
embedded in this page, stops that worker from serving further data-plane work,
and offers update/reload recovery without deleting caches or unregistering any
scope. Ambiguous, missing, or malformed flags leave the running build alone.

The one-shot alignment reload is guarded in `sessionStorage`; a stable load
clears the guard. There is one rollout boundary: a shell cached before this
bootstrap ships cannot run code it does not contain. Its existing worker and
ordinary browser/deployment update path remain responsible for adoption; this
protocol never mutates a retained generation in place. Later deployments are
detected on the first warm load by update-aware documents.

The service worker is the local backend: the UI talks to it over HTTP and
listens for change notifications on a `BroadcastChannel`.

The crate's library side (see [`src/lib.rs`](./src/lib.rs)) provides the pieces the
`ui` binary wires together:

- `api`: HTTP client for the service worker's `/api/*` surface.
- `broadcast` / `watch`: `BroadcastChannel` subscription, bridged into a Leptos
  signal.
- `components`: the Leptos UI, rooted at `TonkShell` / `TonkLauncher`.
- `sync_controller`: automatic push/pull of the active repository's upstream
  branches.
- `did` / `error`: DID parsing helpers and the UI error type.

### Routing

Routing is client-side (`leptos_router`), defined in
[`src/components/launcher.rs`](./src/components/launcher.rs). Every space segment
is a single `:space` param encoding `{branch}@{label}:{id}` (branch defaults to
`main`), parsed by [`src/components/route.rs`](./src/components/route.rs). The
routes:

- `/`: the Tonk Hub (space picker), rendered bare.
- `/space/:space/view/:entity`, `/space/:space/board/:board`, `/profile`, `/join`:
  rendered inside the chromed `<wa-page>` shell.
- `/space/:space` and `/space/:space/*subject`: the bare `<tonk-display>` route
  (the `*subject` wildcard preserves entity URIs containing `/`).

Static-keyword routes (`view`, `board`) are defined before the wildcard display
routes because the router matches in definition order.

## Build and run

Build the app with Trunk (driven through the flake):

```sh
nix develop . -c build:web     # nix build .#tonk-ui
nix develop . -c dev:web       # trunk serve, proxying /ucan/ to the access service
```

`Trunk.toml` disables Trunk's autoreload (the dev-only `hot-swap.js` taps Trunk's
change channel instead, re-seeding the standard library in place and reloading
only on a genuine Wasm change) and watches every workspace crate that contributes
to the bundle.

## Integration tests

Tests live alongside the components (`#[dialog_common::test]`, `it_*` names) and
drive a real browser. They are native-only: `TestServers::start`
([`src/helpers.rs`](./src/helpers.rs), behind the `helpers` /
`integration-tests` features) boots the access service, a Caddy web server serving
the built `dist/` (proxying `/ucan/*` to the access service), and ChromeDriver,
then hands each test a `TestEnvironment`. `TestEnvironment::driver()` opens a
WebDriver session (`thirtyfour`) against that deployment.

Because they require a built deployment plus Chrome/ChromeDriver, these tests are
excluded from the workspace Wasm test archive and run as native cargo tests with
the `integration-tests` feature:

```sh
cargo test -p tonk-ui --features integration-tests
```

Set `NO_HEADLESS` to watch the browser, or `CHROME` to point at a specific Chrome
binary.
