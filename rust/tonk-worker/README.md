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
- **Invite / join** (`create_invite.rs`, `revoke_invite.rs`, `join.rs`): mint,
  list, and revoke invitations for a repo; visit or durably join from one full
  invite URL.
- **Profile** (`profile.rs`, `identify.rs`): `GET /api/identify`,
  `GET /api/profile`, and a parallel profile-as-repository surface
  (`/api/profile/branch/{branch}/{query,evaluate,transact}`) since the profile is
  its own repository outside the named-repo namespace.
- **Inspect** (`inspect/`): read-only views of branch state, remote/remote-branch
  status, and archive index blocks for debugging.
- **Host/guest bridge** (`host.rs`, `bridge.rs`): the iframe bridge (see below).
- **LSP** (`lsp.rs`, `lsp_env.rs`): exact
  `/api/{repository/{repo}|profile/{profile}}/branch/{branch}/language-server`
  endpoints. Each trusted route + client pair owns its server and SSE stream;
  portal clients use a bounded canonical chain of host-minted relay segments,
  and malformed or duplicate client headers are rejected before session lookup;
  accepted JSON-RPC shapes and nested URI/workspace fields are scope-checked,
  the environment adapter enforces the same reach before opening live data,
  and outbound diagnostics are filtered back to that scope only. There is no
  worker-global language-server route.
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

## Guest authority

An accountless guest holds no membership. What it holds is an audience-open
invite URL, retained locally, and one bounded delegation minted from it —
`subject -> ... -> operator`, capped at `VISIT_TTL_SECONDS` (one hour) by
`Invite::visit`. That bound never moves.

Because the bound never moves, the delegation is renewed rather than extended.
Before any remote operation presigns (`pull`, `push`, `sync`, `sync_status`, and
the sync drain), `ensure_session_authority` checks the signing session and every
retained guest record. If the session or any guest is due, it rotates the
operator once and replays every still-valid guest invite onto the new one: a
fresh key means a fresh audience, which is what keeps the retired chain from
being picked out of a content-addressed store that never deletes and never
consults the clock. Durable spaces need no replay — they reach the operator
through `space -> root -> device -> operator`, whose last hop the rotation
re-mints anyway.

Each guest record therefore stores which operator its live chain is addressed to
and when that chain lapses, alongside the URL. A record naming any other
operator is due immediately, whatever its expiry says, which is what makes a
service-worker restart heal on the next request instead of taking a 401. An
invite that has itself expired is not replayed: a guest hop cannot outlive the
chain it extends.

Renewal is local — parse, mint, retain — and adds no request to the account or
access service. Expiry and revocation stay where they were: the access service
checks them on the next ordinary remote call. Renewal only decides which
credential that call presents. And it is still not membership: explicit
promotion (`POST /api/repository/{repo}/membership`) remains the only path from
a guest to a durable member.

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

Everything else passes through to the network only when it is explicitly
non-cacheable, or reads this build's sealed shell cache via
[`cache.rs`](src/cache.rs). Install is the only generation-cache writer and
verifies the build-published shell/UI/lazy/guest graph before activation. A
cached response is never revalidated or overwritten; an eviction miss returns
an actionable `503` online and offline instead of accepting live stable-name
bytes under an older controller. A stamped production worker ignores authored
`no-store`, `reload`, and `no-cache` flags for same-origin static resources;
only the unstamped development worker treats those flags as a live-network
bypass for Trunk hot reload.

## Browser contracts

Browser JSON is camelCase. `POST /api/repository/{repo}/invite` accepts
`baseUrl` and `recipientRoot`; omitted `baseUrl` becomes `/join` on the exact
request origin. The input aliases `base_url` and `recipient_root` remain only
for rollout compatibility and are scheduled for removal no earlier than
2026-08-29. Unknown fields return 400.

Access and revocation relays are separate explicit metadata. A remote without a
stored revocation relay remains readable and syncable, but cannot mint a
remotely revocable invitation. Invitation list responses contain the target
CID, audience kind, optional recipient root, and display status only—never
delegation bytes, relay URLs, seeds, or bearer links.

Service calls use an explicit media type: UCAN invocation containers and signed
revocation artifacts use `application/cbor`; JSON-only operations use
`application/json`. Both native and Wasm transports use a ten-second timeout,
bound error bodies, and preserve structured upstream status and code.

Successful sync responses are 2xx with disposition `completed`, `offline`, or
`paused`. Failures are non-2xx and carry stable codes:

| HTTP | code |
| ---: | --- |
| 403 | `CREDENTIAL_REVOKED` |
| 409 | `SYNC_CONFLICT` |
| 503 | `SYNC_UNAVAILABLE` |
| 502 | `UPSTREAM_ERROR` |

Clients accept legacy `DEVICE_REVOKED` as credential revocation during the
rollout. Revoked, conflict, unavailable, browser-offline, and paused states are
published and rendered distinctly.
