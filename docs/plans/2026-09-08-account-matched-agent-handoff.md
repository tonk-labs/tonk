# Account-matched agent handoff implementation plan

**Goal:** Connect an agent as the account that generated the browser handoff, asking before changing the CLI account and preserving the old account until the replacement is activated.

**Approach:** Use an invitation scoped to the browser account and enforce its audience before any claim or resume. Extend native account activation with a recoverable replacement transition, then wire explicit switch consent and browser authorization into `connect`.

**Constraints:**
- Approved behavior: require the browser account (not the repository owner); ask before switching; preserve A until B is approved and activated; retain cross-account `join`.
- A scoped space invitation identifies the required account but does not authorize account login. Browser passkey approval still supplies the account-to-device grant.
- No automatic logout, space deletion, provider revocation, cache reset, or credential deletion. This does not introduce selectable account profiles.
- Only one account is active. Preparing a replacement must not authorize remote account operations as that replacement.
- Preserve unrelated working-tree edits. At planning time `rust/tonk-core/assets/library/core.yaml` has local blank-canvas and receipt styling changes.
- No dependency or lockfile changes are expected. Use the repository Nix environment and Rust formatting. Newly added CLI integration tests require explicit Cargo registration (`autotests = false`).
- Do not infer hosted or Safari success from native or synthetic browser tests. Do not commit, publish, or deploy implicitly.

## Current evidence and compatibility

- `connect_agent` in `rust/tonk-cli/src/bin/tonk.rs` reuses any active account and resumes a matching local invite before checking account state.
- `invite::claim` chooses the local account root and `Invite::claim` already rejects a scoped invitation addressed to a different root.
- `account_session::validate_state` forbids simultaneous `active` and `pending_login`. `stage_activation` accepts only signed-out activation.
- `account::complete_staged_account` currently projects the local root/provider before finalizing. Those projections span multiple writes; simply relaxing the state invariant is insufficient.
- `identity::local_root_with_operator` reads a credential projection directly. All readers used during replacement must agree with canonical session state or participate in recovery/locking.
- `account::link_via_callback` validates a callback before activation. `tonk-identity/src/install.rs::authorize_device` unlocks account custody before issuing the device grant.
- Worker `create_invite` already accepts an audience DID. The blank canvas instead dispatches the ordinary `tonk:invite` command and joins its shared invitation data into `tonk:agent-invite`.
- Existing spaces freeze seeded descriptors and views. New source YAML alone does not update old prompts. Old open invites must fail clearly under the new `connect` contract; they remain usable with ordinary `join`.

## File map

- `rust/tonk-cli/src/account_session.rs`: durable replacement state, exact-generation locking, commit/recovery.
- `rust/tonk-cli/src/account.rs`: expected-account callback validation and replacement orchestration.
- `rust/tonk-cli/src/identity.rs`, `account_state.rs`, `space.rs`: coherent account projections, account repository selection, registry reconciliation.
- `rust/tonk-cli/src/handoff.rs`, `invite.rs`, `bin/tonk.rs`: scoped preflight, explicit consent, local resume identity and command routing.
- `rust/tonk-cli/tests/account_interrupt.rs`, `account_authority.rs`, `handoff.rs`, `join_profile.rs`: transition, CLI, receipt, and collaboration regressions.
- `rust/tonk-worker/src/router/repository.rs`, `create_invite.rs`, `ceremony.rs`: separate handoff minting and expected-account browser authorization.
- `rust/tonk-schema/src/command.rs`, `domain.rs`, `rust/tonk-core/assets/library/core.yaml`: distinct handoff command/state.
- `rust/tonk-workspace/src/ui_account_settings.rs`, `.html`, `rust/tonk-worker-api/src/identity.rs`: actual approval-page input and ceremony request (the approval page is not seeded in `profile.yaml`).
- `rust/tonk-identity/src/install.rs`: check unlocked root against the requested account before issuing a grant.
- `rust/tonk-worker/src/router.rs`, `rust/tonk-worker/tests/standard_library.rs`, `rust/tonk-ui/src/account_flow.rs`: worker, seeded-library and browser acceptance coverage.
- `rust/tonk-cli/README.md`: account-matched connect, consent, legacy-prompt recovery.

