# tonk-host

`<tonk-host>`, `<tonk-repository>`, `<tonk-branch>`: IO ownership and routing
context for tonk custom elements.

These three WASM custom elements give descendant tonk elements (such as
`<tonk-display>`) a place to send their reads and writes. Consumer elements never
touch transport directly: they dispatch operation events on themselves, those
events bubble up through `<tonk-repository>` and `<tonk-branch>` (which stamp
which repository and branch the request targets), and a `<tonk-host>` ancestor
catches them and performs the actual IO. Wrapping a subtree in a different
repository/branch pair rescopes every consumer inside it, with the innermost
wrapper winning.

The element implementations are `wasm32`-only; the target-independent surface
(`error`, `events`, `ready`) compiles everywhere so consumer crates can run their
native tests against the same wire contract. `register()` installs all three
elements (idempotent).

## `<tonk-host>`

The IO owner. A page-level singleton, mounted outside `<Routes>`. It owns:

- Transport selection (fetch / SSE).
- A phase-1 descriptor cache and an LRU `tonk-query` response cache (shared
  across every consumer in the page, invalidated per-branch on each claim or
  evaluate).
- The registry of live consumer subscriptions, keyed by consumer element
  identity, recording each subscription's `depth` for staggered refresh.

It listens for the operation events as they bubble up, performs the request, and
writes the result back onto the event detail. It also handles
`tonk-context-refresh` (dispatched by routing elements when their attributes
change) by re-running the affected subscriptions in depth order.

## `<tonk-repository name="…">`

A passive annotator. No IO. In bubble phase it stamps `detail.space` with its
`name` attribute on each operation event, but only if not already set, so an
inner `<tonk-repository>` wins over an outer one. With a `profile` attribute it
also stamps `detail.profile`, routing descendant queries to the
profile-as-repository endpoint.

## `<tonk-branch name="…">`

The same annotator pattern as `<tonk-repository>`, writing `detail.branch` from
its `name` attribute instead. Inner-most-wins. Changing its `name` dispatches
`tonk-context-refresh` so the host refreshes subscriptions scoped to it.

## Operation events (the consumer/host protocol)

Consumers dispatch one of five `CustomEvent`s on themselves. All bubble and are
composed; the four operations are cancelable, and the host signals it handled an
event by calling `preventDefault()` (consumers read `defaultPrevented` to detect
that a `<tonk-host>` ancestor exists). Event names live in the `events` module:

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
or drop.

Helper consumer elements (those that mount other consumers via templates or
iteration) install a depth annotator via `install_depth_annotator` /
`DepthAnnotator`. It increments `detail.depth` in bubble phase, so by the time an
event reaches the host, `depth` is the number of consumer ancestors between the
dispatcher and the host. The host uses this for staggered refresh.

## Modules

- `events`: the operation event-name constants and the `OPERATIONS` set.
- `error`: `ErrorDetail` / `ErrorKind`, the data types returned on failure
  (target-independent).
- `consumer`: Rust-side helpers for dispatching operation events and reading
  results (`wasm32`).
- `depth`: the `detail.depth` annotator (`wasm32`).
- `ready`: service-worker readiness gate (`wait()`; a no-op on native).
- `bridge`, `sse`: transport surfaces (`wasm32`).

See `plan/tonk-host.md` at the repository root for the design.
