# tonk-ui

The Trunk-built single-page web application that is the Tonk shell.

This crate bundles the browser-facing Tonk app: a Leptos (CSR) front end, the
service worker that backs every `/api/*` request, and the custom elements the UI
hosts. Trunk compiles it to Wasm and assembles `dist/` from [`index.html`](./index.html),
[`Trunk.toml`](./Trunk.toml), and the static assets under [`assets/`](./assets).
The shell mounts into the page, the service worker (`tonk-worker`) installs and
claims the page, and once the worker is controlling, the UI's `/api/*` fetches
route through it.

## Account route

`/account` is mounted directly in the top document rather than inside a sealed
`<tonk-site>` guest. WebAuthn must run on the `tonk.spot` RP-ID origin, so
`<tonk-account>` owns account creation and passkey self-link there. It reads the
local profile DID from `/api/identify`, sends root-signed ceremony bytes to
`https://accounts.tonk.spot`, then persists the accepted delegation through
`/api/account/link`. The account-service base can be overridden with a `service`
attribute for local tests and off-apex staging.

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

`index.html` registers `/service_worker.js`, then the shell waits (via the
`serviceWorkerActivates` bootstrap in `index.html`) until the worker is
*controlling* the page before issuing any `/api/*` request. The worker is the
local backend: the UI talks to it over HTTP and listens for change notifications
on a `BroadcastChannel`, surfaced to Leptos as reactive signals.

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
nix run .#build:web     # nix build .#tonk-ui
nix run .#dev:web       # trunk serve, proxying /ucan/ to the access service
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
