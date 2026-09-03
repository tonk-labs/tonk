# Sign-out and account profile routing

## Goal

Make browser sign-out non-destructive and make the next ordinary account
ceremony work for either the same account or a different account without ever
rebinding retained local spaces to the wrong account.

The minimum completed behavior is:

1. Signing out disconnects account services but preserves the current browser
   profile, its historical account root, every local space, and every local
   commit.
2. Signing back into the same account reuses that profile and device identity.
3. Signing into an account already represented by another browser profile
   activates that profile before creating the account-to-device grant.
4. Signing into or creating an account not represented locally creates a fresh
   profile and leaves the signed-out profile untouched.
5. Every open tab is prevented from sending profile-scoped work through a
   stale UI after the service worker changes the active profile.

This plan completes the repository's current profile-per-account model. It is
not the larger stable-profile, root-keyed credential-catalogue redesign. Keep
the new account-routing interface narrow enough that a later credential model
can replace its implementation without teaching the ceremony UI about profile
storage.

## Product contract

### Sign-out

`DELETE /api/account` remains a local sign-out operation:

- clear the provider attachment and account-derived replicas;
- invalidate the hidden account-repository routing keys;
- retain the local root record, device/profile signer, certificates, profile
  replica catalogue, space databases, passkeys, and unsynced commits;
- do not rotate profiles;
- do not revoke the account's device grant remotely; that remains the separate
  Devices -> remove-access operation;
- do not delete any browser storage.

The current label, `remove this device`, promises both remote revocation and
local deletion even though the implementation does neither. Replace it with
`sign out on this device`, move it out of the `delete data` section, and say
explicitly that local spaces remain, including spaces that have not been
backed up or synced.

### The next account ceremony

The passkey ceremony discovers the target account root. Profile selection must
therefore happen in the worker after that root is available and before the
worker reads the target device signer, mints a root-to-device delegation, or
persists account attachment facts.

Resolve the target with this matrix:

| Active profile state | Discovered account root | Target |
| --- | --- | --- |
| No historical root | Any root | Keep the active profile. This preserves first-account onboarding and attaches its existing local spaces in place. |
| Historical root equals discovered root | Same account | Keep the active profile and refresh the existing grant. |
| Historical root differs; one or more roster profiles have the discovered root | Existing account on this browser | Activate the first matching profile in the roster's stable name order. Retain any duplicate same-root profiles unchanged and report only a content-free diagnostic. |
| Historical root differs; no readable roster profile matches | New account on this browser | Create and activate a fresh profile. |

An unreadable inactive roster entry is skipped and retained; it is never
deleted or repaired as part of login. If a profile is identified as the match
but cannot boot, fail the ceremony before changing the active pointer. Do not
fall through to a new duplicate after a positive match fails to boot.

At every layer, keep `identity::persist_root`'s different-root conflict as the
last provenance guard. Automatic routing must make that conflict unreachable
in the normal different-account flow, not weaken or remove it.

### Data ownership

Profiles move; spaces do not. Account routing changes only the registry's
active-profile pointer and the in-memory `TonkState`. It must never copy a
replica row, root record, certificate, space database, or pending commit from
one profile to another. Shared underlying space storage remains governed by
each profile's replica catalogue and authority checks.

The currently active local-only reconciliation work in
`router/account_state.rs`, `router/adopt.rs`, `router/repository.rs`, and its
`account_flow.rs` regression is adjacent work, not disposable scaffolding.
Preserve it. The new end-to-end regression creates its unbacked-up space after
sign-out so account reconciliation cannot attach a remote before the profile
transition is exercised.

## Approach

Deepen `router::profiles` into the single profile-lifecycle Module. Its public
worker-internal Interface accepts an account root and returns a read guard over
the correctly selected `TonkState`; callers do not scan the roster, create
profile names, repoint the registry, or coordinate concurrent switches.

The intended Interface is equivalent to:

```rust
pub(crate) enum AccountProfileDisposition {
    Current,
    Existing,
    Created,
}

pub(crate) struct AccountProfileGuard {
    tonk: OwnedRwLockReadGuard<TonkState>,
    disposition: AccountProfileDisposition,
}

pub(crate) async fn for_account(
    state: AppState,
    root: &Did,
    source: Option<&ClientId>,
) -> Result<AccountProfileGuard, TonkWorkerError>;
```

`AccountProfileGuard` is the test surface and concurrency boundary. It pins the
selected state while custody persists the root and provider attachment, so a
second tab cannot switch profiles between selection and the grant write. Do
not expose profile names to `tonk-ui` or make the page choose the target.

