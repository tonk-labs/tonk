# `<tonk-host>` — IO ownership and routing context for tonk elements

> **Status:** superseded by `plan/tonk-routing-attributes.md`. The host is
> no longer an element: `tonk_host::install()` attaches the operation
> listeners to `document` at boot, and the `<tonk-repository>` /
> `<tonk-branch>` annotators are replaced by the `with="branch@repo"`
> attribute. The event protocol below is unchanged.

## Problem

Three custom elements (`<tonk-display>`, `<tonk-concept>`,
`<tonk-layout>`) each open their own SSE subscriptions, run
their own phase-1 lookups, and POST their own `/evaluate` /
`/transact` documents. They build request URLs from their own
`space` / `branch` attributes; when those attributes are
absent the URL falls back to a relative `/query`, which only
works inside the iframe bridge — top-level pages return 405.

This causes a concrete bug today: a view template like the
todo-list example (a `<tonk-display>` whose template mounts
inner `<tonk-display>` elements for each item) renders the
inner displays without `space` / `branch`, and every per-item
phase-1 lookup 405s. The view fails as soon as it composes.

The root cause is broader than attribute forwarding: every
nested element duplicates IO setup, there is no place for
shared concerns to live (caching phase-1 descriptors, deduping
identical subscriptions, choosing transport, batching writes),
and switching the active branch — when we want to support
that — has no clean shutdown / rebuild story.

## Proposal

Introduce three custom elements that separate IO ownership
from routing context:

- **`<tonk-host>`** — the IO owner. Page-level singleton.
  Mounts once at app startup, outlives every route navigation.
  Owns transport selection (fetch / SSE / bridge), phase-1
  descriptor cache, subscription dedup, and the central
  registry of live consumer subscriptions. Has no `space` or
  `branch` attribute of its own.

- **`<tonk-repository name="…">`** — contributes a `space` to
  the routing context for its descendants. Mutable `name`
  attribute. Annotates outbound consumer events as they bubble
  past.

- **`<tonk-branch name="…">`** — contributes a `branch` to the
  routing context for its descendants. Mutable `name`
  attribute. Annotates outbound consumer events as they bubble
  past.

Nesting these mirrors the URL shape directly:

```html
<tonk-host>
  <tonk-repository name="home">
    <tonk-branch name="main">
      …consumer descendants (<tonk-display>, <tonk-concept>, <tonk-layout>)…
    </tonk-branch>
  </tonk-repository>
</tonk-host>
```

A descendant consumer never carries `space` / `branch`
attributes. It dispatches operation events on itself; ancestor
context elements annotate `event.detail` on the bubble; the
host catches the fully-annotated event and performs the IO.

`<tonk-host>` exposes four operations to its descendants:

- **subscribe** — open an SSE-style live subscription against
  a structured query form; deliver frames to the consumer.
- **query** — one-shot read against a structured query form
  (phase-1 descriptor resolution and similar one-time lookups).
- **claim** — write a structured `TransactRequest` form to
  `/transact` (see `plan/transact-endpoint.md`). Preserves
  concept-level classification (Durable / Transient)
  end-to-end.
- **evaluate** — write a raw asserted-notation document
  (YAML / dialog source) to `/evaluate`. The worker parses,
  analyzes, and commits.

`subscribe`, `query`, and `claim` carry pre-structured forms
across the wire. `evaluate` is the odd one out: it carries raw
notation text, intended for hand-authored YAML and
bootstrap-style seeding rather than for machine-issued
per-concept mutations.

## Event names

Five DOM event names form the wire contract. All bubble and
are composed (cross shadow boundaries cleanly). Names are
prefixed with `tonk-` to avoid colliding with generic event
names other libraries might claim:

- `tonk-subscribe` — open a live subscription
- `tonk-query` — one-shot read
- `tonk-claim` — write a structured `TransactRequest`
- `tonk-evaluate` — write a raw notation document
- `tonk-unsubscribe` — close a previously-opened subscription

`<tonk-layout>`'s existing effect events use the
`tonk-layout/*` namespace and are unaffected.

## Discovery via bubbling events

A descendant dispatches a `CustomEvent` on itself:

```js
const ev = new CustomEvent("tonk-query", {
  detail: { query: phase1Body },
  bubbles: true,
  composed: true,
  cancelable: true,
});
this.dispatchEvent(ev);
if (!ev.defaultPrevented) throw new Error("no <tonk-host> in ancestor chain");
const { entity, descriptor } = await ev.detail.result;
```