### Task 1: Prove recoverable account replacement

**Files:**
- Modify: `rust/tonk-cli/src/account_session.rs`, `account.rs`, `identity.rs`, `account_state.rs`, `space.rs`.
- Test: inline `account_session` tests and `rust/tonk-cli/tests/account_authority.rs`.

**Interfaces:**
- Consumes: `ActiveAccount`, canonical session state, existing shared/exclusive transition guards.
- Produces: `replace_account_in(profile, store, expected_previous: &ActiveAccount, replacement: &ActiveAccount) -> Result<()>` in `account.rs`; replacement must already have passed callback and expected-root validation.
- Add a versioned replacement journal containing exact previous and replacement generations and a unique operation identifier. Keep it distinct from `pending_login`; migrate v1 state without changing the active attachment. Old binaries must reject the new version rather than ignore replacement state.

- [x] Add `replacement_failure_preserves_previous_account`: fixture A active, validated B ready; inject failure before each projection and canonical commit; fresh readers must recover A, never use B's account grant with A's root or provider.
- [ ] Run `cargo test -p tonk-cli --lib replacement_failure_preserves_previous_account`; initially expect absence of the replacement operation/test compilation failure.
- [x] Implement the smallest replacement transaction under the exclusive transition guard. Compare the complete previous generation, not just its DID; reject a stale callback after logout or another transition. Persist recovery material before touching projections.
- [x] Treat the canonical active-state replacement as the commit point. Before commit, recovery restores projections from A; after commit, recovery completes projections from B. A post-rename fsync error has an uncertain commit result: reread canonical state and report the actual state; never blindly restore A.
- [x] Audit direct root/provider/registry readers and account repository mounting. Readers must acquire the appropriate guard and reconcile an interrupted replacement before using projections. Keep account directories and already joined spaces intact. Do not hold the transition lock during browser approval or network hydration.
- [ ] Add `replacement_restart_recovers_each_commit_boundary`, `replacement_rejects_stale_previous_generation`, `replacement_readers_never_mix_accounts`, and `replacement_preserves_local_spaces`. Include B hydration failure after successful activation: B stays active with a sync warning, not a false rollback claim.
- [x] Run `cargo test -p tonk-cli --lib account_session`; then `cargo test -p tonk-cli --features integration-tests --test account_authority`.
- [x] Review this checkpoint before adding browser or invitation changes. If the existing storage model cannot meet crash-recovery invariants, report the failing boundary and revise this task explicitly; do not substitute logout-first behavior.

### Task 2: Authorize exactly the expected account without disturbing A

**Files:**
- Modify: `rust/tonk-cli/src/account.rs`, `rust/tonk-workspace/src/ui_account_settings.rs`, `ui_account_settings.html`, `rust/tonk-worker-api/src/identity.rs`, `rust/tonk-worker/src/router/ceremony.rs`, `rust/tonk-schema/src/command.rs`, `rust/tonk-identity/src/install.rs`.
- Test: inline account and identity tests, `rust/tonk-cli/tests/account_interrupt.rs`, `rust/tonk-ui/src/account_flow.rs`.

**Interfaces:**
- Consumes: Task 1 replacement operation and existing callback listener/validation.
- Produces: `link_expected_in(profile, store, options, expected_root: &Did, expected_previous: Option<&ActiveAccount>) -> Result<LinkOutcome>`.
- Carry `expectedAccount` in the approval URL and authorization request. It is a constraint, never proof of account authority. Ordinary login can omit it.

- [x] Add `expected_account_callback_rejects_other_root_without_writes`: waiting for B, valid callback for C; canonical A, projections, registry and space bindings remain unchanged.
- [ ] Run `cargo test -p tonk-cli --lib expected_account_callback`; expect missing API/test compilation failure, then implement the scoped callback path.
- [x] Separate callback acquisition from activation. Keep A active throughout the wait. Validate grant signature, device audience, account scope, service attachment and expected root before any activation write. Recheck A's exact generation under the Task 1 guard before replacement.
- [x] Browser approval displays the expected account and rejects an unlocked different root before minting its device grant. CLI independently repeats the root check. Preserve a synchronous user gesture for passkey invocation.
- [ ] Add denial, Ctrl-C, malformed callback, wrong device audience, and browser-wrong-account cases. No case may log A out. Successful approval activates B once; retry after post-activation sync failure must not reopen approval.
- [x] Run `cargo test -p tonk-cli --lib account`; then `cargo test -p tonk-cli --features integration-tests --test account_interrupt`.
- [ ] Run `nix develop --accept-flake-config . -c test:web:debug -E 'package(tonk-identity) | package(tonk-worker)'` at this integration checkpoint.

