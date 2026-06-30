# SW-driven sync — work-queue in the worker, heartbeat from the page

## Context

After de-Leptos-ing `tonk-ui/src/bin/ui.rs` (it now mounts a bare
`<tonk-host><tonk-repository profile><tonk-branch meta><tonk-site>` root
instead of `leptos::mount::mount_to_body(TonkShell)`), the only caller of
`sync_controller::mount()` — the Leptos `TonkSpaceSealed` component — is
no longer mounted. The launcher's `space/:space → TonkSpaceSealed` route is
dead code. So the in-page 20s sync heartbeat never starts.

Symptom: a space sits `ahead` (local commits not pushed), the sync chip is
stuck on `syncing…`, and it only "settles eventually" because the Background
Sync API registration (`registration.sync.register`) still fires on the
browser's own throttled schedule. The sync *engine* is healthy — a manual
`POST /api/repository/{repo}/branch/main/sync` instantly flips `ahead → synced`.

This plan moves the *what to sync* bookkeeping into the service worker (which
already sees every commit and knows which branches are open) and leaves the
page responsible only for the *when* it cannot supply itself: the pull
heartbeat. Pushes are self-driven SW-side via `event.waitUntil`.

## Design

### Split of responsibility (revised — no poke endpoint, no page heartbeat)

The page needs no sync responsibility at all. Every request the page already
makes (query, subscribe, navigation, transact) flows through the SW's
`on_fetch`; each one schedules a **debounced** sync drain on
`event.waitUntil(...)`. So normal request traffic IS the heartbeat:

- **SW owns the work-queue (the *what*).** It observes every commit (marks the
  repo push-dirty) and holds the cached open repos/branches (`dialog-reactor`
  `repos()` + cached `BranchState`).
- **`on_fetch` schedules the drain for EVERY request, debounced.** The drain
  pushes the dirty set and pulls every open repo — so `waitUntil` performs both
  push AND pull. A burst of boot queries collapses into one drain via the
  debounce; an idle drain (nothing dirty, all up to date) is just a cheap
  fetch+classify per open branch.
- **No `POST /api/sync` poke endpoint, no sync-specific page machinery.** The
  page makes requests; that's the cadence. The Background Sync registration
  stays as the tab-closed backstop (fires the same drain).
- **Idle backstop (not special).** A tab sitting with zero requests (no
  interaction, no live subscription traffic) would never trigger a drain, so
  upstream changes wouldn't pull. So the page still needs *some* low-frequency
  traffic — but NOT a dedicated sync poke: any ordinary periodic request works,
  because `on_fetch` schedules the drain for every request. Reuse an existing
  endpoint (e.g. a periodic `/api/.../sync/status` read the chip already wants,
  or a profile/identity ping) rather than inventing a sync-specific route. The
  drain rides along on whatever request the timer makes.

### The two queues, one drain

A single drain pass:

1. **Push** every `(repo, branch)` in the push-dirty set; clear each on a
   successful push (a `success:false` non-fast-forward leaves it dirty for the
   next pass, with backoff).
2. **Pull** every open branch that has an upstream (the open-with-upstream
   set), so a read-only viewer receives upstream changes without ever
   committing. A pull that advances the head re-polls subscriptions (existing
   reactor behavior), so the view updates.

Prioritize by **recent activity**: most-recently-committed branches first, so
an active editor's pushes land ahead of background pulls of idle spaces.

### Endpoint surface

Collapse the per-branch `POST /api/repository/{repo}/branch/{branch}/sync`
cadence-from-the-UI into a single parameterless **`POST /api/sync`** (the old
per-branch route stays for explicit/manual sync and the status route is
unchanged). `POST /api/sync` drains the queue: prioritized, branches synced in
parallel. The page never names a repo.

The commit-driven push drain does NOT go through an HTTP endpoint — it runs
in-process on `event.waitUntil` directly from the transact handler.

### Pause preference

Unchanged: the durable `tonk:auto-sync` / replica `enabled` fact is the single
gate (`is_sync_enabled`), consulted inside the drain per `(repo, branch)`. A
paused replica is skipped (push and pull) and reports `paused` via the existing
status route. The queue may still hold a paused branch; the drain no-ops it.

## Work items

### 1. `tonk-worker`: the work-queue + drain

- Add a `SyncQueue` to `AppState` (or `TonkState`): a push-dirty set keyed by
  `(repo, branch)` with a last-activity timestamp for priority, behind the
  existing lock. (`Date.now()` is unavailable in the reactor's deterministic
  paths but fine in the SW event context — stamp at enqueue from the fetch/
  message handler, not inside the reactor.)
- **Enqueue on commit.** In `router/transact.rs` (`transact` / `transact_profile`,
  the same place `spawn_dispatch` runs), record `(repo, branch)` into the
  push-dirty set after a successful commit.
- **`drain_sync(state)`** in `router/sync.rs`: push the dirty set (clear on
  success), then pull the open-with-upstream set. Reuse `sync_repository` /
  `branches_to_sync`. Run branches in parallel (`join_all`); never surface
  errors (log only). Respect `is_sync_enabled` per branch.