Three things happen on the way up:

1. **`<tonk-branch>` annotates `detail.branch`.** Bubble-phase
   listener: `ev.detail.branch ??= this.getAttribute("name")`.
   The `??=` means inner-most-wins — if an inner
   `<tonk-branch>` already stamped the event, an outer one
   leaves it alone.

2. **`<tonk-repository>` annotates `detail.space`.** Same shape:
   `ev.detail.space ??= this.getAttribute("name")`.

3. **Consumer-style elements annotate `detail.depth`.** Every
   element along the bubble path that is itself a consumer
   (mounts other consumers via templates or iteration)
   increments: `ev.detail.depth = (ev.detail.depth ?? 0) + 1`.
   By the time the event reaches `<tonk-host>`, `detail.depth`
   is the count of consumer ancestors between the dispatcher
   and the host. The host stores this depth alongside the
   registry entry; it drives the staggered-refresh strategy
   on context change (see "Context change and refresh").

`<tonk-host>` registers root listeners for each operation
event. On arrival it:

1. Calls `event.stopPropagation()` — nothing above the host
   needs to see this.
2. Calls `event.preventDefault()` — the dispatcher uses
   `event.defaultPrevented` as a "provider claimed it" signal.
3. Reads operation parameters and context annotations from
   `event.detail`.
4. Uses `event.target` as the consumer reference.
5. Performs the operation, writing the result back into
   `event.detail` (promises for one-shots; a subscription
   handle plus a `reset` / `update` / `error` method-call protocol
   for streams).

No callbacks cross the event boundary. The request rides in on
`event.detail`; the response rides out on the same
`event.detail`. Subscription frames are delivered by the host
calling well-known methods on the consumer element — one of
`reset(conclusions, opts)`, `update(delta, opts)`, or
`error(detail, opts)` depending on what the SW sent. See
"subscribe — host routes to `reset` / `update` / `error`"
below for the full contract.

## Annotators — who increments depth

Every element type that may sit between a consumer and the
host in the DOM, and which itself acts as a consumer (subscribes
or hosts iteration whose rows subscribe), installs a bubble-phase
listener for each of `tonk-subscribe`, `tonk-query`, `tonk-claim`,
`tonk-evaluate` that increments `detail.depth`. The list as of
v1:

- `<tonk-display>` — mounts nested displays via template
  iteration.
- `<tonk-view>` — the dumb renderer; its template can include
  consumer elements bound to iteration variables.
- `<tonk-concept>` — iterates its own template body.
- `<tonk-layout>` — once migrated; structurally similar.

`<tonk-host>` does not annotate — it's the terminal handler.
`<tonk-repository>` and `<tonk-branch>` do not annotate the
depth — they are passive context annotators, not consumers.

A bubbling event dispatched on an element does not trigger that
element's own bubble listener (bubble starts at
`event.target.parentNode`). So the dispatcher does not count
itself, which is correct: `depth` ends up as the number of
strict consumer ancestors. The structural distance.

When new consumer-style elements are added, they must install
the listener. The plan should explicitly call this out as part
of the contract.

## Result delivery

### query — `detail.result: Promise<…>`

One-shot read. The provider sets `event.detail.result` to a
promise that resolves with the response.

```js
// Dispatcher
const ev = new CustomEvent("tonk-query", {
  detail: { query }, bubbles: true, composed: true, cancelable: true,
});
this.dispatchEvent(ev);
const { entity, descriptor } = await ev.detail.result;

// Host
host.addEventListener("tonk-query", (ev) => {
  ev.stopPropagation();
  ev.preventDefault();
  ev.detail.result = this.performQuery(
    ev.detail.space,
    ev.detail.branch,
    ev.detail.query,
  );
});
```

### claim — `detail.result: Promise<TransactResponse>`

Same shape as query. POSTs a structured `TransactRequest` to
`/api/repository/{space}/branch/{branch}/transact`. Concept
classification (Durable / Transient) flows end-to-end without
re-parsing.

```js
const ev = new CustomEvent("tonk-claim", {
  detail: { request: transactRequest },
  bubbles: true, composed: true, cancelable: true,
});
this.dispatchEvent(ev);
const response = await ev.detail.result;
```

### evaluate — `detail.result: Promise<EvaluateResponse>`

Same shape as claim, against `/evaluate`. POSTs raw
asserted-notation bytes; the worker parses, analyzes, and
commits in one pass.