### Task 3: Enforce account identity in connect, including resume

**Files:**
- Modify: `rust/tonk-cli/src/handoff.rs`, `invite.rs`, `bin/tonk.rs`, `rust/tonk-cli/README.md`.
- Test: inline handoff/parser tests, `rust/tonk-cli/tests/handoff.rs`, `join_profile.rs`.

**Interfaces:**
- Consumes: scoped `Invite` audience and Task 2 `link_expected_in`.
- Produces: `preflight_connect(url) -> Result<ConnectInvite>` with `url`, `subject`, `invitation`, `expected_root`; derive expected root from the validated scoped chain, not a free-standing query parameter.
- Add `connect --switch-account <DID>` as explicit consent for the exact target. Without it, a mismatch returns nonzero with current and expected DIDs and the rerun instruction. This supports agents asking their user before rerunning and requires no terminal interaction.
- Add a separate versioned local `agent-handoff.json` containing only subject, invitation entity and expected root. Preserve the existing `claimed-invitation` file and ordinary join retry behavior.

- [ ] Add `connect_rejects_open_invite_before_mutation`, `connect_mismatch_requires_explicit_target`, and `connect_resume_checks_account_before_binding`.
- [ ] Run `cargo test -p tonk-cli --lib handoff`; expect the new preflight/metadata interface to be missing, then implement it.
- [x] Route both fresh claims and matching-invite resumes through the same identity gate before bind, claim, pull, roster writes or receipt writes. A matching active B proceeds; signed-out starts approval; A requires the exact switch flag. A mismatched switch flag fails before browser opening.
- [x] For URL-free `--space NAME connect`, require valid local handoff metadata and verify its subject against the opened repository. Old replicas without metadata must request the new scoped handoff URL. Do not infer expected identity from a replicated roster or the currently active account.
- [x] On URL-bearing resume, compare scoped audience with the authority installed on the replica; refuse reuse when it belongs to another root. Offer a fresh local alias rather than rewriting existing authority.
- [x] Persist handoff metadata before confirmation so interruption is resumable. Keep bearer URLs out of metadata. An old open prompt explains that a new account-scoped handoff is required; do not silently turn it into a cross-account join.
- [x] Retain `join` open/scoped behavior. Only emit connection success after pull and receipt push succeed.
- [x] Run `cargo test -p tonk-cli --bin tonk account_spaces_parser_tests`; `cargo test -p tonk-cli --test handoff`; `cargo test -p tonk-cli --test join_profile`.

### Task 4: Generate a distinct browser-account handoff

**Files:**
- Modify: `rust/tonk-schema/src/command.rs`, `rust/tonk-worker/src/router/repository.rs`, `create_invite.rs`, `rust/tonk-core/assets/library/core.yaml`.
- Test: `rust/tonk-worker/src/router.rs`, `rust/tonk-worker/tests/standard_library.rs`, `rust/tonk-cli/tests/handoff.rs`.

**Interfaces:**
- Consumes: existing scoped mint implementation and the current browser account root.
- Produces: a distinct `tonk:agent-handoff` command and overlay response keyed separately from ordinary `InviteState`/`Credential`; its ready response contains the scoped URL and expected account DID.
- Feed `tonk:agent-invite` from this handoff response, retaining repository display-name lookup. New concept fields require descriptions.

