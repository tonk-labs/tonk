# Delta-native rendering + revision-stamped frames (drift detection & depth ordering)

## Context

Incremental subscriptions now emit `Delta { asserted, retracted }` over SSE, but
the consumer side accumulated the same delta-application logic in **four** places
that can drift out of sync: `tonk-display` `apply_delta` (raw rows) + `last_frame`
(folded), the portal bridge `subRows` (JS, `window.tonk.subscribe` sugar), and
`ui-sync-status` `on_delta`. A stale/dropped delta silently corrupts a consumer's
retained set (the counter-shows-stale-value bug). Root causes:

1. **Duplicated, drift-prone retained state** across layers, each re-implementing
   "apply a delta to a set".
2. **No frame sequencing** — a consumer cannot detect a dropped/out-of-order
   frame; it silently diverges until the next reconnect snapshot heals it.
3. **Full re-render on every frame** — the display reconstructs the full set and
   re-renders, discarding the delta's "what changed" info, even though the
   browser renderer is already a keyed in-place reconciler.
4. **Directory instance-removal waste** — a data delta that retracts a directory
   instance removes a nested `<tonk-display>` (and its subscriptions); the reactor
   can broadcast a doomed child's delta *before* the parent removes it (arbitrary
   `QueryHash`/branch poll order), wasting a render on a node about to vanish.

Intended outcome: one authoritative retained set per consumer, deltas applied
in-place by the already-keyed renderer, drops detected and self-healed, and
parent-before-child (depth) ordering so doomed leaves aren't updated.

## Verified pipeline facts (do not re-assume)

- **Host page is a dumb fetch-stream proxy.** Elements subscribe via plain
  `window.fetch` SSE; the portal bridge relays a *streaming fetch* up one level
  (transfers the `ReadableStream`), it does NOT relay a `subscribe`. Each nested
  guest runs the REAL host (`tonk_host::install_io`) and parses SSE locally.
  `tonk-portal/src/bridge.rs` `handle_host_fetch`; `tonk-host/src/sse.rs:125-157`.
- **`<tonk-site>` is a host/portal**, the recursive routing unit
  (`tonk-portal/src/site.rs`), not a guest.
- **Dedup + per-subscriber reframing already exist — ONLY at the reactor.** One
  engine per `(branch, query)` via `QueryHash`; a new/reconnected subscriber is
  `Pending` → gets a full `Frame::Snapshot`, `Established` → gets `Delta`.
  `dialog-reactor/src/subscription/{state.rs,reference.rs}`,
  `branch/state.rs:146-172`. No host/bridge/site-level dedup or reframing exists,
  and none is needed for correctness.
- **`subRows` (bridge.rs) is only the `window.tonk.subscribe` sugar path** — no
  element uses it. Red herring for displays (but still has the drift bug; already
  patched to match the display heal, uncommitted).
- **Depth is already computed client-side** by bubble-phase annotation:
  `tonk-host/src/depth.rs` increments `event.detail.depth` per consumer ancestor;
  stored as `Entry.depth` (`registry.rs`). Today used only by `refresh_under`
  (structural refresh), shallow-first with a microtask yield
  (`ops.rs:592-632`) so a parent's reset can unmount children (which unsubscribe)
  before deeper groups run.
- **No frame carries a revision/epoch today.** But a commit HAS a `revision`
  (`revision_after`), and the branch broadcast `Notification` already carries it.
  A poll triggered by commit R computes its delta at revision R — a natural
  per-commit correlator that just isn't stamped on the `Frame`.

## Design

Two numbers do most of the work: **the branch revision on every frame**.

### 1. Stamp `Frame` with the branch revision (reactor)
`Frame::Snapshot`/`Delta` gain a `revision` (the branch state the frame was
computed at). Cheap: the reactor already holds it. Same-branch subscriptions
fanning out from one commit share the revision → a natural epoch. Files:
`tonk-core/src/conclusion.rs` (Frame enum), `dialog-reactor/src/subscription/reference.rs`
(stamp on Snapshot/Delta build).