```js
const ev = new CustomEvent("tonk-evaluate", {
  detail: { document: notationYaml },
  bubbles: true, composed: true, cancelable: true,
});
this.dispatchEvent(ev);
const response = await ev.detail.result;
```

When to use which: `tonk-claim` for any machine-issued
mutation where the writer already knows the concept being
asserted — element event delegates, layout-effect writers,
transient assertions. `tonk-evaluate` when the input genuinely
is hand-authored notation text and needs the worker's
parse/analyze pass — bootstrap seeding, `<tonk-code>` cells,
paste-in YAML. The structured path is strictly more
informative end-to-end; prefer it whenever the writer has the
structure already.

### subscribe — host routes to `reset` / `update` / `error`

The consumer dispatches the event; the host sets
`event.detail.subscription = { cancel() {…} }` for explicit
teardown. Per-frame data flows back through method calls on
the consumer element itself. The wire format has two
semantically distinct payloads — snapshots and deltas — and
the host routes each to its own consumer method:

- **`reset(conclusions, opts)`** — full snapshot. The consumer
  must discard whatever prior state it held and reconcile its
  DOM against `conclusions` as if mounting fresh. Called on
  the first frame after subscribe, on context-change refresh,
  on reconnect after a disconnect that lost incremental state,
  and any other time the SW cannot produce a delta against
  prior state.
- **`update(delta, opts)`** — incremental change. The consumer
  applies `delta.added` / `delta.modified` / `delta.removed`
  against its existing mounted state. Called for steady-state
  frames within a continuous subscription where the SW can
  compute a delta.
- **`error(detail, opts)`** — the subscription is unhealthy.
  The consumer typically sets `data-state="error"` on its
  host. Not a frame; the host stops calling `reset` / `update`
  on this subscription until a recovery happens.

```js
// Dispatcher (a consumer element like <tonk-display>)
const ev = new CustomEvent("tonk-subscribe", {
  detail: { query, tag: "view" },  // tag optional
  bubbles: true, composed: true, cancelable: true,
});
this.dispatchEvent(ev);
const subscription = ev.detail.subscription;
// ...frames arrive via this.reset(...) / this.update(...) /
// this.error(...) , called by the host.

// Host
host.addEventListener("tonk-subscribe", (ev) => {
  ev.stopPropagation();
  ev.preventDefault();
  const consumer = ev.target;
  const { space, branch, query, tag, depth } = ev.detail;
  const entry = this.registry.add({ consumer, space, branch, query, tag, depth });
  entry.abort = this.openFrameStream({ space, branch, query }, (payload) => {
    if (!consumer.isConnected) { entry.abort(); return; }
    switch (payload.kind) {
      case "reset":  consumer.reset(payload.conclusions, { tag }); break;
      case "update": consumer.update(payload.delta,       { tag }); break;
      case "error":  consumer.error(payload.detail,       { tag }); break;
    }
  });
  ev.detail.subscription = { cancel: () => this.registry.drop(entry) };
});
```

The consumer publishes `reset`, `update`, `error` as stable
methods on its element class — a contract every consumer
implements. No function is *passed* across the boundary; the
host calls well-known methods on the event's target. Mirrors
the existing `<tonk-view>.render(conclusion)` contract,
generalized for two snapshot-vs-delta semantics.

Even a consumer with no incremental-update machinery can
implement the contract trivially: `reset` rebuilds, `update`
internally promotes the delta to a snapshot and calls its own
reset path, `error` records the error. Sophisticated consumers
(DBSP-aware ones, or iteration-keyed renderers like the
existing `MountedIteration`) can apply updates incrementally.

#### Ordering and contract guarantees

- The **first** call after subscribe is always `reset`. The
  consumer can assume no prior state exists before that call.
- `reset` followed by `update` is a sequence the consumer must
  process in order. The host invokes the methods synchronously
  from frame arrival; consumer methods are synchronous; DOM
  work is synchronous. Ordering is preserved by the call stack.
- After `error`, the host stops invoking `reset` / `update` on
  this subscription. If the SW reports recovery, the host
  invokes a fresh `reset` to re-baseline the consumer.

#### Tags — disambiguating multiple concurrent subscriptions

A consumer like `<tonk-display>` has more than one live
subscription (view + entity, or views-for-model + entity).
`<tonk-layout>` has three (workspace + focus + tiles). The
methods carry an `opts.tag` so the consumer can dispatch on
which stream the call belongs to.