- [x] Add `agent_handoff_targets_current_account_not_owner`: browser B in C-owned space produces a chain addressed to B; an ordinary share remains audience-open.
- [ ] Run `nix develop --accept-flake-config . -c test:web:debug -E 'package(tonk-worker) & test(agent_handoff)'`; first expect the missing command/response test to fail.
- [x] Extract/reuse scoped minting with the existing remote/provisioning checks. Resolve the target from browser session identity, never from space ownership or arbitrary request audience. Refuse signed-out handoff generation.
- [x] Capture the browser account generation while minting and verify it remains current before publishing/copying. Account changes invalidate the old response; share and agent handoff responses must not overwrite each other.
- [x] Change the blank canvas to request the distinct handoff and update the copied prompt: explain mismatch to the user; rerun with the exact switch flag only after consent; wait for browser approval and confirmed receipt. Do not change the unrelated local canvas/toast styling.
- [ ] Add repeated minting, browser account change, ordinary-share coexistence, missing-account, revoked authority, and unprovisioned-space cases.
- [x] Verify new seeded views render; document that existing frozen views need the repository's explicit library refresh mechanism before they emit new prompts. Do not silently rewrite existing spaces as part of connect.
- [ ] Run `nix develop --accept-flake-config . -c test:web:debug -E 'package(tonk-worker)'`; then `cargo test -p tonk-cli --test handoff`.

### Task 5: Verify the complete two-account flow

**Files:**
- Modify: `rust/tonk-ui/src/account_flow.rs`, `rust/tonk-cli/tests/handoff.rs`, `account_interrupt.rs`.
- Update: this plan with evidence and `rust/tonk-cli/README.md` with final supported behavior.

**Interfaces:** Consumes all preceding behavior; produces regression coverage and a recorded native/browser/hosted verification boundary.

- [x] Add an end-to-end regression: CLI A, browser B, B-scoped prompt. First run prints mismatch, returns nonzero and leaves A/space/roster/receipt unchanged. Explicit consent opens approval; B approval switches once, joins as B, and publishes the receipt. A never gains membership.
- [ ] Add same-account/no-approval, signed-out/B-approval, cancelled switch/A-preserved, wrong-root/A-preserved, interrupted activation recovery, URL-free resume mismatch, and receipt-push failure/retry cases. Include B collaborating in C's space.
- [x] Run the focused new browser test using the existing account-flow harness; then `nix develop --accept-flake-config . -c test:web:debug -E 'package(tonk-ui) | package(tonk-worker) | package(tonk-identity)'`.
- [x] Run `cargo fmt --all -- --check`, `cargo test -p tonk-cli --features integration-tests`, and `git diff --check` once the final source changes are complete. Run native Cargo commands inside the repository Nix shell when required by the local toolchain.
- [x] Inspect the actual copied prompt and approval page in a browser. Verify passkey cancellation and account mismatch with isolated test accounts; no user account switching for testing without explicit authorization. Record Safari verification separately from Chrome automation.
- [x] Report any hosted provisioning, physical passkey, old-space refresh or Safari gaps. Do not call local receipt tests a hosted round trip.

## Review and handoff state

The implementation and isolated Chrome acceptance are complete. The evidence below records the actual test coverage; original red-test commands and unchecked compound matrix items are not claims that every named scenario received its own dedicated test. Hosted production and Safari remain outside this local verification.

### Implementation

- Replacement uses a v2 canonical journal with the exact prior generation. Fault injection covers each grant/root/provider projection, precommit, and postcommit restart. A remains active until canonical commit; B remains active after a later hydration failure. V1 migration preserves its active generation and records the original branch owner.
- The first cross-account hydration regression reproduced A's private facts leaking through the shared profile `main` branch. Account-root-specific content branches and remote records now isolate replacements. The original account retains `main`/`origin`; local spaces and credentials remain intact. A retained A handle with an actual unpushed revision cannot push after B becomes active.
- Expected-account callback validation is independent of the scoped invite. Native denial/malformed/wrong-root checks preserve A. Browser approval displays the expected DID and checks it after passkey unlock, before granting authority.
- Connect uses validated scoped audience and exact `--switch-account` consent. URL-free retries validate local metadata, repository subject, local claim marker, and installed root prefix before approval. Receipt success requires pull and push. Metadata contains exactly four fields and no bearer URL.
- Browser handoff has independent command and session overlay attributes; it captures root plus delegation generation. Ordinary share remains open. Browser acceptance consumes the actual copied blank-canvas prompt.
- Existing-space compatibility is explicit: `tonk --space NAME eval /path/to/tonk/rust/tonk-core/assets/library/core.yaml` replaces the frozen canvas and carries the new handoff event. A typed-replica regression passes; README documents the opt-in workflow. No automatic library migration was added.
- Concurrent changes in `docs/plans/2026-09-08-handoff-empty-state-fixes.md` and `rust/tonk-render/tests/compat.rs` are preserved. Browser acceptance uses the combined final library, including separate pending/prompt attributes and receipt visibility.