All profile switches, including `POST /api/profiles/activate`,
`POST /api/profiles/add`, and automatic account routing, use one transition
mutex and one private promotion path:

1. inspect the current historical root;
2. resolve or create a candidate without changing the active pointer;
3. boot the candidate completely;
4. write its roster entry;
5. write the active pointer;
6. swap the in-memory state, an infallible operation after the pointer write;
7. advance the client-context generation and notify other top-level tabs;
8. return a read guard that pins the selected state for the rest of the
   ceremony.

Failures before step 5 leave the old active pointer and state intact. A failure
later in account linking may leave the successfully selected target profile
active, but it must leave every old profile and local space intact and allow a
retry on that target.

## Constraints and non-goals

- Never clear IndexedDB, OPFS, CacheStorage, service-worker registrations,
  passkeys, credentials, profile roster entries, or space databases as part of
  sign-out or recovery.
- Do not replace a historical root on a profile with attachment history.
- Do not make sign-out perform remote device revocation. The existing Devices
  pane remains the explicit security operation for removing account access.
- Do not rotate when the user clicks sign-out or merely opens the login UI.
- Do not introduce a second persisted account-root index in the device roster.
  The root record on each profile is authoritative; the resolver reads it.
- Do not merge duplicate same-root profiles or move their local-only spaces.
  Profile consolidation and destructive local-profile deletion are follow-ups.
- Do not change permanent account deletion's fresh-profile transition.
- Do not change CLI account state in this pass.
- Keep logs content-free: disposition and profile handle are acceptable local
  diagnostics; account roots, credential IDs, email addresses, and space names
  are not.
- A local-data deletion feature is out of scope until the UI can inventory
  local-only spaces and prove what is backed up.

## File map

- `rust/tonk-worker/src/device.rs`
  - Separate fresh profile creation from active-pointer mutation.
  - Keep roster validation and existing-profile opening in the registry.
- `rust/tonk-worker/src/worker.rs`
  - Preserve the profile-transition lock and client-context generation across
    `TonkState` swaps.
  - Thread the source client ID into custody handoffs.
- `rust/tonk-worker/src/router/profiles.rs`
  - Own root-based profile resolution, serialized promotion, stable account
    guards, and profile-change notification.
- `rust/tonk-worker/src/router/identity.rs`
  - Factor authoritative historical-root loading so it can inspect an explicit
    roster profile without constructing a full `TonkState`.
- `rust/tonk-worker/src/router/custody.rs`
  - Route login and account creation through the selected profile and keep its
    guard for all local account writes.
- `rust/tonk-worker/src/router/session.rs`
  - Bind each service-worker client to the context generation under which it
    registered.
- `rust/tonk-worker/src/router.rs`
  - Apply the stale-client fence to profile-scoped API requests.
- `rust/tonk-worker/src/router/navigate.rs`
  - Broadcast a profile-change reload request to top-level clients other than
    the ceremony or activation source.
- `rust/tonk-host/src/navigate.rs`
  - Recognize the worker's profile-change message and reload the top-level
    document through the existing guest-safe reload path.
- `rust/tonk-workspace/src/ui_account_settings.html`
  - Replace the false remove/delete language with the sign-out contract.
- `rust/tonk-worker/tests/standard_library.rs`
  - Pin the settings markup and exact non-destructive copy.
- `rust/tonk-ui/src/account_flow.rs`
  - Cover same-account re-login, different existing-account routing, retained
    local-only spaces, stable profile cardinality, and sibling-tab reload.

## Task 1: Pin the non-destructive sign-out contract

**Files**

- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-workspace/src/ui_account_settings.html`
- Test: `rust/tonk-worker/src/router/account.rs`
- Test: `rust/tonk-worker/tests/standard_library.rs`

**Behavior**

- Keep `account::unlink` as a provider/account-replica disconnection.
- Add a worker regression,
  `it_signs_out_without_deleting_the_root_or_local_spaces`, which attaches a
  test account, creates a zero-remote local space, records the root and profile
  space keys, calls `unlink`, and asserts:
  - provider status is absent;
  - account replicas and hidden account-key routing are unavailable;
  - the root record is byte-for-byte unchanged;
  - profile name and profile DID are unchanged;
  - the local space remains listed and loadable;
  - its remote map remains empty.
- Change the settings structure to separate sections:
  - `sign out`: `disconnect this account; keep local spaces on this device`
    with action `sign out on this device`;
  - `delete data`: retain only permanent account deletion.
- Use confirmation heading `confirm sign out`, consequence copy
  `this disconnects the account from this browser. local spaces stay on this
  device, including spaces that have not been backed up or synced. you can sign
  into this or another account later.`, and submit label `sign out`.
- Extend the standard-library test to require those phrases and to reject
  `remove this device`, `confirm device removal`, and any claim that sign-out
  removes local data.

**TDD checklist**

- [ ] Add the worker regression first and run
  `nix develop . -c test:web:debug -p tonk-worker -E 'test(it_signs_out_without_deleting_the_root_or_local_spaces)'`;
  expect the current unlink implementation to pass this characterization and
  pin its non-destructive behavior before routing changes begin.
- [ ] Add the exact markup assertions first and run
  `cargo test -p tonk-worker --test standard_library it_serves_settings_as_a_routed_page_of_the_hub`;
  expect failure on the current remove/delete copy.
- [ ] Make only the required unlink and settings changes, rerun both commands,
  and expect success.

## Task 2: Build the account-to-profile routing seam

**Files**

- Modify: `rust/tonk-worker/src/device.rs`
- Modify: `rust/tonk-worker/src/worker.rs`
- Modify: `rust/tonk-worker/src/router/identity.rs`
- Modify: `rust/tonk-worker/src/router/profiles.rs`
- Test: the colocated test modules in those files

**Interfaces and implementation**

- Replace `Registry::rotate`'s combined create-and-repoint behavior with:
  - `create_profile(&Storage) -> (String, Profile)`, which creates only;
  - the existing `set_active`, called only by the promotion path after boot and
    roster preparation succeed.
- Update `profiles::add` to use the same promotion path. Preserve its current
  abandoned-add reuse rule for an active rootless profile with no real spaces.
- Add a transition mutex shared across every `TonkState` replacement. Preserve
  the same `Arc` in `boot_state`/promotion just as the worker already preserves
  the retirement latch.
- Factor `identity::load_record` into a wrapper over an explicit-profile helper
  that reads and validates `tonk-local-root-v1` for `(Profile, Operator)`.
  Expose a smaller `historical_root_did` helper to the profile Module; do not
  expose `LocalRootRecord` outside identity.
- Implement `profiles::for_account` and `AccountProfileGuard` with the decision
  matrix above. Read only roster-named profiles with `Registry::open_profile`;
  never call open-or-create for an unvalidated candidate.
- Search candidates in roster name order. Skip and retain unreadable entries
  with a content-free warning. If several readable entries match, choose the
  first deterministically and retain all others.
- Boot only the chosen candidate. A matching candidate that fails boot aborts
  before pointer mutation. A fresh candidate that fails boot remains an
  unreachable orphan profile but never becomes active or receives account
  facts.
- Make the promotion path write the candidate roster entry before the active
  pointer, then swap state without another fallible step between pointer and
  memory. Roster refresh after the swap remains best-effort.
- Return an owned read guard over the selected active state while still holding
  the transition mutex, then release the transition mutex. A concurrent switch
  can queue but cannot acquire the state write lock until the account ceremony
  drops the guard.

**TDD checklist**

- [ ] Add `device::tests::it_creates_a_profile_without_repointing_the_device`
  and update the existing rotation tests; run
  `nix develop . -c test:web:debug -p tonk-worker -E 'test(it_creates_a_profile_without_repointing_the_device)'`;
  expect the new test to fail against `Registry::rotate`.
- [ ] Add profile routing tests:
  - `it_keeps_a_rootless_local_workspace_for_its_first_account`;
  - `it_reads_an_inactive_profiles_historical_root_without_booting_it`;
  - `it_keeps_the_current_profile_for_the_same_account_root`;
  - `it_reuses_the_roster_profile_with_the_discovered_root`;
  - `it_creates_a_fresh_profile_for_an_unknown_account_root`;
  - `it_never_moves_spaces_when_routing_between_accounts`;
  - `it_leaves_the_active_pointer_unchanged_when_the_matching_profile_cannot_boot`;
  - `it_serializes_add_activate_and_automatic_account_routing`.
- [ ] Run them as one focused slice with
  `nix develop . -c test:web:debug -p tonk-worker -E 'test(it_.*profile.*account|it_never_moves_spaces|it_serializes_add_activate)'`;
  expect the existing different-root path to fail before the resolver exists
  and all cases to pass after the refactor.
- [ ] Rerun the existing `profiles` and `identity` module tests; expect
  `it_rejects_a_different_root_on_a_previously_linked_profile` and
  `it_accepts_the_same_root_again_after_signing_out` to remain green unchanged.

## Task 3: Route custody login and account creation through the seam

**Files**

- Modify: `rust/tonk-worker/src/router/custody.rs`
- Modify: `rust/tonk-worker/src/worker.rs`
- Test: `rust/tonk-worker/src/router/custody.rs`
- Test: `rust/tonk-worker/src/router/profiles.rs`

**Behavior**

- Thread the source `ClientId` from `TonkServiceWorker::on_message` through
  `custody::receive`, `perform`, `login`, `complete_login`, and `create`.
- In `complete_login`, derive the recovered root first, call
  `profiles::for_account`, and only then read the device signer and create the
  account-to-device delegation.
- In `create`, derive the new account root first, call the same resolver, and
  only then build the account request. This prevents a new signup from being
  bound to a signed-out historical profile.
- Refactor the local link helper to accept `&TonkState` from
  `AccountProfileGuard` instead of reacquiring `AppState`. Keep that guard
  across `persist_root`, `persist_link`, `finish_link`, and enrollment's local
  writes so no intervening profile activation can redirect them. Refactor
  `enroll` and any helper it calls to accept the guarded `&TonkState` as well;
  none of those helpers may reacquire `AppState` while the guard is held.
- Keep network error semantics explicit:
  - passkey/account-open failure happens before profile routing;
  - candidate boot failure leaves the old active profile unchanged;
  - a provider/enrollment failure after successful promotion leaves the target
    active and retryable, with prior profiles untouched.
- Do not retry a different-root `persist_root` conflict by overwriting the root.
  Treat it as an invariant failure in routing and surface the normal safe login
  error with the diagnostic retained in logs.

**TDD checklist**

- [ ] Add a custody regression that signs out A, returns B's account root from
  the passkey, and asserts the minted delegation's audience is B's existing
  profile DID, not A's; run it with
  `nix develop . -c test:web:debug -p tonk-worker -E 'test(it_mints_login_for_the_profile_that_owns_the_recovered_root)'`;
  expect the current implementation to fail at `persist_root` with `Conflict`.
- [ ] Add
  `it_creates_an_account_on_a_fresh_profile_after_sign_out` and assert the
  account-service enrollment request names the fresh profile DID while A's
  historical root and spaces remain unchanged.
- [ ] Add a concurrency regression that pauses after routing, attempts
  `/api/profiles/activate` from another task, and proves activation cannot swap
  state until the `AccountProfileGuard` is dropped.
- [ ] Run the focused custody and profile tests, then
  `nix develop . -c test:web:debug -p tonk-worker`; expect all worker Wasm tests
  to pass.

## Task 4: Fence and reload stale tabs at a profile transition

**Files**

- Modify: `rust/tonk-worker/src/worker.rs`
- Modify: `rust/tonk-worker/src/router/session.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/router/profiles.rs`
- Modify: `rust/tonk-worker/src/router/navigate.rs`
- Modify: `rust/tonk-host/src/navigate.rs`
- Test: the colocated router, session, navigation, and host test modules

**Interfaces and behavior**

- Add an in-memory `context_generation` shared across `TonkState` swaps.
  Store the generation on `ClientState` when a browser client first reaches a
  profile-scoped API route. A newly loaded document gets a new browser
  `ClientId` and binds to the current generation; an existing stale client may
  not refresh its own binding by reposting `/api/site`.
- Add router middleware that compares the requesting `ClientId` binding with
  the current generation before dispatching profile/account/repository routes.
  Requests without a browser client ID remain allowed for native tests and
  internal callers. Health and bootstrap routes remain outside the fence.
- On mismatch, return `409 Conflict` with a stable `profile changed; reload
  required` diagnostic and perform no handler work. The fence applies to reads
  as well as writes: a stale A document must neither mutate nor render B data.
- When promotion publishes a different profile, increment the generation
  before releasing the state write lock. Existing clients then become stale
  atomically with the swap.
- Add `navigate::notify_profile_changed(except: Option<&ClientId>)`, posting
  `{ type: "profile-changed" }` to every top-level `WindowClient` except the
  initiating client. Carry no profile, root, account, or space identifier.
- Extend `tonk_host::navigate`'s installed service-worker listener to recognize
  that message and call its existing guest-safe `reload_page()`.
- Keep the initiating page's current explicit reload/navigation after the
  activation or custody reply. Excluding it from the broadcast avoids aborting
  the response or MessagePort before success is delivered.

**TDD checklist**

- [ ] Add router tests proving a client bound before a profile swap receives
  `409` for a profile read and transact after the generation advances, while a
  new client binds and succeeds.
- [ ] Add `tonk-host` tests
  `it_reloads_for_a_profile_changed_worker_message` and
  `it_ignores_unrelated_worker_messages`; run
  `nix develop . -c test:web:debug -p tonk-host -E 'test(it_.*profile_changed.*worker_message)'`.
- [ ] Add a worker navigation test that enumerates two top-level clients and
  proves the initiator is excluded and the sibling receives an identifier-free
  message.
- [ ] Run
  `nix develop . -c test:web:debug -p tonk-worker -p tonk-host`; expect all
  profile, router, and host Wasm tests to pass.

## Task 5: Prove the complete browser lifecycle

**Files**

- Modify: `rust/tonk-ui/src/account_flow.rs`

**Primary regression**

Add
`it_signs_into_another_account_without_rebinding_retained_local_spaces`:

1. Start with `driver_with_prf_authenticator`, sign up A, and record A's active
   profile name and first virtual-authenticator credential ID.
2. Use the existing Add Account path to sign up B; record B's profile and the
   newly added credential ID.
3. Switch back to A and sign out through Settings.
4. While A is provider-free, create `Retained Draft` with
   `create_space_awaiting_remote(..., false)`. Assert the space is listed and
   its remote map is empty.
5. Remove only A's credential from the test virtual authenticator with
   `WebAuthn.removeCredential`, modeling the user choosing B in a discoverable
   credential picker without changing any Tonk storage.
6. Use the ordinary provider-free `link an account` flow and
   `run_cluster_login` for B. Do not call `/api/profiles/add`.
7. Assert B's existing profile becomes active, the profile count does not grow,
   account summary reports B, and B's profile space list omits A's retained
   space.
8. Switch to A's local-workspace row. Assert `Retained Draft` returns, remains
   loadable, and still has no remote. Dispatch one small profile or space
   transaction and read it back to prove the retained local workspace remains
   writable while signed out.

Keep and strengthen
`it_signs_back_into_the_same_account_after_signing_out`: record the active
profile name, device count, a local space key, and the root before sign-out;
after login assert all four are unchanged. This is the same-account half of the
matrix and must not create a fresh profile.

**Multi-tab regression**

Extend the primary regression or add
`it_reloads_sibling_tabs_before_they_can_use_a_new_active_profile`:

- open a second top-level tab while A is active and record its
  `performance.timeOrigin`;
- complete the A-to-B transition in the first tab;
- assert the second tab reloads, receives a new time origin/client binding, and
  renders B only after reload;
- separately use the router-level stale client fixture from Task 4 to prove an
  old A client ID cannot transact against B even if notification delivery is
  delayed.

**TDD checklist**

- [ ] Add the different-account regression first and run
  `nix develop . -c cargo test -p tonk-ui --features integration-tests it_signs_into_another_account_without_rebinding_retained_local_spaces -- --test-threads=1 --nocapture`;
  expect the current code to fail with the different-root conflict while still
  preserving A's space.
- [ ] Implement Tasks 2-4 and rerun the regression; expect B's existing profile
  to be selected with no third profile and A's local-only space to return after
  switching back.
- [ ] Rerun
  `it_signs_back_into_the_same_account_after_signing_out` and
  `it_adds_a_second_account_and_switches_between_disjoint_space_lists`; expect
  both existing lifecycle paths to remain green.
- [ ] Run the sibling-tab regression; expect the stale tab to reload and the
  stale-client request fixture to be refused without a commit.

## Integration checkpoint

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test -p tonk-worker --test standard_library`.
- [ ] Run `cargo test -p tonk-ui --lib`.
- [ ] Run
  `nix develop . -c test:web:debug -p tonk-worker -p tonk-host -p tonk-workspace`.
- [ ] Run the serialized account browser suite with
  `nix develop . -c cargo test -p tonk-ui --features integration-tests -- --test-threads=1 --nocapture`.
- [ ] Run `git diff --check`.

Fresh evidence after the last production change must show:

- sign-out leaves the root, profile identity, local space list, and zero-remote
  local space intact;
- same-account re-login retains the profile and device row;
- different-account login selects an existing matching profile or creates one
  fresh profile, never rebinds the old root, and never copies the old space;
- every stale top-level client either reloads or receives a no-side-effect
  conflict before reading or writing through the new profile;
- permanent deletion, Add Account, explicit profile activation, account
  reconciliation, and the existing identity conflict guard remain green.

If Chrome/ChromeDriver, Nix daemon access, or another environment boundary
prevents browser/Wasm execution, record the exact command and failure and leave
that boundary explicitly unverified; do not replace it with native-only proof.

## Deferred follow-ups

- A root-keyed credential catalogue on one stable local profile.
- Consolidating duplicate profiles for one account.
- An explicit `delete local data` flow guarded by a local-only/backed-up
  inventory and a typed confirmation.
- Remote device revocation as an optional sign-out mode.
- Namespacing or reference-counting the shared underlying space stores before
  any profile-storage deletion is introduced.
