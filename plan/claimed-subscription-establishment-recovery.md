# Claimed subscription establishment recovery implementation plan

**Goal:** Make a space recover when a boot-time `tonk-subscribe` event is
claimed but does not synchronously receive `detail.subscription`, so an
existing dictionary view and its entity rows render without a reload and the
sync disc leaves its pending state.

**Approach:** Treat “claimed without a subscription handle” as the same
bounded establishment race as “no host claimed” in the shared consumer helper,
then move the remaining one-shot sync-status consumer onto that helper with
generation-safe cancellation. Pin the behavior at three levels: the event
contract, a real `tonk-display` resolve chain, and a disposable real-browser
space whose root model uses a `show: {ui: ...}` dictionary view. Keep the
branch session in place; this plan does not restore the removed session-swap,
subscription-rebind, or retained-plan machinery.

**Plan basis:** `609d42d58` on `staging`, inspected 2026-09-02. The affected
restored space is `did:key:z6MkgK1sLdYGnRg26r42jtnkZ6X3XnZDsjfiikoTEfeMWNHE`.
At this revision its CLI branch is synced and `tonk render --space
tonk-team-restored workspace/sheet` returns the existing artifacts; a browser
one-shot view query returns the `tonk:sheet` `ui` template, while live
subscription startup logs `tonk-subscribe: host did not write
detail.subscription` and the root remains on the empty launchpad. This makes
subscription establishment—not R2 recovery or stored view migration—the
remaining boundary.

**Implementation result (2026-09-03):** The shared subscription contract,
`tonk-display`, and `ui-sync-status` changes are complete. A browser Wasm event
test failed before the change after one claimed-without-handle dispatch, then
passed after the shared helper classified that error as a bounded establishment
retry. A `tonk-display` test omits the first `view` handle while exercising the
real model → dictionary view → entity chain; it observes two view attempts,
mounts the model-specific template, accepts an entity frame, and reaches
`data-state="ready"`. The real-browser regression creates a disposable space,
renders its dictionary view through both sealed frames, consumes the first
`view` handle on the actual root display, observes the retry, and receives a
live facet update without reloading. The original restored staging space still
requires a post-deploy check; no test mutates it.

**Constraints:**

- Preserve the recovered space and normal browser storage. Automated browser
  coverage must use a disposable profile and disposable space; no test or
  recovery step may clear IndexedDB, unregister workers, revoke credentials,
  rewrite the recovered branch, or fetch/import the R2 backup again.
- Keep the dictionary view model shipped by #808: a view is stored on the
  model as `show: {[symbol]: text}`, and `view=` names a facet such as `ui`.
  Do not reintroduce view entities, `model`/`display` view fields, `--anchor`,
  or old `tonk:view/*` facet concepts.
- Keep `Branch::refresh` in place on the cached branch handle. Do not restore
  `Subscription::rebind`, cached-session replacement, overlay adoption, or
  any other pre-#820 refresh design; those would discard the site stamp that
  the repaired flow is required to retain.
- Retries are bounded and limited to subscription *establishment*. A malformed
  descriptor, an HTTP authorization failure, or an error delivered by an
  already-open SSE must retain its current immediate/transport behavior.
- A stale async `ui-sync-status` attempt must never install a handle after the
  element disconnects or its `with` route changes. Dropping such a late handle
  must cancel it upstream.
- Keep the detached `<tonk-fab>` document-fragment activation error as a
  separately scoped follow-up. PostHog `ERR_FAILED` noise is also unrelated to
  whether the local Tonk subscription is established.
- Keep evidence layers separate: focused Wasm tests prove event/element
  behavior, the native WebDriver test proves the sealed nested-frame journey,
  and the existing restored space is checked only after the fixed build is
  deployed to staging.

## File map

- `rust/tonk-host/src/consumer.rs`: classify retryable subscription-start
  failures and retain the last concrete error after bounded retries.
- `rust/tonk-display/src/element.rs`: extend `FakeHost` so a claimed event can
  omit its handle once, and prove the model/view/entity resolve chain recovers
  to the model-specific dictionary view.
- `rust/tonk-workspace/src/ui_sync_status.rs`: use the shared asynchronous
  establishment helper and guard late attempts across route changes and
  disconnects.
- `rust/tonk-ui/src/account_flow.rs`: deterministically inject the claimed
  without-handle race into sealed frames and verify a disposable space renders
  and updates without reload.

## Requirement coverage

| Required outcome | Owning task | Fresh evidence |
| --- | --- | --- |
| A claimed event that omits its handle does not wedge a consumer | Task 1 | `tonk-host` Wasm event-contract test |
| The recovered dictionary-view shape reaches `ready` | Task 2 | `tonk-display` Wasm resolve-chain test |
| The FABB sync disc does not remain pending | Task 3 | `tonk-workspace` Wasm lifecycle tests |
| The behavior holds across the actual sealed frame hierarchy | Task 4 | serialized `tonk-ui` WebDriver test |
| The original restored space renders and its branch is unchanged | Task 4 | post-deploy staging check plus independent CLI status/render |

