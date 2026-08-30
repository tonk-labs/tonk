# Recoverable CLI device handoff implementation plan

**Goal:** Make interrupted or stale browser-to-CLI login and offline logout converge without leaving an active provider row that blocks the same device from signing in again.
**Approach:** Make provider registration replace/reuse the exact same-account device generation atomically instead of failing on a recoverable active row, and add an authenticated attachment-recovery seam for legacy callback payloads. Persist signed detach intents in the native account-session state before local logout, retry them independently of the active account, and keep visible messages explicit about local safety versus provider cleanup.
**Constraints:**
- Never weaken the rule that one device DID cannot be active under two different account roots.
- Never detach or replace a row using only caller-supplied DID/attachment text; the provider must verify account authority or the device-signed generation-bound detach intent.
- Callback payloads remain backward-compatible with pages that omit `attachmentId` or `serviceUrl`; every unversioned attachment shape is provider-verified, and malformed or mismatched grants still commit no local authority.
- Logout remains local-first and preserves profile identity, spaces, and edits while offline.
- Provider cleanup is idempotent: `detached`, `alreadyDetached`, `superseded`, and `revoked` all retire the outbox item.

## File map
- `rust/tonk-account-service/src/core/devices.rs`: same-account retry/rotation and authenticated attachment recovery.
- `rust/tonk-account-service/src/handlers/devices.rs`: recovery endpoint and versioned registration response.
- `rust/tonk-account-service/src/store.rs`: atomic replace-or-recover store interface.
- `rust/tonk-account-service/src/store/sqlite.rs`: transaction-backed local adapter and fault tests.
- `rust/tonk-account-service/src/store/d1.rs`: D1 batch/transaction adapter.
- `rust/tonk-account-service/src/auth.rs`: exact active-grant authorization coverage for recovery.
- `rust/tonk-account-service/src/helpers/server.rs`: native helper parity for new provider routes and response fields.
- `rust/tonk-account-service/tests/service.rs`: full-ceremony HTTP coverage for canonical registration and legacy recovery.
- `rust/tonk-worker/src/router/account_devices.rs`: provider-authorized callback registration and canonical-generation validation.
- `rust/tonk-ui/src/account.rs`: versioned CLI callback payload and use the provider's canonical active generation.
- `rust/tonk-ui/src/account_flow.rs`: browser integration contract for callback v2 fields.
- `rust/tonk-cli/src/account.rs`: callback compatibility, authenticated missing-attachment recovery, logout messaging, and outbox flushing.
- `rust/tonk-cli/src/account_session.rs`: state version 2 with durable `PendingDetach` records and cross-process handoff/settlement exclusion.
- `rust/tonk-cli/src/bin/tonk.rs`: flush retryable cleanup at account command boundaries without blocking reads/login indefinitely.
- `rust/tonk-cli/tests/account_interrupt.rs`: process interruption and callback retry behavior.
- `rust/tonk-cli/README.md`: replace promises about nonexistent recovery with the implemented contract.
- `docs/storybook/accounts/browser-cli-handoff.md`: lifecycle/retry state machine.
- `docs/storybook/verification/accounts.md`: `HANDOFF` fault cases for stale worker, lost callback, and offline logout.
- `docs/storybook/bug-triage.md`: retire fixed handoff and same-browser generation findings without losing their history.
- `docs/storybook/app/data.{json,js}`: generated Storybook source data.

### Task 1: Make same-account CLI registration converge atomically

**Files:**
- Modify: `rust/tonk-account-service/src/core/devices.rs:register_device`
- Modify: `rust/tonk-account-service/src/store.rs:Store`
- Modify: `rust/tonk-account-service/src/store/sqlite.rs`
- Modify: `rust/tonk-account-service/src/store/d1.rs`
- Test: `rust/tonk-account-service/tests/registration.rs`

**Interfaces:**
- Consumes: verified caller account, target device DID, device name, verified root-to-device delegation, and current time.
- Produces: `RegisteredDevice { attachment_id, delegation_hex, delegation_cid, reused }` representing the one active generation the callback must deliver.