The `tag` field in `detail` solves this: the consumer attaches
its own opaque label when subscribing, and the host round-trips
it on every method call. The consumer dispatches on `opts.tag`
inside each method.

```js
class TonkDisplay extends HTMLElement {
  reset(conclusions, { tag }) {
    switch (tag) {
      case "view":   return this.resetView(conclusions);
      case "entity": return this.resetEntity(conclusions);
    }
  }
  update(delta, { tag }) { /* same dispatch */ }
  error(detail, { tag }) { /* same dispatch */ }
}
```

Properties of the tag:

- **Optional.** Single-subscription consumers omit it.
- **Consumer-scoped.** Two different instances using the same
  string `"view"` don't collide; each host→consumer call is
  keyed by `event.target` plus the subscription handle.
- **Opaque to the host.** The host stores and returns it
  unchanged. Strings recommended for debuggability.
- **Per-subscription.** A consumer may open the same query
  twice with different tags if it wants two parallel streams.

#### Delta shape

The SW emits delta payloads with three optional collections,
each keyed by entity URI:

```
{
  added?:    Conclusion[],    // entities new to the result set
  modified?: Conclusion[],    // entities whose fields changed
  removed?:  string[],        // entity URIs no longer in the result set
}
```

`reset` simply takes the full snapshot:

```
conclusions: Conclusion[]
```

Empty result sets are represented faithfully: `reset([])` is a
valid call meaning "the subscription matches nothing."

### unsubscribe — no detail

The consumer's `disconnectedCallback` dispatches
`tonk-unsubscribe` on itself, no detail. The host uses
`event.target` to drop the consumer's entries from the
registry and aborts the underlying stream if no other
consumer is sharing it.

As a backstop, the host checks `consumer.isConnected` before
every `reset` / `update` / `error` call. A consumer that
detached without dispatching unsubscribe is silently dropped
on the next frame.

## API surface on consumer elements

The event protocol is the implementation detail. The intent is
that view authors and Rust callers see a higher-level API on
the element:

```js
// One-shot read
const { entity, descriptor } = await display.query(phase1Body);

// Subscribe — host drives this element's reset / update / error
// methods, optionally tagged so the consumer can dispatch.
const subscription = display.subscribe(query, { tag: "view" });
subscription.cancel(); // when done; happens automatically on disconnect

// Writes
await display.evaluate(notationDocument);
await display.claim(transactRequest);
```

Each method dispatches the corresponding `tonk-*` event on the
element and returns whatever the host wrote into
`event.detail`. The consumer no longer needs to know about
`space`, `branch`, fetch, SSE, or the bridge — the host owns
all of it.

The consumer **must** implement `reset(conclusions, opts)`,
`update(delta, opts)`, and `error(detail, opts)` as methods on
its element class to receive subscription frames.

## Phase-1 caching

`<tonk-host>` caches `phase1_lookup` results across all
consumers. Cache key is `(space, branch, source_name)`. A
view template that mounts fifty per-item `<tonk-display
model="todo">` issues one phase-1 for `"todo"` on
`(space, branch)`, not fifty.

Entries from a branch the user has navigated away from sit
idle until the host garbage-collects them (LRU eviction is
fine; can be deferred to a future optimization).

## Subscription dedup

When two consumers issue identical subscribe queries (same
`{space, branch, query}` after canonicalization), the host
opens **one** upstream SSE and fans frames out to both. The
N-todo-items case becomes one subscription per *kind* of
query, not per consumer.

The dedup table is ref-counted by consumer. The last
unsubscribe (or `isConnected === false` detection) closes the
upstream.

## Transport selection

`<tonk-host>` is the only place that decides between the
iframe bridge (`globalThis.tonk`) and the fetch/SSE path. Every
consumer is transport-agnostic. When the bridge migration
proceeds, only `<tonk-host>` changes.

## Mount point

A single `<tonk-host>` wraps the Leptos application root,
*outside* the `<Routes>` block, so it outlives every route
navigation:

```rust
view! {
    <tonk-host>
        <Router>
            <Routes ...>
                ...
            </Routes>
        </tonk-host>
    </Router>
}
```

Inside each route component, the route reads `:space` /
`:branch` from URL params and renders the routing wrappers:

```rust
view! {
    <tonk-repository name=space_name>
        <tonk-branch name=branch_name>
            <TonkDisplayView />
        </tonk-branch>
    </tonk-repository>
}
```