### 2. Consumer gap-detection + self-heal (drops)
The consumer tracks the last applied revision. A delta whose revision is not the
expected next step (gap, or out-of-order) → **close + reopen the SSE** → the
reactor sends a fresh `Snapshot` by construction (new subscriber = `Pending`).
Reuses the existing reconnect→snapshot path (`ops.rs:714-748`, `reconnect:true`).
No new server machinery. This is the "generation counter" realized as the real
revision.

### 3. Reactor-side priority dispatch, order-only (anti-waste)
Client-side ordering does NOT work: a parent's and child's deltas arrive over
independent SSE streams as separate async network events, so the host cannot hold
a leaf frame waiting for a shallower one it doesn't know is coming — the child may
be applied before the parent's frame even arrives. **Only the reactor sees a
commit's whole fan-out in one place** (`run_scheduled_polls` drains all affected
`BranchState`s together, `lib.rs:121-140`), so only it can order the emission.

- **Subscribe carries `priority` (= depth).** The display already computes depth
  by bubble annotation (`depth.rs`); it travels in the subscribe request to the
  reactor and is stored on the subscriber (`SubscriberSession`,
  `subscription/state.rs:96`; reactor has no depth today).
- **`run_scheduled_polls` dispatches shallow-first** across the whole drained
  batch — low priority number (outermost) first. A parent's pruning delta is
  SENT before any child delta.
- **Order-only, no round-trip wait.** The reactor does not block waiting for a
  child to unsubscribe (that needs a full client round-trip and would couple the
  reactor to client timing). It only guarantees send-order. The client receives
  frames in priority order and does the pruning: the parent's delta applies,
  unmounts the doomed child (→ `tonk-unsubscribe`), and any child delta that
  already slipped through is dropped at the client because its consumer element
  is detached / its registry entry is gone. Waste is avoided at APPLY time by
  the parent-first receipt order, not by withholding the send.
- **Client robustness (required for order-only) — ALREADY DONE.** The SSE frame
  loop already drops any frame whose consumer element is detached:
  `ops.rs:502-504` `if !consumer_frame.is_connected() { return; }` runs before
  `deliver_frame`. So a late child frame after the parent unmounted it is a
  no-op with no new work. This is the safety net order-only relies on.

### 4. Delta-native renderer + one retained set (display)
The browser renderer (`tonk-display/src/render.rs`) is ALREADY a keyed in-place
reconciler: `Renderer.mounted` persists across frames; `MountedRepeat.rows` keyed
by `this`; iterations keyed by value; `apply()` patches only changed nodes. Give
`<tonk-view>` the two methods originally intended:
- **`draw(state)`** — render a (possibly fresh) template from the full retained
  state. Used on mount, template/slide swap, and reconnect snapshot. (Template
  swap REQUIRES a retained copy that outlives the disposable DOM — see below.)
- **`update(delta)`** — apply `{asserted, retracted}` directly to the keyed
  reconcile: asserted → `build_repeat_row`+insert or `update_nodes`; retracted →
  `remove_child`+map remove; tuple-level via `update_iteration`. No full-frame
  reconstruction.

Collapse the display's TWO copies (`retained` raw + `last_frame` folded) into
**one** authoritative retained set that (a) survives template swaps (replay
source — `element.rs:1290` re-renders a new slide from it) and (b) is what deltas
apply to. Delete `apply_delta`/heal from `element.rs` (the keyed reconcile
subsumes it). Chrome binds to the **scoped entity**, not `frame.first()`
(positional-lead is the wrong model — a `{count}` updates when that entity's
count changes).

Retained state stays **display-local** (not cached in tonk-site): the reactor is
the authoritative shared copy and re-subscribing for a fresh snapshot is a cheap
fetch. A site-level dedup/edge-cache is deferred until profiling shows duplicate-
subscription fan-out is a real cost (it would have to re-implement the reactor's
per-subscriber reframing — a third place to drift).

## Build sequencing (phased, each independently shippable)