- **`POST /api/sync`** route → `drain_sync`. Parameterless.

### 2. `tonk-worker`: commit push on `event.waitUntil`

- Add `"ExtendableEvent"` to `tonk-worker`'s `web-sys` features (currently only
  `FetchEvent` + `ExtendableMessageEvent` are enabled; `wait_until` lives on the
  `ExtendableEvent` base).
- **Collector = `dialog_common::r#async::TaskQueue`** (not a bespoke Vec<Promise>).
  A handler `.spawn`s background work into a **global bucket** on `AppState`
  (settled: global is fine — over-attaching one event's `waitUntil` to another's
  work only keeps the SW alive slightly longer, never drops work). `on_fetch`
  upcasts its `FetchEvent` to `ExtendableEvent` and, after `router.call` returns,
  `wait_until(future_to_promise(bucket.join()))` so the SW stays alive until the
  spawned work settles.
- On commit, the transact handler `.spawn`s a **debounced** push drain of the
  just-committed repo into the bucket. Debounce collapses an edit burst into one
  push. This makes pushes survive the tab backgrounding without any page poke.

### 2b. Per-branch incremental status (NOT gated on the whole drain)

The chip must move `syncing… → synced` **per space as each finishes**, never
waiting for every space in the drain to complete.

- Today `sync` (per-branch) flips the chip to `pending` at the start but does
  NOT publish the settled status — the old Leptos controller published it via a
  follow-up `sync_status` call. With the controller gone, nothing settles the
  chip, so it stays `syncing…` even after the push lands. This is the second
  half of the regression.
- Fix: the per-branch sync path publishes its OWN settled status (classify
  local-vs-remote → `publish_sync_status`) right after it reconciles, the same
  computation `sync_status` already does. So every caller (manual `/sync`, the
  drain, the background event) settles the chip per-branch with no separate
  follow-up.
- In the drain, this publish happens as each branch finishes, so a slow space
  never pins another space's chip on `syncing…`.

### 3. `tonk-host`: the pull heartbeat (the *when*)

- In `<tonk-host>`'s `connected_callback`, install:
  - an **interval** (≈20s) that fires `POST /api/sync`;
  - `online` and `visibilitychange→visible` listeners that fire `POST /api/sync`.
- Gate so only the **top-level** host runs the heartbeat (a host with no
  `<tonk-host>` ancestor and not inside a sealed guest) — nested sealed-guest
  hosts relay up, so one heartbeat covers the page. Avoids N redundant timers
  across nested iframes.
- No per-repo bookkeeping in the page. The poke is parameterless.
- Keep the Background Sync registration on commit (tab-close durability); its
  SW `sync` handler calls the same `drain_sync`.

### 4. Cleanup

- Delete `tonk-ui/src/sync_controller.rs` (Leptos `Signal`/`use_interval_fn`/
  `use_event_listener` — replaced by the host heartbeat + SW queue).
- Remove the orphaned sync wiring from `components/space_sealed.rs`
  (`sync_controller::mount`) and, if otherwise dead, the
  `TonkSpaceSealed`/launcher `space/:space` routes.
- Remove `notify_committed` / `COMMITTED_EVENT` plumbing from the editor/inspector
  commit paths (the SW now learns of commits directly from the transact it
  serves).
- Keep `state_chip` / the `<tonk-sync-state>` element and the read-only
  `/sync/status` route as-is — the chip is fine; it just needs the queue to
  actually run so `ahead → synced` happens promptly.

## Open questions / decisions

- **Drain concurrency cap.** Unbounded `join_all` over every open branch could
  burst the network on a many-space profile. Probably cap at a small N and let
  priority order decide who goes first.
- **Pull cadence vs push cadence.** Push is event-driven (commit → waitUntil);
  pull is the 20s heartbeat. Do we also want a faster pull right after a push
  lands (to catch a concurrent remote writer)? Defer — the next heartbeat covers
  it.
- **`is_sync_enabled` cost.** Consulted per branch per drain; it acquires the
  branch and runs a query. For an idle pull pass over many branches this is
  overhead — consider caching the enabled flag on the cached `BranchState`.

## Verification

- Reload `/space/{did}`: chip should reach `synced` within one heartbeat
  (≈20s) of a commit, and a manual `POST /api/sync` should drain immediately.
- Make a local edit in tab A, confirm tab B (same space) pulls it within a
  heartbeat without any commit in B.
- Background a tab mid-edit: the `waitUntil` push should still land (Background
  Sync as the longer-tail backstop).
- `cargo fmt --all` + `cargo clippy --all --all-targets --all-features -D warnings`;
  the wasm-gated `#[dialog_common::test]` tests for the queue/drain.

Relates to [[project_navigate_as_message]], [[project_sync_redesign_storage]],
[[project_sync_pause_design]], [[project_de_leptos_ui_plan]],
[[project_slow_load_main_transact]].