- [x] Add a test where `/devices/register` succeeds, its response is discarded, and the same account registers the same device again; the second response must identify one usable active generation and leave exactly one active row.
- [x] Add a concurrent retry test and a cross-account test; concurrent same-account calls converge, while a different account receives the existing privacy-safe conflict and neither row changes.
- [x] Run `cargo test -p tonk-account-service --features helpers register -- --test-threads=1`; expect same-account retry to fail on `devices_one_active_did`.
- [x] Add one atomic store operation that either inserts a first generation or retires/reuses the same-account active generation and returns the canonical grant that matches it. Do not implement this as a check followed by two independent writes.
- [x] Keep browser `/devices/link` behavior compatible and ensure revoked/detached history remains immutable and list deduplication still exposes only the actionable generation.
- [x] Run the focused registration, exact-generation recovery/detach, and full-ceremony HTTP proofs recorded below.
- [ ] Run the complete `cargo test -p tonk-account-service --features helpers -- --test-threads=1` package suite; omitted from this disk-constrained shared-target window and left to CI.

### Task 2: Recover an attachment omitted by an outdated page/worker

**Files:**
- Modify: `rust/tonk-account-service/src/lib.rs`
- Modify: `rust/tonk-account-service/src/handlers/devices.rs`
- Modify: `rust/tonk-account-service/src/core/devices.rs`
- Modify: `rust/tonk-ui/src/account.rs:CLI callback approval`
- Modify: `rust/tonk-cli/src/account.rs:CallbackAuthorization,account_from_callback`
- Test: `rust/tonk-account-service/tests/registration.rs`
- Test: `rust/tonk-cli/src/account.rs`

**Interfaces:**
- Consumes: an invocation signed by the callback's validated root-to-device chain for `account/device/attachment`.
- Produces: the active `attachmentId` and delegation CID only when account root, device DID, and grant generation match; otherwise a typed conflict/not-found without changing state.

- [x] Add CLI tests for unversioned callbacks with omitted, CID-placeholder, and explicit attachment fields: each validates the grant and asks the provider for its exact generation; a mismatched provider row remains a hard pre-write error.
- [x] Add provider tests for authorized exact match, wrong account, wrong delegation CID, detached generation, and unknown DID.
- [x] Run focused tests; expect the CLI's current `authorization is missing its service attachment generation` failure.
- [x] Add the authenticated lookup route and build its invocation from the already validated callback chain before any local account write.
- [x] Version new callback JSON (for example `schemaVersion: "tonk.cli-authorization.v2"`) and have the browser deliver the provider-returned canonical delegation/attachment fields. Continue parsing unversioned legacy fields.
- [x] Keep fallback `serviceUrl` behavior for legacy payloads, but refuse a recovered attachment from a provider whose root/generation does not match the validated grant.
- [x] Re-run the focused CLI/provider proofs and the account-service, worker, and UI `wasm32-unknown-unknown` compile boundaries recorded below.

### Task 3: Queue detach before clearing active local state

**Files:**
- Modify: `rust/tonk-cli/src/account_session.rs:AccountSessionState,logout_transition_for_store,deliver_detach`
- Modify: `rust/tonk-cli/src/account.rs:logout_with_operator_in`
- Test: `rust/tonk-cli/src/account_session.rs`

**Interfaces:**
- Consumes: `ActiveAccount`, device signer, and current unix time while the exclusive account-session transition lock is held.
- Produces: version-2 `AccountSessionState { active, pending_login, pending_detaches }` and `PendingDetach { provider, signed_intent, queued_at }`.

- [x] Add a migration test loading a version-1 state as version 2 with an empty outbox, without rewriting malformed/unsupported states.
- [x] Add an offline logout test asserting active state clears, profile/spaces remain, and one signed exact-generation detach survives process restart.
- [x] Run `cargo test -p tonk-cli account_session -- --test-threads=1`; expect failure because logout currently returns an in-memory row and records no retry.
- [x] Sign each detach intent and persist it in the outbox before clearing `active`/`pending_login` in the same locked state write. Deduplicate by provider plus attachment ID.
- [x] Split dispatch from transition: delivery reads queued signed intents and removes only terminal success outcomes under the lock; timeout/network/5xx leaves the item unchanged.
- [x] Make logout return success once local transition is durable. Its warning must say the local account is signed out, spaces/identity remain safe, provider cleanup is queued, and a later online account command retries it.
- [x] Re-run focused tests.