- **Phase 1 — Reactor priority dispatch (order-only).** Depth→`priority` on the
  subscribe request; store on the reactor subscriber; `run_scheduled_polls`
  dispatches shallow-first across the drained batch. Fixes the directory
  instance-removal waste. Reactor + subscribe wire only; client already drops
  detached-consumer frames (`ops.rs:502-504`).
- **Phase 2 — Revision-stamped frames + gap-detect/reopen.** Stamp `Frame` with
  the branch revision; consumer tracks last-applied revision, reopens SSE on a
  gap → fresh snapshot. Drift self-heals regardless of the renderer. This is the
  standalone safety net.
- **Phase 3 — Delta-native renderer + single retained set.** The `draw`/`update`
  split on `<tonk-view>`; collapse the display's copies into one retained set;
  delete `apply_delta`/heal; chrome-by-entity. The core rewrite.
- **Phase 4 (optional) — concept-aware attr-map.** Refine the retained set into a
  concept/cardinality-typed `entity → {attr: value|[values]}` map (vs keyed
  conclusion rows). Cleaner end state; requires the display to consume concept
  definitions.
- **`ui-sync-status`** folds onto the shared consumer path in Phase 3 (drops its
  ad-hoc cardinality-one shortcut).

## The counter bug is NOT actually fixed (verified live 2026-07-10)

The committed `apply_delta` heal (`9a85c4b7`) is deployed (trunk rebuilt the
guest bundle after the commit) but does NOT fix the real bug: the durable count
is 4 while the display shows 3 — a persistent 1-behind drift, and a fresh
evaluate to 4 did not propagate. The `apply_delta` unit tests pass, and the
full-DOM integration test passes, yet the live path stays stale. So the failure
is NOT in `apply_delta`'s merge logic — it is somewhere the tests don't exercise:
the delta either does not reach `on_update` on the live SSE path, or reaches it
and the render does not reflect the healed set. **Unresolved — the minimal fix
still has to be found by instrumenting the live guest path** (needs a log in
`on_update`/`deliver_frame` to see whether the entity delta arrives with its tag
and what `apply_delta` produces on the real frame). Do this OUTSIDE plan mode.

Candidate suspects to check first (evidence-ordered):
- Does the entity subscription's DELTA arrive with the `entity` tag, or untagged
  → dropped at `on_update` (`element.rs:513` `let Some(tag) = tag else return`)?
- Is a real `JSON.parse`'d frame's `asserted`/`retracted`/`fields` deserialized
  correctly by `read_conclusions` (the `fields`-as-Map-vs-object gotcha)?
- Does the display even render a fresh entity frame, or is `handle_entity_frame`
  short-circuiting (e.g. reconnect-empty hold, or the double-mount)?

## Also landed this session (context)

- Bridge `subRows` heal (uncommitted) — matches the display heal; same caveat
  (the display path doesn't use it, so it doesn't fix the counter).
- `/evaluate` poll-drain (committed) — a reactor test showed the inline
  `session.poll()` already covers the plain route, so this is redundant/defensive;
  candidate to revert.
- Regression tests (all green, but they test the layers in ISOLATION and did NOT
  catch the live failure): reactor self-consistency, `deliver_frame` routing (5),
  `apply_delta` supersession/drift/multi-row, reconnect re-sync, full
  reactor→display DOM integration. The gap: no test drives a REAL SSE frame
  through the guest's live `on_update` — the integration test called `on_update`
  directly, bypassing whatever the live path does differently.

## Verification

- Reactor/frame: `nix develop --command bash -c "cargo test -p tonk-worker
  --target wasm32-unknown-unknown it_"` (revision stamping, gap→reopen→snapshot).
- Display: `nix develop --command bash -c "cargo test -p tonk-display --target
  wasm32-unknown-unknown --lib it_"` (draw/update, keyed reconcile, template-swap
  replay, chrome-by-entity).
- Browser (chrome-devtools MCP): the counter case — evaluate a supersession,
  confirm the DOM count updates in place (no flush, no stale value) and no
  duplicate rows; a directory instance removal does not render the doomed child.
