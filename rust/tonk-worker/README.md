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

The HTTP surface is pinned in [`router/route_table.rs`](src/router/route_table.rs)
and is deliberately small. A page reads with a query and acts with a command it
transacts; the worker answers with facts the page queries or subscribes to.

- **Query** (`query.rs`): `POST .../branch/{branch}/query` takes a serialized
  `ConceptQuery` and returns conclusions. With `Accept: text/event-stream` the
  response is an SSE subscription that re-broadcasts on every branch change.
- **Transact** (`transact.rs`): `POST .../branch/{branch}/transact` takes a
  typed `TransactRequest`, bypassing notation so per-mutation
  transient/durable classification flows straight to the reactor's transaction
  builder. A transient concept the worker registers is a command: its provider
  runs after the commit (see [`router/command.rs`](src/router/command.rs)).
- **Evaluate** (`evaluate.rs`): `POST .../branch/{branch}/evaluate` accepts an
  asserted-notation document (any mix of queries and mutations), runs the
  analyze to query to plan to commit pipeline (via `tonk-evaluator`), and
  returns matches plus a commit summary.
- **Blob** (`blob.rs`): `GET .../branch/{branch}/blob/{entity}` serves an
  entity's bytes with their content type (what `<img src>` points at), and
  `POST .../branch/{branch}/blob` ingests an upload. Raw bytes are not facts,
  so these stay routes.
- **LSP** (`lsp.rs`, `lsp_env.rs`): a language-server surface merged into the
  router, carrying its own `LspHub` state and an SSE event stream.

Each route exists for a space (`/api/repository/{repo}/branch/{branch}/…`) and
for the profile, which is its own repository outside the named-repo namespace
(`/api/profile/branch/{branch}/…`).

Everything else is a command: creating, joining, and inviting to spaces;
account, passkey, and device ceremonies; sync pause; profile switching;
onboarding; and the inspector's diagnostics (`InspectBranch`). When a page has
to wait on one outcome, the command carries an `at` stamp and the worker
answers on a `state:*` overlay row carrying the same stamp. Background sync is
paced by the page's `{type:"keepalive"}` message, not a route. Read
`.claude/skills/commands-not-routes/SKILL.md` before adding a route.

- **Host/guest bridge** (`host.rs`, `bridge.rs`): the iframe bridge (see below).

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
promotion (the `PromoteMember` command) remains the only path from
a guest to a durable member.

## Host/guest routing model

A view is rendered in a sandboxed iframe. Routing policy lives entirely in
`on_fetch` / `route_for` (see [`worker.rs`](src/worker.rs)):

- `/api/...` from an ordinary client routes through axum unchanged.
- A registered guest iframe (recorded by client id against `{repo, branch}` in
  the worker's view bindings) gets a virtual root: its subresource fetches
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

## Browser contracts

Browser JSON is camelCase. An invite is the `InviteRequest` command, asserted on
the profile branch or on the space's own branch; its outcome lands as facts the
share control subscribes to.

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
