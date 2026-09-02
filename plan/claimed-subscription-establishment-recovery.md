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

**Plan basis:** `ccffaac2755e` on `staging`, inspected 2026-09-02. The affected
restored space is `did:key:z6MkgK1sLdYGnRg26r42jtnkZ6X3XnZDsjfiikoTEfeMWNHE`.
At this revision its CLI branch is synced and `tonk render --space
tonk-team-restored workspace/sheet` returns the existing artifacts; a browser
one-shot view query returns the `tonk:sheet` `ui` template, while live
subscription startup logs `tonk-subscribe: host did not write
detail.subscription` and the root remains on the empty launchpad. This makes
subscription establishment—not R2 recovery or stored view migration—the
remaining boundary.

**Spike result (2026-09-02):** Direction confirmed for the shared subscription
contract and `tonk-display`. A browser Wasm event test failed before the change
after one claimed-without-handle dispatch, then passed after the shared helper
classified that error as a bounded establishment retry. A `tonk-display` test
now omits the first `view` handle while exercising the real model → dictionary
view → entity chain; it observes two view attempts, mounts the model-specific
template, accepts an entity frame, and reaches `data-state="ready"`. The spike
does not yet move `ui-sync-status` onto the retrying helper, exercise the sealed
nested-frame browser journey, or verify a deployed build against the restored
staging space. Those remain Tasks 3 and 4 below.

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
  `fn is_retryable_subscribe_establishment_error(error: &ErrorDetail) -> bool`.
  It returns true only when the message reports either `no host claimed the
  event` or `host did not write detail.subscription`.
- Keep `subscribe`, `subscribe_with_route`, `subscribe_claimed`, and
  `subscribe_claimed_with_route` public signatures unchanged.
- Replace the local `unclaimed` accumulator with `last_error`; after all 12
  attempts, return the last real `ErrorDetail` so diagnostics preserve whether
  the final failure was unclaimed or claimed-without-handle.

- [ ] Add a browser Wasm test that mounts a connected consumer under a fake
  `tonk-subscribe` listener. On dispatch one the listener calls
  `prevent_default()` but omits `detail.subscription`; on dispatch two it
  installs `{ cancel }`. Assert `subscribe_claimed` succeeds, exactly two
  dispatches occurred, and dropping the returned `Subscription` invokes the
  second attempt's cancel function once.
- [ ] Add pure classifier assertions for both retryable message forms and for
  an unrelated network/descriptor message that must remain non-retryable, so a
  wording change cannot silently broaden or remove either boot race from the
  policy.
- [ ] Run
  `nix develop path:. -c test:web:debug -E 'package(tonk-host)'`; expect the
  claimed-without-handle test to fail before implementation because only one
  dispatch occurs and the helper returns the generic missing-handle error.
- [ ] Implement the classifier and use it in
  `subscribe_claimed_with_route`. Preserve the current delays—250 ms, 500 ms,
  750 ms, then 1 s capped—and the 12-attempt bound; do not add an unbounded
  loop or retry after a handle has been returned.
- [ ] Rerun the focused `tonk-host` Wasm command. Expect both retryable races
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

- [ ] Add
  `it_recovers_a_dictionary_view_after_a_claimed_subscription_omits_its_handle`.
  Configure the fake host to omit the first `view` handle, auto-deliver the
  model frame, then deliver a `show_rows(..., &[("ui", template)])` frame and
  an entity frame after the retry registers them.
- [ ] Assert two `view` attempts occurred, the template marker renders with
  the entity value, and `data-state` is `ready` rather than `loading`,
  `default-view`, or `no-entity`. This is the focused reproduction of the
  restored space: the one-shot-compatible data exists, but the first live view
  subscription handshake is incomplete.
- [ ] Run
  `nix develop path:. -c test:web:debug -E 'package(tonk-display)'`; expect the
  new test to time out in `loading` before Task 1 because the missing-handle
  error ends the resolve chain.
- [ ] Complete Task 1, rerun the same command, and expect the regression plus
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

