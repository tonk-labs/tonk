# Adopt dialog's incremental subscriptions in the reactor, deltas end-to-end

## Goal

Replace the reactor's full-recompute-per-poll subscriptions with dialog's
demand-gated incremental `Subscription` (dialog PR #382,
`feat/incremental-retraction`). A poll that finds no change inside the
subscription's demand cover returns `None` and does zero query work; a poll that
changes touched entities maintains the result incrementally (DRed per-entity
re-derivation) instead of recomputing the whole query. Then carry the resulting
`Delta { asserted, retracted }` over the SSE wire so consumers (tonk-display and
friends) apply deltas to a retained set instead of re-diffing full frames.

This is the structural follow-through on the July 8 sync work: the no-op-pull
gate (#573) stopped polling on *unrelated* syncs; this stops the *poll itself*
from recomputing when a change misses the cover, and shrinks what a real change
costs to the touched entities.

## Decisions (settled)

- **Dialog source: prototype off the branch first.** Point tonk's Cargo at the
  local dialog worktree (`/Users/gozala/Projects/dialog-db/worker/pery`, already
  on `feat/incremental-retraction` @ `2511510e`) via `[patch]`, accepting a
  temporary regression of the #373 cache fix (that branch forks from upstream
  #298 and lacks it). Prove the reactor + display delta integration and stop
  there — the dialog-side reconciliation (merging `tonk-2026-07-07` in, cutting a
  tag) is explicitly out of scope and left to the user.
- **Wire format: deltas with full-set compat.** Reactor emits delta frames
  (`asserted`/`retracted`); the first frame per subscriber (and any
  post-reconnect frame) is a full snapshot so a fresh consumer needs no prior
  state. Consumers keep a retained set and apply subsequent deltas.

## What dialog gives us (verified on the branch)

- `Branch::subscribe<Q: Application>(&self, query: Q) -> Subscription<Q>`.
- `Subscription::poll(env) -> Result<Option<Delta<Q::Conclusion>>>`:
  - `Ok(None)` — branch unmoved, or moved but nothing intersects the demand
    cover (pin advances, no eval).
  - `Ok(Some(delta))` — (re-)evaluated; first poll reports the whole result as
    `asserted`. `delta.asserted` / `delta.retracted` are `Vec<Q::Conclusion>`.
- `Subscription::recomputes()` / `maintenances()` — poll-path counters, ready-made
  instrumentation for the perf measurement.
- Env bound on `poll` is exactly our `SelectProvider`. No new provider plumbing.
- Our subscriptions carry a `ConceptQuery`, and `ConceptQuery: Application` with
  `Conclusion = ConceptConclusion` — the same type our poll already selects and
  projects to the wire `Conclusion`. Clean mapping.

### One required dialog-side change (blocker)

`Subscription::poll` bounds `Q::Conclusion: PartialEq + Clone`. `ConceptConclusion`
derives `Clone` but **not** `PartialEq`. Its fields (`Entity`, `Parameters`,
`Match`) are all `PartialEq`, so this is a one-line `#[derive(PartialEq)]` on
`ConceptConclusion` in `rust/dialog-query/src/concept/descriptor.rs` on the
retraction branch. (dialog's tests only subscribe with `AttributeQuery` / typed
`Query<T>`, so they never exercised this bound with `ConceptConclusion`.)

## Stages

### Stage 0 — dialog dep + unblock
- `Cargo.toml`: add a `[patch."https://github.com/dialog-db/dialog-db.git"]`
  section pointing the 16 `dialog-*` crates at the local worktree paths (branch
  already checked out). Keep the `tag = "tonk-2026-07-07"` deps as the patched
  base.
- In the dialog worktree, add `PartialEq` to the `ConceptConclusion` derive;
  commit on the retraction branch (local prototype). `cargo check -p dialog-query`.
- `cargo build -p dialog-reactor` in tonk to confirm the new dialog API resolves.

### Stage 1 — reactor subscription state carries a `dialog_repository::Subscription`
`rust/dialog-reactor/src/subscription/state.rs` + `branch/state.rs`:
- Our `Subscription` gains a `dialog_repository::Subscription<ConceptQuery>` as
  its engine, replacing `query` + `last_hash` recompute. Keep `subscribers` +
  per-subscriber `Status`.
- The full-set snapshot for a new subscriber comes from dialog's
  `Subscription::results()`. Drop the hash-dedup — delta emptiness is the signal.
- `BranchState::subscribe(query)` builds `self.branch.subscribe(query)` once,
  registered under the same `QueryHash` identity (one dialog `Subscription` per
  distinct query, many subscribers).

**Overlay handling — push a transient `Changes` layer into dialog's demand
(decided):** the current poll folds the session overlay (`self.state.overlay()`)
into the read. dialog's `Subscription` builds its own `QueryEnv` internally at
`evaluate`/`maintain` (`let overlay = layer.overlay(&operator)` — schema metadata
only) and gates on the tree diff, so as shipped it can't see our session
overlay. Rather than a fallback, extend dialog's `Subscription` on the retraction
branch to carry a caller-supplied transient `Changes`:
- `Branch::subscribe_with(query, overlay: Changes)` (or `Subscription` gains an
  `overlay: Changes` field, `subscribe` = empty) stored on the `Subscription`.
- At both `evaluate` and `maintain`, merge that `Changes` into the `overlay` fed
  to `QueryEnv::new` and into `tombstones_from`, *before* `.with_demand(...)`.
  Because demand records at the `Select` boundary, the overlay-touched key ranges
  then land in the cover automatically — absence reads over overlay keys included.
- **Overlay-change re-trigger:** an overlay write doesn't move the tree, so the
  cover-gated diff won't fire on it. The reactor must re-seed the subscription's
  retained `Changes` and force a re-evaluation when the overlay changes. Concretely:
  `assert_overlay`/`retract_overlay`/`clear_overlay` push the branch onto
  `schedule_poll`; the poll updates the dialog `Subscription`'s stored overlay and
  runs a full evaluate (the overlay delta is off-tree, so incremental maintenance
  can't derive it). This confines full recomputes to actual overlay mutations —
  rare (invite seed, pause dot) — while tree-driven polls stay incremental.
- Reactor keeps mirroring `self.state.overlay()` into the dialog `Subscription`
  so the two never drift.

### Stage 2 — poll path emits deltas
`subscription/reference.rs::SubscriptionPoll::perform`:
- Call `dialog_subscription.poll(env).await`:
  - `Ok(None)` → nothing to broadcast (the win: no serialize/hash/broadcast),
    except Pending subscribers still need their first snapshot (below).
  - `Ok(Some(delta))` → project `asserted`/`retracted` via `Conclusion::project`
    into wire `Conclusion`s; build a Delta frame.
- **Pending → Snapshot:** a subscriber that attached since the last poll is
  `Status::Pending` and gets a full-snapshot frame from `results()`; Established
  subscribers get the Delta frame. Preserves "first event is the current
  snapshot."
- Instrument `recomputes()`/`maintenances()` deltas per poll via
  `tonk_common::log!` for the browser measurement.

### Stage 3 — wire frame shape (`tonk-schema`)
```
enum Frame {
  Snapshot { conclusions: Vec<Conclusion> },        // first frame, reconnect
  Delta { asserted: Vec<Conclusion>, retracted: Vec<Conclusion> },
}
```
- Reactor serializes `Frame` instead of a bare `Vec<Conclusion>`.
- Coordinated cutover: reactor + all consumers ship in one wasm bundle, no
  mixed-version wire. Confirm SSE is same-origin SW→page only (no external
  parser of the raw array).

### Stage 4 — consumers apply deltas (tonk-display + friends)
`tonk-display/src/element.rs` already keeps `last_frame: Vec<Conclusion>` and
diffs full frames. Rework the ingest (`on_message`, ~439):
- `Frame::Snapshot` → replace `last_frame` wholesale (today's behavior, now
  explicit); re-render.
- `Frame::Delta` → apply `asserted`/`retracted` to `last_frame` keyed by
  conclusion identity (same key `slide_keys` uses: `this` + terms), then run the
  existing render-diff. Internal slide-key diffing stays; only the input becomes
  a delta-updated set.
- Give the same Snapshot/Delta handling to the other terminal SSE consumers:
  `tonk-inspector` (element.rs), `tonk-fab` (dock/pause reads). The guest bridge
  (`tonk-portal`/`tonk-guest`) forwards SSE bytes without decoding — confirm it's
  byte-transparent so only terminal consumers need frame awareness.

### Stage 5 — measure (chrome-devtools MCP, real space)
- Log per-poll `recomputes` / `maintenances` / `None`-skip counts.
- Sync tick outside a view's cover → poll returns `None`, zero eval for that
  subscription.
- Single-field edit → `maintenances += 1` (per-entity re-derive), not a
  recompute, for supported queries.
- Note which of our views fall back to recompute (recursive/unsupported shapes).
- Confirm pause-dot / invite-overlay still update (Stage-1 fallback).
- Measure *incremental hit rate*, not absolute latency (branch lacks the cache
  fix until Stage 6).

### Stage 6 — stop at prototype-proven (no merge, no tag)
Deliberately **not** merging or tagging dialog here — that reconciliation is the
user's to do later. This effort ends with:
- The reactor + display delta integration working and measured against the local
  `[patch]` build (branch lacks the #373 cache fix, so absolute latency is not
  the yardstick — the incremental hit rate is).
- The dialog-side changes (the `ConceptConclusion` `PartialEq` derive and the
  transient-`Changes`-into-demand `subscribe_with` seam) committed on the local
  `feat/incremental-retraction` branch so they're ready to push when the user
  decides to reconcile `tonk-2026-07-07` in and cut a tag.
- Journal the adoption + measured impact. tonk `Cargo.toml` stays on the `[patch]`
  (not a tag) — flagged clearly as prototype wiring to swap once a tag exists.

## Verification

- `cargo fmt --all` + `cargo clippy --all --all-targets --all-features -- -D
  warnings`; native `cargo test`; `cargo test --no-run --target
  wasm32-unknown-unknown --workspace`.
- Reactor unit tests: cover-miss poll → `None`, no broadcast; Pending → Snapshot;
  Established → Delta; overlay write re-seeds the subscription's `Changes` and
  forces a re-evaluation.
- Display unit tests: applying a `Delta` to a retained set == the equivalent full
  `Snapshot` render set (delta/snapshot equivalence).
- Browser: hub + `/space/:did` render; edits propagate as deltas; cover-miss sync
  ticks cost nothing; pause-dot/invite overlay live.

## Risks / watchpoints

- **Overlay correctness** — the transient `Changes` fed into dialog's demand must
  reach *both* `evaluate` and `maintain`, or a maintained poll would drop the
  overlay facts. And the overlay-change re-trigger is load-bearing: an overlay
  write is off-tree, so without the reactor forcing a re-evaluate on
  assert/retract/clear the cover-gated diff never fires and the overlay update is
  silently missed.
- **Delta identity** — applying `retracted` needs a stable row key (the
  `slide_keys` identity); a mismatch leaks stale rows. Covered by the
  delta/snapshot-equivalence test.
- **Cache-fix regression while prototyping** — the branch lacks #373; measure hit
  rate, not latency. (No reconciliation in this effort — see Stage 6.)
- **Reconnect** — a reconnected SSE subscriber must be Pending so it gets a
  Snapshot; verify re-register sets Pending.
- **Persistent fallbacks** — a hot view that always recomputes gains nothing;
  Stage 5 flags them.
