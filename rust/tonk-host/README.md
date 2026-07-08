# tonk-host

IO ownership and `with="branch@repo"` routing context for tonk custom
elements. There is no host element: `install()` runs once at app boot and
attaches the operation-event listeners to `document`.

Consumer elements (such as `<tonk-display>`) never touch transport directly:
they dispatch operation events on themselves, those events bubble to the
document, and the installed host performs the actual IO. The routing context
comes from the `with` attribute — the nearest ancestor (including the
consumer itself) carrying `with="branch@repo"` decides which repository and
branch the request targets, innermost wins. Wrapping a subtree in a
different `with` rescopes every consumer inside it.

The implementation is `wasm32`-only; the target-independent surface
(`error`, `events`, `location`, `ready`) compiles everywhere so consumer
crates can run their native tests against the same wire contract.

## `install()`

Called once at boot (idempotent). It owns:

- Transport selection (fetch / SSE).
- The registry of live consumer subscriptions, keyed by consumer element
  identity, recording each subscription's `depth` for staggered refresh.
- The navigate provider (worker-requested redirects) and the idle-sync
  heartbeat.
- A document-level `MutationObserver` on the `with` attribute: when a
  routing context changes (a re-stamped template row, a rewritten
  wrapper), the affected subscriptions re-issue in depth order.

It listens for the operation events at the document, resolves the route
(explicit detail fields first, else the dispatcher's `with` ancestry),
performs the request, and writes the result back onto the event detail.

## The `with` / `allow` grammar (`location` module)

A location is `branch@repo`: `main@did:key:zAlice`, `did:key:zAlice` (bare
repo ⇒ its default branch), or `meta@profile:tonk` (a `profile:<name>` repo
token targets the profile-as-repository endpoint). An `allow` list — read
by `<tonk-site>` / the portal bridge, not by the host — is `*` or a set of
explicit locations. Target-independent, natively tested; parse at connect,
error on malformed. See `plan/tonk-routing-attributes.md`.

Inside a sealed guest the same events are caught by the guest relay
(`tonk-guest`), which resolves `with` identically and forwards the location
over the bridge; the trusted portal side enforces its `allow` list there.

## Operation events (the consumer/host protocol)

Consumers dispatch one of five `CustomEvent`s on themselves. All bubble and
are composed; the four operations are cancelable, and the host signals it
handled an event by calling `preventDefault()` (consumers read
`defaultPrevented` to detect an installed host). Event names live in the
`events` module:

- `tonk-query`: one-shot read. Detail carries `query`; the host writes back a
  `result` promise.
- `tonk-claim`: write a structured transact request. Detail carries `request`.
- `tonk-evaluate`: write a raw asserted-notation document. Detail carries
  `document` and a `transact` flag (`false` previews what the document would do
  without committing; `true` commits).
- `tonk-subscribe`: open a live subscription. Detail carries `query` and an
  optional `tag`; the host writes back a `subscription` handle (with a `cancel`
  function). Frames are delivered by calling `reset` / `update` / `error` methods
  on the consumer element.
- `tonk-unsubscribe`: close a previously opened subscription.

The `consumer` module provides Rust helpers (`query`, `claim`, `evaluate`,
`subscribe`, `dispatch_unsubscribe`) that build and dispatch these events and
read the results, returning a `Subscription` handle that tears down on `cancel()`
or drop. The `*_with_route` variants pre-fill an explicit route on the
detail, which wins over `with` resolution (used by the portal bridge when
relaying a guest's already-judged route).

Helper consumer elements (those that mount other consumers via templates or
iteration) install a depth annotator via `install_depth_annotator` /
`DepthAnnotator`. It increments `detail.depth` in bubble phase, so by the time an
event reaches the host, `depth` is the number of consumer ancestors between the
dispatcher and the host. The host uses this for staggered refresh.

## Modules

- `events`: the operation event-name constants and the `OPERATIONS` set.
- `error`: `ErrorDetail` / `ErrorKind`, the data types returned on failure
  (target-independent).
- `location`: the `Location` / `Allow` grammar (target-independent).
- `consumer`: Rust-side helpers for dispatching operation events and reading
  results (`wasm32`).
- `context`: `resolve_with` — the nearest-`with`-ancestor resolver (`wasm32`).
- `depth`: the `detail.depth` annotator (`wasm32`).
- `ready`: service-worker readiness gate (`wait()`; a no-op on native).
- `bridge`, `sse`: transport surfaces (`wasm32`).

See `plan/tonk-routing-attributes.md` at the repository root for the design.