### Task 1: Make claimed-without-handle a bounded establishment retry

**Files:**

- Modify: `rust/tonk-host/src/consumer.rs:subscribe_claimed_with_route,
  subscribe_with_route`
- Test: `rust/tonk-host/src/consumer.rs`

**Interfaces:**

- Add a private
  `fn is_establishment_error(error: &ErrorDetail) -> bool`.
  It returns true only when the message reports either `no host claimed the
  event` or `host did not write detail.subscription`.
- Keep `subscribe`, `subscribe_with_route`, `subscribe_claimed`, and
  `subscribe_claimed_with_route` public signatures unchanged.
- Replace the local `unclaimed` accumulator with `last_error`; after all 12
  attempts, return the last real `ErrorDetail` so diagnostics preserve whether
  the final failure was unclaimed or claimed-without-handle.

- [x] Add a browser Wasm test that mounts a connected consumer under a fake
  `tonk-subscribe` listener. On dispatch one the listener calls
  `prevent_default()` but omits `detail.subscription`; on dispatch two it
  installs `{ cancel }`. Assert `subscribe_claimed` succeeds, exactly two
  dispatches occurred, and dropping the returned `Subscription` invokes the
  second attempt's cancel function once.
- [x] Add pure classifier assertions for both retryable message forms and for
  an unrelated network/descriptor message that must remain non-retryable, so a
  wording change cannot silently broaden or remove either boot race from the
  policy.
- [x] Run
  `nix develop . -c test:web:debug -E 'package(tonk-host)'`; expect the
  claimed-without-handle test to fail before implementation because only one
  dispatch occurs and the helper returns the generic missing-handle error.
- [x] Implement the classifier and use it in
  `subscribe_claimed_with_route`. Preserve the current delays—250 ms, 500 ms,
  750 ms, then 1 s capped—and the 12-attempt bound; do not add an unbounded
  loop or retry after a handle has been returned.
- [x] Rerun the focused `tonk-host` Wasm command. Expect both retryable races
  to be classified correctly, the claimed-without-handle fixture to establish
  on its second dispatch, unrelated errors to remain outside the retry policy,
  and cancel to target only the installed handle.

### Task 2: Prove `tonk-display` recovers its dictionary view

**Files:**

- Modify test support: `rust/tonk-display/src/element.rs:tests::FakeHost,
  FakeState`
- Test: `rust/tonk-display/src/element.rs`

**Interfaces:**

- Add a test-only `claimed_without_handle: BTreeMap<String, usize>` to
  `FakeState` and an installation helper that accepts the number of omitted
  handles by subscription tag. The listener still claims those attempts, but
  it must not register the consumer, push a frame, or create a cancel handle
  until the configured omissions are exhausted.
- Production `tonk-display` APIs and lifecycle states remain unchanged; this
  task exercises its existing use of `host_consumer::subscribe_claimed`.

- [x] Add
  `it_recovers_a_dictionary_view_after_a_claimed_subscription_omits_its_handle`.
  Configure the fake host to omit the first `view` handle, auto-deliver the
  model frame, then deliver a `show_rows(..., &[("ui", template)])` frame and
  an entity frame after the retry registers them.
- [x] Assert two `view` attempts occurred, the template marker renders with
  the entity value, and `data-state` is `ready` rather than `loading`,
  `default-view`, or `no-entity`. This is the focused reproduction of the
  restored space: the one-shot-compatible data exists, but the first live view
  subscription handshake is incomplete.
- [x] Run
  `nix develop . -c test:web:debug -E 'package(tonk-display)'`; expect the
  new test to time out in `loading` before Task 1 because the missing-handle
  error ends the resolve chain.
- [x] Complete Task 1, rerun the same command, and expect the regression plus
  the existing default-view, no-entity, repeat, and static-sibling tests to
  pass without a production change in `tonk-display`.

### Task 3: Put `ui-sync-status` on the same recoverable contract

**Files:**

- Modify: `rust/tonk-workspace/src/ui_sync_status.rs:UiSyncStatus,
  connected_callback, attribute_changed_callback, disconnected_callback,
  subscribe_status`
- Test: `rust/tonk-workspace/src/ui_sync_status.rs`

**Interfaces:**

- Add `generation: Rc<Cell<u64>>` to `UiSyncStatus` and a private increment
  helper. Each connect, changed `with`, and disconnect invalidates prior async
  attempts; `subscribe_status` captures the generation it belongs to.
- Replace synchronous `consumer::subscribe(...)` with
  `consumer::subscribe_claimed(...).await` inside the existing microtask.
- Before storing a returned handle, require that the element is still
  connected, the captured generation is current, and no newer subscription is
  installed. Otherwise drop the handle immediately so its cancel function
  runs.