When the user navigates to a different `(space, branch)`, the
routing wrappers' `name` attributes change. `<tonk-host>`
outlives the change; it sees the routing elements'
attribute-change notifications and orchestrates a context
refresh of the affected subscriptions.

## Context change and refresh

The `name` attribute on `<tonk-repository>` and `<tonk-branch>`
is **mutable**. When it changes, the routing element's
`attributeChangedCallback` dispatches a `tonk-context-refresh`
event on itself (bubbling, composed, no detail beyond the
element identity). `<tonk-host>` catches it and orchestrates a
depth-staggered refresh.

### Refresh algorithm

The host walks its subscription registry, selects entries
whose `consumer` is a DOM descendant of the changed routing
element (`changedEl.contains(consumer)`), and groups them by
recorded `depth`.

Then for each depth from shallowest to deepest:

1. For each entry in this depth group, still present in the
   registry and with `consumer.isConnected === true`:
   - Abort the existing upstream subscription.
   - Re-issue the same query against the new
     (space, branch) — read the current values from the
     routing element annotations (the host can re-dispatch a
     dummy event on the consumer to capture annotations, or
     consult its known state of the routing tree directly).
   - Await the SW's first response. The SW cannot diff a
     fresh subscription against prior state, so the response
     is a `reset` payload by construction.
   - Call `consumer.reset(conclusions, { tag })`.
2. Between depth groups, yield to the microtask queue so any
   synchronous DOM diffs the consumer triggered (template
   iteration, child detachment) have a chance to fire
   `disconnectedCallback` → `tonk-unsubscribe` → registry
   drops the doomed entries.
3. Proceed to the next depth. Entries removed by the previous
   depth's pruning are simply absent; the host skips them
   without ceremony.

### Why depth-staggered

In the naive parallel-refresh, the host re-issues every
subscription at once. Children whose parent's iteration diff
would have pruned them anyway still incur one wasted upstream
round-trip plus a brief flash of stale-but-fresh data before
the parent's frame arrives and detaches them.

In the depth-staggered version, the parent's refresh runs
first. Its `reset` (or `update`) call triggers the iteration diff
synchronously. Doomed children detach, dispatch
`tonk-unsubscribe`, get dropped from the registry. The next
depth's pass simply doesn't see them.

The cost is N round-trips of latency in series-of-depths
rather than 1 in parallel. Branch switches are not hot-path
operations; this is acceptable.

### Sequential or parallel within a depth?

Within a single depth group, entries are independent — none of
them prunes a sibling. The host can issue them in parallel
and `Promise.all` their first-frame waits before moving to
the next depth.

### Consumer responsibility

The consumer's `reset` / `update` methods must run iteration diffs and
child detachment **synchronously**. This is already true today
(`MountedIteration` in `tonk-display/src/render.rs` is
synchronous DOM manipulation). Worth being explicit about: an
async consumer breaks the depth-staggered pruning, because
detachments wouldn't have fired by the time the next depth
pass runs.

### What if the entity doesn't exist on the new branch?

The re-issued subscription returns an empty result. The host
calls `consumer.reset([], { tag })`. The consumer treats this
as "no data" — whatever empty state it would have shown had it
mounted fresh against the new branch from scratch. If the
consumer is a row in a parent's iteration, the parent's own
refresh (one depth shallower) likely already detached it
before this call would have arrived.

### Pending writes during context change

A `tonk-claim` or `tonk-evaluate` issued against
(home, main, …) resolves after the user has navigated to
(home, staging, …). Two cases:

- Consumer is still mounted: the host resolves the promise
  with the response (success against the old context). The
  user's intent was against the old branch; surfacing the
  result is honest. The consumer may or may not still care.
- Consumer has been unmounted: the consumer's `await` is on a
  detached element. The promise resolves into the void. No
  harm.

The host does not attempt to retroactively re-route the write
against the new context. Writes are issued against the context
that was active at dispatch time.

## Migration

The three existing elements migrate one at a time. Each
migration is self-contained.

1. The element dispatches `tonk-*` events in
   `connectedCallback` instead of opening its own
   subscriptions.
2. The element exposes `reset(conclusions, opts)`,
   `update(delta, opts)`, and `error(detail, opts)` to
   receive subscription frames.
3. The element installs the `detail.depth++` bubble listener.
4. The element drops its `space` / `branch` attributes — the
   routing comes from ancestors annotating the event.
