# A steady interface

What you have on screen stays on screen until something newer replaces it.
Nothing you already have waits on the network. Updates land at boundaries,
never under the person using it.

This plan comes out of a read of why spaces keep falling back to the pulsing
dot, why the same action is fast one time and slow the next, and why the
page sometimes reloads by itself. Every mechanism below is cited to code;
where something is inferred rather than read, it says so.

## What people see, and what causes it

### The pulsing dot over a space that was already showing

The dot is the placeholder the top `tonk:site` display shows in its
`loading`, `no-entity` and `no-model` states
(`rust/tonk-portal/src/site_content.rs:34`). A space falls back to it when:

1. **The service worker restarted and lost the tab's site stamp.** The stamp
   (`tonk:site`: which route this tab shows) lives only in the worker's
   in-memory overlay (`rust/tonk-worker/src/router/session.rs:411-487`). The
   browser stops an idle worker after about 30s. The page's keepalive is the
   only thing preventing that, and it is skipped whenever
   `navigator.onLine` is false (`rust/tonk-host/src/host.rs:125-165`), although
   a fetch to the worker never touches the network. Background tabs throttle
   the timer too. After a restart the reconnected subscription answers empty,
   `<tonk-display>` holds that empty frame for 5s (`reconnect_grace`,
   `rust/tonk-display/src/element.rs:2466-2493`, `:3245`), and if the heal
   claim plus a full worker boot take longer, it clears to the dot.
2. **A view frame came back empty or without its facet.** A worker that has
   not loaded the repository yet answers with the empty set. View frames
   have no grace at all: the slide is removed at once
   (`element.rs:1661-1678`) and rebuilt when the real frame arrives.
3. **A deploy reached an open tab.** On `controllerchange` the page either
   reloads, or removes and re-inserts every top-level `<tonk-site>`
   (`remountGuests`, `rust/tonk-ui/index.html:510-532`), which tears down
   and reboots every guest frame. Any change to any shipped asset produces a
   new worker (`rust/tonk-ui/scripts/stamp-service-worker.sh:269-286`), and
   it activates under every open tab through `skipWaiting()`.
4. **Switching from one space to another boots a new guest.** `<tonk-site>`
   rebuilds its iframe whenever `with` or `allow` changes. Until the fix on
   this branch it did so two or three times per switch, because each
   attribute write decided reuse against stale state.
5. **A display treats a transient answer as "clear everything".** Any
   non-offline error (`fail()`, `element.rs:1041`), a changed descriptor
   (`:1299-1303`) or a changed view template (`:1742-1753`) removes every
   rendered child, and the next good frame builds them again.

The common shape: absence, error and reconnect are all read as "remove what
is rendered". The renderer itself patches in place and never moves nodes
(`rust/tonk-display/src/render.rs`). The removals come from the layer above
it deciding that its content is gone.

### Fast one time, slow the next