- [x] Expand the Wasm test module with a mounted `ui-sync-status` fixture and a
  fake listener that claims the first subscription without a handle, installs
  the second, and sends a `sync:local` reset frame. Assert two attempts occur
  and the element paints `sync--local` instead of remaining
  `sync--syncing`/`sync:pending`.
- [x] Add a route-change test: leave attempt one retrying, change `with` from
  `main@did:key:zOld` to `main@did:key:zNew`, then allow both attempts to
  settle. Assert only the new route owns a live handle and the old returned
  handle is canceled once.
- [x] Add a disconnect test that removes the element during the retry delay and
  asserts no late subscription is retained and any late successful handle is
  canceled.
- [x] Run
  `nix develop . -c test:web:debug -E 'package(tonk-workspace)'`; expect
  the first test to remain pending before implementation and the route-change
  test to expose competing async attempts once `subscribe_claimed` is first
  introduced without the generation guard.
- [x] Implement the generation-safe restart and switch to
  `subscribe_claimed`. Log only the final exhausted error; intermediate
  establishment races stay inside the shared helper and must not paint the
  disc offline.
- [x] Rerun the focused workspace Wasm command. Expect retry recovery, route
  replacement, disconnect cancellation, and the existing modifier-class tests
  to pass.

### Task 4: Pin the sealed nested-frame space journey in a real browser

**Files:**

- Modify test support: `rust/tonk-ui/src/account_flow.rs:CDP probes, browser log
  helpers`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Add test-only `arm_claimed_view_race(&WebDriver)`. It finds the root
  `tonk-display` that owns the seeded marker, installs a one-shot capture
  listener on that exact composed-event target, claims the first `view`
  subscription without a handle, and selects the `race` facet. Record attempt
  counts and terminal harness errors on `globalThis` for bounded diagnostics.
- Keep the shell sync assertion observational: the FABB must leave pending,
  but the deterministic injected race belongs to the content display. A
  document-global CDP listener is not reliable across the sealed srcdoc
  document rewrite and can consume a subscription on the wrong display.

- [x] Add `it_recovers_a_claimed_subscription_race_in_a_space`. Start with
  `driver_with_prf` and create a disposable local space with `create_space`.
- [x] Use the existing `post_yaml` helper against
  `/api/repository/{key}/branch/main/evaluate` to install this minimal root
  fixture: an `e2e/space` concept with the repository `subject`, a
  `view!` whose `this: e2e/space` has `show: {ui: ...}` containing a stable
  `[data-e2e-restored]` marker, a `name!` that points `id:tonk/space` at
  `e2e/space`, with separate `ui` and `race` templates carrying stable test
  markers.
- [x] Navigate afresh to `/space/{key}` so all top, shell-guest, and content-
  guest documents perform a fresh boot. Use
  `enter_space_view`—not `iframe.contentDocument` or `/json` target counting—
  and assert the marker renders `Recovered content v1` with the enclosing
  `tonk-display[data-state="ready"]`. In the first guest, assert the FABB
  reaches a defined non-pending state; a disposable test environment may
  legitimately be local or offline.
- [x] Arm the one-shot race on the actual root display, select its `race`
  facet, and assert at least two `view` attempts before the v1 marker renders.
  Without reloading, re-assert that facet as v2 and require the same recovered
  live subscription to render the update. Reject final missing-handle and
  dropped-closure errors captured by the harness.
- [x] Run the focused real-browser command:
  `nix develop . -c test:e2e --no-capture -E 'test(it_recovers_a_claimed_subscription_race_in_a_space)'`.
  The production UI artifact is built before running the serialized test.
- [x] Run the integration checkpoint:
  `nix develop . -c test:web:debug -E 'package(tonk-host) | package(tonk-display) | package(tonk-workspace)'`,
  then `cargo fmt --all -- --check` and `git diff --check`. The affected
  package matrix is the scoped checkpoint for this change; the full Web suite
  remains CI evidence.
- [ ] After a build containing the fix reaches staging, open
  `https://staging.tonk.xyz/space/did:key:z6MkgK1sLdYGnRg26r42jtnkZ6X3XnZDsjfiikoTEfeMWNHE`
  in the already-linked account. Verify the existing artifact tabs/content
  render without rejoining or re-importing, the sync disc settles, and a fresh
  console has no missing-subscription-handle error. Independently rerun
  `tonk status --space tonk-team-restored` and
  `tonk render --space tonk-team-restored workspace/sheet`; the branch must
  remain synced and continue returning the recovered artifacts after the
  browser check.

## Completion criteria

- A claimed-without-handle event is retried only during bounded establishment,
  and the installed handle still owns exact-once cancellation.
- `tonk-display` resolves the dictionary `ui` facet and reaches `ready` after
  the first view subscription handshake is consumed.
- `ui-sync-status` recovers from the same race without retaining a stale route
  or post-disconnect handle.
- The real-browser test renders and live-updates a disposable root view through
  both sealed frames with no reload.
- The deployed staging build renders the already-recovered `tonk-team-restored`
  data without another R2 mutation, and CLI status/render checks remain green.