### Task 4: Retry provider cleanup without blocking sign-in recovery

**Files:**
- Modify: `rust/tonk-cli/src/account.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs:account command dispatch`
- Test: `rust/tonk-cli/tests/account_interrupt.rs`
- Test: `rust/tonk-cli/src/account_session.rs`

**Interfaces:**
- Consumes: durable pending detaches and a bounded provider request (existing ten-second request timeout).
- Produces: `flush_pending_detaches` summary with retired, retryable, and permanently malformed counts; a retryable old detach never prevents a new same-device registration.

- [x] Add a restart test: logout while provider is unavailable, start a new CLI process, restore provider, run account status/login, and assert the outbox drains and provider row becomes detached.
- [x] Add stale-generation and settlement tests: defer an old intent if login has re-adopted its exact generation, treat a provider `superseded` receipt as terminal after a newer generation becomes active, and retain login/logout guards through outer registry settlement.
- [x] Run the restart process proof against a marker-verified branch binary: an offline logout survives process restart, later online status drains the outbox, and the provider row becomes detached.
- [x] Flush best-effort at account login/logout/status boundaries and after successful callback activation. Bound total cleanup work and report only once per command on stderr.
- [x] Ensure registration convergence from Task 1 lets login proceed even while an older cleanup item is retryable.
- [x] Run the focused `account_session` library tests, callback compatibility test, retained-guard tests, and the external-process restart filter recorded below.
- [ ] Run the complete `account_interrupt` and `tonk-cli --lib` suites; omitted from this disk-constrained shared-target window and left to CI.

### Task 5: Document and verify interruption outcomes

**Files:**
- Modify: `rust/tonk-cli/README.md`
- Modify: `docs/storybook/accounts/browser-cli-handoff.md`
- Modify: `docs/storybook/verification/accounts.md`

**Interfaces:**
- Consumes: callback v2, legacy recovery, registration convergence, and detach outbox semantics.
- Produces: exact recovery instructions for Ctrl-C, lost callback response, outdated worker, offline logout, and cross-account conflicts.

- [x] Remove or correct references to nonexistent `--abandon-detach`; document only commands and states backed by tests.
- [x] Add HANDOFF rows that discard the registration response, omit `attachmentId`, terminate CLI after callback, log out offline, and retry under same/different account.
- [x] Run `python3 docs/storybook/scripts/build.py`, `python3 docs/storybook/scripts/build.py --check`, and `python3 docs/storybook/scripts/check-links.py docs/storybook`.
- [x] Run `cargo fmt --all -- --check` plus the focused provider, CLI, external-process, and account-service/worker/UI Wasm checks after the final production changes.

## Verification evidence

Observed red before implementation:

- Same-account registration retry reached the active-DID uniqueness conflict instead of returning the committed row.
- `POST /devices/attachment` returned 404 because no authenticated recovery route existed.
- A valid legacy callback without an attachment generation failed before activation.
- A version-1 account-session fixture remained version 1 instead of migrating in memory to the version-2 outbox shape.

Fresh focused green evidence after the final production changes:

- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo test --config 'profile.test.package.tonk-cli.codegen-units=19' -p tonk-account-service --features helpers --lib registration -- --nocapture --test-threads=1`: 4 passed, 0 failed, 42 filtered out.
- `/Users/jackdouglas/tonk/tonk/target/debug/deps/tonk_account_service-b7af30055311cbd3 attachment_recovery_requires_the_exact_active_grant_generation --nocapture --test-threads=1`: 1 passed, 0 failed, 45 filtered out.
- `/Users/jackdouglas/tonk/tonk/target/debug/deps/tonk_account_service-b7af30055311cbd3 it_detaches_only_the_exact_signed_generation_idempotently --nocapture --test-threads=1`: 1 passed, 0 failed, 45 filtered out.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo test --config 'profile.test.package.tonk-cli.codegen-units=19' -p tonk-account-service --features helpers --test service it_drives_the_full_ceremony_over_http -- --nocapture --test-threads=1`: the sandboxed run could not bind loopback (`Operation not permitted`) before product behavior; the unchanged command with loopback permission passed 1 test, with 0 failed and 6 filtered out.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo test --config 'profile.test.package.tonk-cli.codegen-units=19' -p tonk-cli --lib account_session -- --nocapture --test-threads=1`: 18 tests ran; 16 passed and 2 loopback tests were denied by the sandbox before behavior. The unchanged fresh executable `/Users/jackdouglas/tonk/tonk/target/debug/deps/tonk_cli-e5e77828a77d5dd4 account_session --nocapture --test-threads=1` with loopback permission passed all 18, with 0 failed and 177 filtered out.
- The same fresh CLI library executable passed `it_recovers_a_callback_without_an_attachment_generation`, `link_outcome_retains_handoff_exclusion_through_outer_settlement`, and `logout_outcome_retains_exclusion_through_outer_registry_clear` individually: 1 passed, 0 failed, 194 filtered out for each. The callback test's first sandboxed run was denied loopback before behavior; its unchanged loopback-permitted retry passed.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo test --config 'profile.test.package.tonk-cli.codegen-units=19' -p tonk-cli --features integration-tests --test account_interrupt --no-run`: compiled the fresh process-test executable `/Users/jackdouglas/tonk/tonk/target/debug/deps/account_interrupt-b97b77b7cbb42b18` and CLI binary without warnings. The binary contained the branch-specific handoff marker before it was copied to an isolated temporary path.
- `NEXTEST_BIN_EXE_tonk=/tmp/tonk-audit-cli-handoff-recovery /Users/jackdouglas/tonk/tonk/target/debug/deps/account_interrupt-b97b77b7cbb42b18 offline_logout_cleanup_survives_a_process_restart --test-threads=1`: 1 passed, 0 failed, 2 filtered out in 0.97 seconds with loopback permission. The exact temporary binary was removed afterward.

Fresh compile and static evidence:

- `cargo fmt --all` completed, and the final `cargo fmt --all -- --check` passed.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo check --config 'profile.dev.package.tonk-cli.codegen-units=19' -p tonk-account-service --target wasm32-unknown-unknown`: passed in 26.14 seconds; reported one target-specific unused-import warning for `serde::Deserialize` in `handlers/accounts.rs`.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo check --config 'profile.dev.package.tonk-cli.codegen-units=19' -p tonk-worker --target wasm32-unknown-unknown`: passed in 27.21 seconds without warnings.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/jackdouglas/tonk/tonk/target cargo check --config 'profile.dev.package.tonk-cli.codegen-units=19' -p tonk-ui --target wasm32-unknown-unknown`: passed in 27.32 seconds; reported dead-code warnings for `focus_input`, `resettle`, and `register_claim`.
- `python3 docs/storybook/scripts/build.py`: 26 screens, 78 journeys, 121 verification items, 6 triage findings; generated `data.json` and `data.js` refreshed.
- `python3 docs/storybook/scripts/build.py --check`: passed with the same counts.
- `python3 docs/storybook/scripts/check-links.py docs/storybook`: 178 local references valid.
- `git diff --check`: passed after the final production changes.

Behavioral assumptions and explicit verification boundaries:

- `tonk account status` keeps its durable account answer local, but may now make two bounded best-effort network attempts: detach-outbox cleanup and the access-service customer probe. A network failure does not replace the local answer. Visible retry copy therefore says to run an **online** account status; “still queued” is emitted only for a confirmed pending count, while an unreadable state reports only that cleanup could not be checked.
- The compatibility boundary is intentional: v2 callbacks carry the canonical `attachmentId`, `delegationCid`, `delegationHex`, and `reused`; unversioned callbacks and an omitted `serviceUrl` remain accepted only after authenticated provider recovery validates the exact active root/device/grant generation. Bare identifiers never authorize recovery.
- The D1 adapter compiled through the account-service Wasm boundary, but no deployed Worker, live D1 binding, production migration, or remote concurrency test was run locally.
- No real-browser, passkey, service-worker-upgrade/reload, stale-tab, Safari, or callback-navigation scenario was exercised. The browser contract was checked through Rust/Wasm compilation, focused callback tests, and Storybook source/generated data only.
- The complete account-service, CLI library, and process integration suites were not run locally. The shared target and filesystem ended with less than 4 GiB free, so the bounded matrix above is the final local Cargo scope; CI remains responsible for the full matrix.