### Verification

- Full native CLI suite passed: `cargo test -p tonk-cli --features integration-tests --no-fail-fast -- --test-threads=2`, 580 passed, 0 failed, 1 ignored across 29 binaries including doc tests. The tested CLI is copied to `/private/tmp/tonk-account-handoff-complete` for browser acceptance.
- Full worker suite passed before the subsequent focused approval-decoder fix: `cargo nextest run -p tonk-worker --target wasm32-unknown-unknown --test-threads 2 --no-fail-fast`, 372 passed. An earlier immutable archive also passed all 118 selected identity/UI runtime tests.
- The initial current archive run had two test expectation errors: legacy refusal field selection and HTTP 200 versus successful join 201. Corrected both; all three focused handoff tests passed, followed by the full worker pass above.
- Browser acceptance initially passed expected-account authorization and signed-out handoff refusal. The two-account flow stopped during initial ordinary CLI login: the transient decoder does not realize an absent optional field. A narrowly scoped original-command decoder now handles omitted `expectedAccount`, while a supplied malformed constraint cannot fall back. The focused decoder regression passes absent, valid, and malformed cases.
- Wrong-account browser acceptance initially required exact equality with a substring. Corrected the wait; the final browser test passed and confirmed no grant was delivered and the browser root stayed unchanged.
- Final `cargo fmt --all -- --check` and `git diff --check` passed.
- Nix cache HTTP 401 warnings fall back to local builds. Use exact returned artifact store paths: separate build/eval wrapper calls can select different source snapshots while editing.

- Final Chrome acceptance: 4 passed, 76 unrelated tests skipped. The two-account test needed one configured retry after an initial passkey-consent timeout; its second attempt passed in 15.8s. It copied the real prompt, refused an unconsented mismatch, preserved A after denial, approved B, confirmed the pushed receipt, and resumed without the URL or another approval.
- Native callback regression reproduced a delivered authorization waiting on an unfinished second browser connection. Delivery now has a bounded one-second response drain; all 12 callback tests pass.
- Native self-account handoff regression reproduced the absent reusable prefix when the invitation traverses its own account. Claim now recovers and stores an already-signed prefix from installed authority, without minting a new hop. All 13 handoff tests and the final 580-test native suite pass.
- Local browser/CLI acceptance uses `TONK_TEST_WEB_HOST=localhost`: Chrome's private DNS mapping alone does not make the production-shaped test hostname resolve to loopback for the CLI. CI retains its existing hostname mapping; no machine hosts file was changed.
- Browser server: `/nix/store/5zqp4anrsf44r3nggsybm3bmgzpv8c85-tonk-ui-test-server/bin/tonk-ui-test-server`. Final test command used local incremental `cargo nextest run -p tonk-ui --features integration-tests --profile e2e` with that exact server and the tested CLI copy. Final log: `/private/tmp/handoff-browser-completion.log`; native log: `/private/tmp/handoff-native-completion.log`.
- Inspected actual app screenshots `/private/tmp/tonk-handoff-review/copied-prompt.png` and `expected-account-approval.png`. The prompt copied successfully and the approval view displayed the complete expected DID. This is Chrome with a virtual passkey, not physical-device validation.

### Remaining boundary

- The handoff implementation includes the final callback/prefix fixes, acceptance helpers, and verification evidence. Chrome acceptance is complete. No publication, deployment, user-account switching, hosted production check, physical-passkey check, or Safari check has been performed. Local fixture receipt success does not establish a hosted production round trip.
- Commit-time `cargo fmt --all -- --check` and `git diff --check` passed. Prior full-suite results above were not rerun solely for the commit.