5. The element's `disconnectedCallback` dispatches
   `tonk-unsubscribe` for cleanup.

Order:

1. **`<tonk-display>` first** — unblocks the todo-list bug,
   the most exercised element today.
2. **`<tonk-concept>` next** — fewer call sites, similar
   shape.
3. **`<tonk-layout>` last** — needs `tonk-claim` and possibly
   `tonk-evaluate` for effect transactions, which lets us
   validate the write path end-to-end.

During the transition, the legacy attribute fallback can be
preserved for the iframe-bridge use case — children inside an
iframe bridge with no `<tonk-host>` ancestor still resolve via
their own SSE path. After migration, that fallback can be
removed.

## Risks and open questions

- **Re-issue against the new context — annotation source.**
  When the host refreshes a depth group, it needs the current
  (space, branch) for each entry. Two options: re-dispatch the
  subscribe event on the consumer (capture the annotations
  fresh) and intercept it server-side; or, have the host hold
  references to the routing-element ancestors of each
  subscription and read their current `name` attributes. The
  second is cheaper; the first composes more cleanly with the
  bubble-time annotator model. Probably the second, with the
  annotator chain captured at subscribe time.

- **Causal-vs-structural depth.** Depth is structural — number
  of consumer ancestors. Two depth-2 subscriptions might be
  causally unrelated, or a depth-2 subscription might depend
  on data from a depth-3 subscription (rare, via template
  bindings on iteration-produced values). The staggered
  refresh handles the structural case correctly; the rare
  causal-inverted case produces a transient flicker but
  eventual consistency. Revisit only if it bites in practice.

- **Pre-DBSP SW emits only `reset`.** The consumer contract
  is `reset` / `update` / `error`, but the SW does not yet
  track per-subscription state needed to compute deltas. In
  v1 the SW emits a `reset` payload on every frame; the
  consumer's reconciler (iteration-keyed on entity URI)
  handles per-frame DOM diffing as it does today. Once DBSP
  integration lands, the SW starts emitting `update` payloads
  for steady-state frames and `reset` for fresh subscriptions
  / reconnects / context changes. Consumers get the
  wire-efficiency benefit without code change.

- **Transactional batching.** Each `claim` / `evaluate` is one
  POST. Coalescing multiple writes into one document is a
  future optimization, not a v1 concern.

- **Phase-1 cache invalidation under schema edits.** Today's
  phase-1 doesn't change for a given source name on a stable
  branch, but schema edits could invalidate descriptors.
  Cache TTL? Subscription-based descriptor freshness? Defer
  until we hit this in practice.

- **`<tonk-host>` lifetime.** Page-level singleton, mounted
  outside `<Routes>`. The plan assumes it's never unmounted.
  If we ever need to (e.g. multi-tab compare across hosts),
  this assumption needs revisiting.

- **Cross-branch queries from one consumer.** A consumer under
  one branch cannot read from another. If we need that later,
  it becomes an explicit option on `subscribe` rather than
  ambient context.

- **New consumer-element types must install the depth
  annotator.** Documented contract; easy to forget. A small
  Rust helper / macro could enforce it.

## Non-goals (for v1)

- **Auth / identity scope** beyond what the worker already
  provides via `x-tonk-client-id`.
- **Transactional batching.**
- **Cross-branch queries.**
- **`disabled` flag on `<tonk-host>`.** No use case yet;
  context-change refresh is the cleaner story.
- **`<tonk-host>` unmount-and-remount.** Singleton for v1.

## Concrete v1 scope

- New `tonk-host` crate (or addition to `tonk-concept`'s
  surface; TBD) with the three elements + the five event
  types + the registry / cache / dedup machinery.
- `<tonk-display>` migrated; iframe-bridge path preserved as
  fallback for now.
- Leptos route mounts `<tonk-host>` outside `<Routes>`, and
  each route mounts `<tonk-repository>` + `<tonk-branch>` for
  its params.
- Smoke test: the journal's todo-list view renders end-to-end
  at the existing display route, with no per-item 405s and
  with a single phase-1 for `"todo"` in the network panel.
- Branch-switch smoke test: navigating between two branches
  on the same display URL refreshes subscriptions without
  page reload, shows the new branch's data, and produces no
  errors in the console.

Deliberately left for follow-up: `<tonk-concept>` and
`<tonk-layout>` migration, claim/evaluate batching,
schema-edit cache invalidation, the `disabled` semantics,
cross-host scenarios.