1. **Reads fetch missing tree nodes from the remote, mid-query, with no
   timeout.** A space branch is a partial replica: a pull adopts the
   upstream head by reference and downloads nothing
   (`dialog-db/rust/dialog-repository/src/repository/branch/download.rs:165-176`).
   A read that misses a block hydrates it through the tracked remote
   (`dialog-db/rust/dialog-repository/src/repository/archive/networked.rs:186-191`).
   Only the profile, access and account branches download before their
   head moves (tonk #1167, `rust/dialog-reactor/src/pull.rs:94-145`). So
   after every background pull, the next read of anything another device
   changed goes to the network. The S3 transport has no request timeout
   (`dialog-db/rust/dialog-remote-s3/src/client.rs:18-24`), so a stalled
   connection hangs the read instead of failing it.
2. **One global lock is held across network round trips.** Worker state is
   an `Arc<tokio::sync::RwLock<TonkState>>` (`rust/tonk-worker/src/router.rs:218`).
   Sync, account sweeps and boot chores hold its read guard across remote
   pulls and pushes (`router/sync.rs:650-656`, `:1319-1334`,
   `worker.rs:2057`). Join, space create and remove, blob serve and
   session renewal take the write guard, some across network work
   (`join.rs:487`, `repository.rs:263`, `blob.rs:233`). Tokio's lock is
   write-preferring, so one queued writer stalls every later query and
   transact until the slow sync finishes. Every request also takes the read
   guard twice before routing (`router/bridge.rs:675-708`, `worker.rs:127`).
   (The queueing is inferred from tokio's documented fairness, not measured.)
3. **Subscription polls on a branch are sequential**
   (`rust/dialog-reactor/src/branch/state.rs:287-334`). One poll waiting on a
   remote node delays every frame on that branch, the site stamp included.
4. **Worker boot can wait on a remote before it answers any request.**
   `reconcile_root_delegation` commits under the profile transactor with a
   remote-capable environment (`router/identity.rs:155-177`, `:395-424`).
   Before the offline generation is adopted, the profile library is fetched
   with `no-store` on every boot (`router/repository.rs:5548-5569`). A boot
   runs again after every worker restart, which point 1 above makes common.
5. **Deploy choreography.** While a successor is installed, page IO is held
   up to 10s (`index.html:441-469`) and streams reconnect after 20-30s
   unless `controllerchange` arrives first (`rust/tonk-host/src/ops.rs:112-146`).

### The page reloads by itself

1. **A deploy** whose page build changed (`requestAlignmentReload`,
   `index.html:496-505`).
2. **The boot watchdog** reloads after 30s without boot progress
   (`index.html:900-999`). The worker's own boot does not count as progress,
   so a slow worker boot (above) reloads the page once.
3. **`tonk.reload()`**, which any guest can call, with no guard
   (`rust/tonk-portal/src/bootstrap.js:124`, `bridge.rs:1157`). It was added
   in #746 for a profile switch run inside the sealed Hub. Since #891 the
   worker broadcasts `profile-changed` to top-level pages instead, and
   sandboxed guests have no service worker container to receive it, so
   nothing first-party calls it any more.
4. **A profile change** (`rust/tonk-host/src/navigate.rs:62`). This one is
   justified: every portal and subscription belongs to the old profile.

## The invariants

Each workstream below exists to make one of these true. They are the review
standard for any later change to these paths.

1. **What is rendered changes only to a newer answer.** An empty frame, an
   error, a reconnect or a restart never removes rendered content. Deleting
   something is an explicit signal, not something inferred from absence.
2. **A live guest frame is torn down only when its element is really
   removed.** No remount on deploy, no rebuild on a space switch, no
   remove-and-reinsert anywhere.
3. **Nothing on the read path waits on the network.** A read answers from
   what is local. What is missing is fetched in the background and arrives
   as a later frame.
4. **A worker restart is invisible.** No per-tab state lives only in the
   worker's memory, and boot serves local requests before any remote work.
5. **Background work never holds what foreground work needs.** No lock is
   held across a network round trip.
6. **Updates apply at boundaries.** New code takes effect on the next load
   or the next time a site boots, never under a live view. The only
   exception is a profile change.

## Workstreams

In the order I recommend, by how much of the symptom each removes per unit
of work.

### 1. Keep what is rendered (invariant 1)

In `rust/tonk-display/src/element.rs`:

- Once a display has rendered, hold it through any empty entity frame, not
  only reconnect-flagged ones for 5s. Apply emptiness only when the frame
  is a confirmed deletion (see the question below), or when the display has
  never rendered.
- Give view frames the same hold: a view frame that is empty or lacks the
  facet keeps the mounted slide.
- On `fail()` for a transient class (worker booting, repository not loaded,
  upgrade in progress), keep the rendered children and mark state, instead
  of `clear_host`. Clear only for an error that means the content is really
  unavailable (403 after revocation, a removed entity).
- On a changed view template or descriptor, render the new template and
  keep the old one mounted until the new one has drawn its first frame,
  then swap. That turns a blank interval into a single replacement.

Tests: each trigger above, driven through a real subscription, asserting the
rendered DOM stays put and that no nested `<tonk-site>` disconnects (count
`disconnected_callback` calls).

**Question to settle first:** what is the explicit deletion signal? Today
absence is the only signal there is. Candidates: the worker includes a
`removed` marker when an entity it previously delivered to this stream no
longer matches after a local or pulled commit, as opposed to a cold or
restarted stream that simply has not got there yet. This is the one design
decision in this plan that touches the stream protocol.

### 2. Stop rebooting guests (invariants 2 and 6)

- **Delete `remountGuests` and the alignment reload on `controllerchange`.**
  Subscriptions already re-establish against the new worker on their own
  (`rust/tonk-host/src/host.rs:102-122`). New guest code takes effect the
  next time a site boots. To make that safe:
  - keep the previous generation's guest assets (including the lazily
    fetched editor bundles) until no client of that generation remains;
  - version the guest–worker protocol. Only when it changes incompatibly,
    reload, and only while the tab is hidden.
  - A changed page build is the same case: defer the reload until the tab
    is hidden or the next navigation, rather than reloading in front of
    the person.
- **Remove `tonk.reload()`** from `bootstrap.js` and the bridge's `"reload"`
  handler. Keep `reload_page` for the top page's `profile-changed`.
- **Reuse the guest across a space switch.** The guest content is the same
  `tonk:site` display for every space, and reach is enforced on the parent
  side of the bridge. On a `with`/`allow` change: cancel the old
  subscriptions and relays (`clear_subs`), set the new `with`/`allow` on the
  portal state, send the context update (`refresh_context`), and re-claim.
  Only a missing or dead iframe rebuilds. Needs a test that a subscription
  opened under the old reach is refused after the switch (see
  `.claude/rules/authority-tests.md`).

### 3. Make a worker restart invisible (invariant 4)

- **Keep the keepalive running offline.** The `navigator.onLine` check in
  `spawn_keepalive` stops the worker being kept alive exactly when nothing
  can replace it from the network. A POST to the worker is local.
- **Persist the site stamp.** Either write the overlay through to
  CacheStorage on change (the handoff path already restores it from there
  at boot, `worker.rs:2028-2035`), or have the page re-claim its own path on
  every reconnect before the first frame, rather than waiting to see an
  empty frame.
- **Serve local requests first at boot.** Move `reconcile_root_delegation`,
  the profile library fetch and every other remote step off the path to
  the first `/api` answer, into the detached chores, with timeouts.
- **Count worker boot progress as life in the boot watchdog**, or drop the
  automatic reload and keep only the terminal message.

### 4. Never block foreground work behind the network (invariant 5)

- Do not hold `AppState`'s lock across a network round trip. Sync, sweeps
  and boot chores take what they need under the guard, release it, do the
  network work, and take it again briefly to apply the result.
- Writers that currently do network work under the write guard (join,
  space create and remove, session renewal, blob serve) do the network part
  first, then take the guard only to install the result.
- Stop taking the lock twice per request before routing; `sweep_stale_clients`
  can run on its own schedule.
- In dialog, a commit holds `branch.writer()` across Mint, which can hydrate
  remotely (`dialog-db/rust/dialog-repository/src/repository/branch/commit.rs:260-261`).
  Fixed by workstream 5: once heads move only after a download, a commit
  touches only local nodes.

### 5. Reads answer from what is local (invariant 3)

This is the root of "fast sometimes, slow other times", and it is where the
trade-off sits.

**Recommended: a space's head moves only once what reads need is local.**
Extend #1167's download-before-advance from the profile branch to every
space branch, using the `operational()` download (EAV indexes and blobs;
history and coverage stay lazy, `download.rs:87-101`). The pull runs in the
background, and the UI keeps showing the old head until the new one is
complete. That is literally "outdated over unusable": reads never wait,
because the head never names a node this device lacks.

The cost is bandwidth and storage: a device downloads every change to every
space it follows, not just the parts it reads. For the spaces people have
today that is small. For a very large space it is not, and the partial
replica was designed for that case.

**For data this device has never seen** (a cold open, or a space too large to
download whole), keep on-demand hydration but take it off the blocking path:

- add a request timeout to the S3 and UCAN transports so a stalled
  connection fails instead of hanging;
- give reads a local-only mode in dialog that returns what is local plus a
  signal that the answer is incomplete, and schedule the missing ranges for
  background hydration; the subscription then delivers a later frame when
  they land;
- the display renders the partial answer under invariant 1.

**Assumption to check:** that "functional with partial data" means partial
results are acceptable for every query. For a list or a document, yes. For
an authorization check, or a rule whose answer flips when a fact is missing
(a negation), a partial answer is a wrong answer. The local-only mode needs a
rule for which queries may answer partially and which must wait or fail;
that is why authorization branches already download first.

### 6. Measure it

Without numbers none of this can be shown to work, and the problem reads as
random precisely because nothing counts it. Add counters, reported through
the existing analytics, for:

- guest iframe builds per navigation, and `<tonk-site>` disconnects;
- displays that went from rendered to placeholder, by cause (empty frame,
  error, template change);
- worker boots per hour of tab life, and boot duration split by step;
- query latency (p50, p99), split into lock wait, local read and remote
  hydration;
- reads that hydrated remotely.

Add a scenario to `rust/tonk-perf` that opens a space, restarts the worker,
switches spaces and lands a deploy, asserting zero placeholder regressions
and zero guest rebuilds.

## Already done on this branch

`fix(site): build a site's guest once per navigation, heal the current space`:
the attribute callbacks on `<tonk-site>` coalesce into one pass, so a space
switch boots the guest once instead of two or three times. The self-heal
probe now follows the site's current `with`, is installed once a placeholder
resolves, and is removed on disconnect.

## Questions for the maintainer

1. What is the deletion signal (workstream 1)? It is the one change to the
   stream protocol.
2. Are you willing to pay the bandwidth of downloading every followed space's
   changes (workstream 5), or does a size threshold decide per space?
3. Is a cold open of a space never seen on this device in scope for "instant"?
   It cannot be: it needs the network. The realistic goal there is chrome
   that renders at once and content that fills in without blocking.
