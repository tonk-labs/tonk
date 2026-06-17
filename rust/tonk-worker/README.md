# tonk-worker

The WASM service worker that fronts a dialog repository over an HTTP API.

This crate runs as a browser service worker, intercepts fetch events, and serves
them from an [`axum::Router`] instead of the network. Authoring a route is the
same mental model as a server, but the "server" is a service worker running in a
browser tab. `TonkServiceWorker` (the JavaScript-visible binding in
[`worker.rs`](src/worker.rs)) wires the SW lifecycle (`onfetch`, `onmessage`,
`onupdatefound`, `onactivate`, `sync`) into that router. A small JavaScript shim
(`tonk-ui`'s `service_worker.js`) loads the WASM module, because SW
install/activate timing is sensitive and WASM init is async.

## API surface

Routes are assembled in [`router.rs`](src/router.rs) under `/api/`, with one
submodule per route family in [`router/`](src/router):

- **Repository lifecycle** (`repository.rs`): `PUT/GET /api/repository/{repo}`
  to create (each PUT mints a fresh `did:key` routing key) and read a repo;
  `POST .../remote` to attach a remote and branch upstream.
- **Query** (`query.rs`): `POST .../branch/{branch}/query` takes a serialized
  `ConceptQuery` and returns conclusions. With `Accept: text/event-stream` the
  response is an SSE subscription that re-broadcasts on every branch change.
- **Evaluate** (`evaluate.rs`): `POST .../branch/{branch}/evaluate` accepts an
  asserted-notation document (any mix of queries and mutations), runs the
  analyze to query to plan to commit pipeline (via `tonk-evaluator`), and
  returns matches plus a commit summary.
- **Transact** (`transact.rs`): `POST .../branch/{branch}/transact` takes a
  typed `TransactRequest`, bypassing notation so per-mutation
  transient/durable classification flows straight to the reactor's transaction
  builder.
- **Claim** (`claim.rs`): low-level `assert`/`retract`/`select` over individual
  `(entity, attribute)` facts.
- **Sync** (`sync.rs`): `sync`, `sync/pull`, `sync/push`, `sync/status` per
  branch, plus the background-sync sweep driven by the SW `sync` event.
- **Transfer** (`transfer.rs`): CSV `export` (stream branch artifacts as
  `text/csv`) and `import` (commit CSV rows as assertions).
- **Invite / join** (`create_invite.rs`, `join.rs`): mint invites for a repo and
  `POST /api/profile/join` to create or renew a replica from an invite URL.
- **Profile** (`profile.rs`, `identify.rs`): `GET /api/identify`,
  `GET /api/profile`, and a parallel profile-as-repository surface
  (`/api/profile/branch/{branch}/{query,evaluate,transact}`) since the profile is
  its own repository outside the named-repo namespace.
- **Inspect** (`inspect/`): read-only views of branch state, remote/remote-branch
  status, and archive index blocks for debugging.
- **Host/guest bridge** (`host.rs`, `bridge.rs`): the iframe bridge (see below).
- **LSP** (`lsp.rs`, `lsp_env.rs`): a language-server surface merged into the
  router, carrying its own `LspHub` state and an SSE event stream.
- **Migration** (`migration.rs`): `GET /api/migrate/repo-vs-profile`.

## TonkState and dialog-reactor

The router's shared state is `Arc<RwLock<TonkState>>`. `TonkState` owns:

- the user's `Profile` and the derived `Operator` (both `dialog-operator`),
- a `Reactor` (re-exported from `dialog-reactor`) that caches repository/branch
  handles and runs the live query subscriptions; mutating routes flow through
  `reactor.repository(r).branch(b)` so subscription broadcasts happen
  automatically,
- a `CommandRegistry<CommandEnv>` (also from `dialog-reactor`) of typed-Rust
  command handlers fired by transient command concepts after a commit,
- the iframe bridge bookkeeping (`view_bindings`, `bridges`).

`dialog-reactor` is the branch layer: it was extracted from this crate and is
re-exported here as `tonk_worker::reactor` (and flattened), so `Reactor`,
`CommandRegistry`, and friends are usable directly off this crate.

## Host/guest routing model

A view is rendered in a sandboxed iframe. Routing policy lives entirely in
`on_fetch` / `route_for` (see [`worker.rs`](src/worker.rs)):

- `/api/...` from an ordinary client routes through axum unchanged.
- A registered guest iframe (recorded by client id against `{repo, branch}` via
  `GET .../host/{host}/{entity}`) gets a virtual root: its subresource fetches
  are rewritten under `/api/repository/{repo}/branch/{branch}/...`, so a fetch
  for `/foo.js` lands inside its branch.
- A view client hitting `/api/...` directly is rejected with a synthetic 404:
  the data plane is reachable only through the bridge, not from the iframe.
- The `/__tonk/bridge.js` module is exempt from rewriting so the iframe can
  install `globalThis.tonk`. View clients then talk to the worker over a
  transferred `MessagePort` (`onmessage` to `bridge::handle_message`), not over
  the data-plane routes.

Everything else passes through to the network (or the shell cache, via
stale-while-revalidate in [`cache.rs`](src/cache.rs)).