- [ ] Expand the Wasm test module with a mounted `ui-sync-status` fixture and a
  fake listener that claims the first subscription without a handle, installs
  the second, and sends a `sync:local` reset frame. Assert two attempts occur
  and the element paints `sync--local` instead of remaining
  `sync--syncing`/`sync:pending`.
- [ ] Add a route-change test: leave attempt one retrying, change `with` from
  `main@did:key:zOld` to `main@did:key:zNew`, then allow both attempts to
  settle. Assert only the new route owns a live handle and the old returned
  handle is canceled once.
- [ ] Add a disconnect test that removes the element during the retry delay and
  asserts no late subscription is retained and any late successful handle is
  canceled.
- [ ] Run
  `nix develop path:. -c test:web:debug -E 'package(tonk-workspace)'`; expect
  the first test to remain pending before implementation and the route-change
  test to expose competing async attempts once `subscribe_claimed` is first
  introduced without the generation guard.
- [ ] Implement the generation-safe restart and switch to
  `subscribe_claimed`. Log only the final exhausted error; intermediate
  establishment races stay inside the shared helper and must not paint the
  disc offline.
- [ ] Rerun the focused workspace Wasm command. Expect retry recovery, route
  replacement, disconnect cancellation, and the existing modifier-class tests
  to pass.

### Task 4: Pin the sealed nested-frame space journey in a real browser

**Files:**

- Modify test support: `rust/tonk-ui/src/account_flow.rs:CDP probes, browser log
  helpers`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Add test-only `install_claimed_subscription_race_probe(&WebDriver)`. Through
  `Page.addScriptToEvaluateOnNewDocument`, install an early capture listener in
  every new document that claims and stops exactly the first
  `tonk-subscribe` for each of the `view` and `ui-sync-status` tags while
  deliberately omitting `detail.subscription`. Record per-tag attempt counts
  on `globalThis` for diagnostics.
- Add a reusable browser-log reader beside `dump_browser_log`; it uses the
  existing `/session/{id}/se/log` endpoint and returns entries so the test can
  reject the final `host did not write detail.subscription` message rather
  than only printing it on failure.

- [ ] Add `it_recovers_claimed_subscription_boot_races_in_a_space`. Start with
  `driver_with_prf`, install the race probe before the first navigation, and
  create a disposable local space with `create_space`.
- [ ] Use the existing `post_yaml` helper against
  `/api/repository/{key}/branch/main/evaluate` to install this minimal root
  fixture: an `e2e/space` concept with cardinality-one text `title`, a
  `view!` whose `this: e2e/space` has `show: {ui: ...}` containing a stable
  `[data-e2e-restored]` marker, a `name!` that points `id:tonk/space` at
  `e2e/space`, and an `e2e/space!` instance on the created replica whose title
  is `Recovered content v1`.
- [ ] Navigate afresh to `/space/{key}` so all top, shell-guest, and content-
  guest documents boot under the deterministic race. Use
  `enter_space_view`—not `iframe.contentDocument` or `/json` target counting—
  and assert the marker renders `Recovered content v1` with the enclosing
  `tonk-display[data-state="ready"]`. In the first guest, assert the FABB
  reaches `data-sync-status="sync:local"` rather than retaining the pulsing
  pending disc.
- [ ] Run
  `nix develop path:. -c cargo test -p tonk-ui --features integration-tests it_recovers_claimed_subscription_boot_races_in_a_space -- --test-threads=1 --nocapture`;
  expect the current build to leave the content display loading/defaulted and
  the sync status pending after the probe consumes their first handles.
- [ ] After Tasks 1–3, re-run the focused browser command. Without reloading,
  post a second `e2e/space!` assertion changing the title to `Recovered content
  v2`; re-enter the content frame and wait for the rendered text to update.
  Assert the browser log contains neither the final missing-subscription-handle
  error nor an unhandled `tonk-subscribe` promise rejection.
- [ ] Run the integration checkpoint:
  `nix develop path:. -c test:web:debug -E 'package(tonk-host) or package(tonk-display) or package(tonk-workspace)'`,
  then
  `nix develop path:. -c cargo fmt --all -- --check`.
  Run `nix develop path:. -c test:web:debug` once after the focused packages
  pass; report an interrupted or filtered-away run as incomplete rather than a
  full-suite pass.
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
