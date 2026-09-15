# Reconcile the profile library across boot and account sync

## Outcome and scope

Existing profiles should resolve the shipped `rust/tonk-core/assets/library/profile.yaml` after worker boot, profile activation, and a successful account synchronization. An account created with a Hub template containing `no spaces yet` should adopt the current template while preserving its spaces, identity, credentials, and independently authored profile content. The repository filename is `profile.yaml`, although this task was requested as `profile.yml` reconciliation.

Keep the existing architecture: profile definitions remain on profile `main`, which replicates through the account upstream. Add a profile-specific reconciliation operation that installs current definitions, withdraws obsolete definitions whose ownership can be established, and records the installation atomically. Reuse the evaluator and reactor transaction machinery. Do not broaden this work into named-space upgrades, Welcome snapshot migrations, browser-storage cleanup, or moving the whole profile library to an overlay.

The compatibility tradeoff is eventual repair on current clients. An old client can still publish old definitions after seeing newer ones; content hashes do not establish release order. This plan does not promise permanent monotonic upgrades while old writers remain active, or guarantee that no transient old template is ever observed between a pull and reconciliation. Continuous cross-version isolation would require a separate design for runtime-local system definitions or a version protocol. Do not add a repeated network fetch and full library evaluation to every unchanged sync sweep.

## Evidence and boundaries

`bootstrap_profile` in `rust/tonk-worker/src/router/repository.rs` already seeds on boot and profile activation. `seed_profile_library` fetches the library, writes a seed record with `prior` and `replaces` both set to `seed:none`, and invokes `evaluate_profile_body_recording`. That evaluator supplies an empty retraction list. Seeding is best effort: errors are logged and boot continues.

`boot_state` in `rust/tonk-worker/src/worker.rs` calls bootstrap before later account synchronization. `configure_account_upstream`, `hydrate_untrusted`, and `sync_ready` in `rust/tonk-worker/src/router/account_state.rs` subsequently attach and pull account data into profile `main`. Neither successful pull path reconciles the profile library afterward. `converge_account_state` currently handles account and space projections and has a native no-op implementation; it is not a suitable place to hide a platform-independent library repair.

Named-space `upgrade_seed` supplies useful mechanics: `prior_seed_retractions`, `seed_record_facts`, and `evaluate_with_retractions`. It is not directly reusable as policy. It targets named-space content branches, deliberately skips unrecorded seeds, and calls `read_installed_seed`, which selects the first installation returned by an unfiltered query. Profile histories can contain multiple installation records and unrelated libraries. Installation metadata is committed in a second staged commit, so inverting the first commit alone does not retire the old metadata.

The current Hub directory template already omits the text, and `it_builds_one_centered_hub_launcher_with_a_settings_route` in `rust/tonk-worker/tests/standard_library.rs` asserts its absence. Earlier investigation ran that test successfully and saw no message in a fresh local browser profile. Those checks establish current-source behavior, not an existing-account upgrade. The screenshot is consistent with an old template and newer CSS; its exact runtime cause has not been reproduced. Treat post-pull replacement and competing claims as hypotheses to exercise, not a proven trace from that browser.

## Reconciliation contract

Expose a profile-scoped operation from `repository.rs` (or a small sibling module if extraction improves readability), conceptually `reconcile_profile_library(&TonkState) -> Result<ProfileLibraryOutcome, RepositoryError>`, with outcomes distinguishing unchanged state from an installed or repaired library. Keep acquisition of library bytes separate from installation so tests can supply historical documents and failures without replacing production logic.

The target is exactly `reactor.profile_repository().branch(PROFILE_BRANCH)`. Select provenance by the exact `/library/profile.yaml` source. Never choose an arbitrary `SeedInstalled` row or interpret a hash as a version ordering. Account facts and unrelated library installations are outside this operation's ownership.

Plan changes against the current branch under the existing writer serialization. On a publish conflict, refresh and recompute both the reconciliation decision and its retraction set; replaying an outdated decision against a new head is insufficient. Avoid recursively acquiring the transactor when adapting `evaluate_on_branch_with`.

For recorded installations, use their actual installation histories to identify exact old assertions, and subtract assertions retained by the desired library. Reassert desired definitions in the same batch as removing obsolete owned assertions. Retire superseded profile installation metadata explicitly and record the new installation's actual staged version. Do not retract another library's metadata. Do not use a history scan over the whole account or namespace-wide deletion as a substitute for ownership. Independently authored values outside the system library's defined fields must survive.

For legacy installations without trustworthy provenance, supersede current system-owned definitions and facets by their known identities through normal evaluator replacement semantics. The shipped values are authoritative for those fields, including the entire Hub directory facet and its event bindings. Preserve unknown legacy definitions that cannot safely be attributed; full garbage collection of all historical profile content is outside scope. Invalid or unavailable provenance must remain distinguishable from a clean first install: repair known current fields where safe, report deferred cleanup, and do not claim complete migration.

A target hash alone does not prove correctness: a subsequent pull may have changed a template while retaining its installation record. The unchanged check must also establish that current system definitions resolve as intended. Cache validated input and the successfully checked branch revision within the active `TonkState`; invalidate on branch change, profile replacement, or changed library bytes. Do not write an install record or advance the branch on a true no-op. Preserve the existing development hot-swap path and do not pin a process-global cached library across rebuilds or profiles.

Use the existing served-asset fetch path initially. Verify its offline and worker-update behavior in the final browser step rather than assuming `RequestCache::NoStore` guarantees generation consistency. If current bytes cannot be acquired, preserve the last usable definitions, leave reconciliation pending, and retry through the next lifecycle opportunity. Do not mark failed attempts as checked or require account reauthentication for a library failure.

## Implementation tasks

### 1. Prove replacement behavior on a populated historical profile

- [x] A focused regression reproduces stale profile-library behavior and identifies the first incorrect transition.

Add fixtures and tests beside the repository seed tests in `rust/tonk-worker/src/router/repository.rs`. Use a historical profile template containing `no spaces yet`, one genuine directory entry, and independent sentinel facts for account data and an authored profile route. Include both a seed with recorded provenance and a legacy seed without it. Pin fixture provenance to a historical Git revision in a comment; never depend on live Git history at test runtime.

Drive the production profile seeding seam and query the resolved directory template through `tonk_template::resolve::view_query`; assert the entire expected facet rather than merely searching bundled text. Exercise old local seed followed by current boot seeding, then a later account pull carrying the old claims. Record whether the stale value occurs during replacement, merge, or subscription delivery. Include a direct read and a subscribed result to distinguish database state from mounted view state.

Name new tests with a shared `profile_library` substring. Run `cargo test -p tonk-worker profile_library -- --nocapture`. If a fixture requires the service-worker environment, run `nix develop -c test:web:debug -E 'package(tonk-worker) & test(profile_library)'`. The expected initial failure is a stale resolved facet after the relevant transition, obsolete recorded definitions surviving, or a missing reconciliation call. If boot plus pull already resolves correctly, record that result and narrow the bug claim before proceeding; do not manufacture a migration failure from a source-text assertion.

### 2. Make profile installation atomic, selective, and repeatable

- [x] Recorded and legacy fixtures resolve current definitions, preserve sentinels, and leave the branch unchanged on a second successful reconciliation.

Implement the reconciliation contract in `repository.rs`, adapting `rust/tonk-worker/src/router/evaluate.rs` with a profile-branch counterpart to `evaluate_with_retractions` or a shared branch-reference helper. Share `stage_and_publish`, parse/evaluation error handling, and CAS retry logic. Keep the named-space policy unchanged.

Add behavioral coverage for two historical profile records, multiple records for the same digest, a record from another source, a removed route, a retained shared definition, a changed directory facet and bindings, and unknown legacy content. Verify the new record points to the commit containing the library assertions, old profile install records cease masquerading as current, and unrelated records survive. Exercise malformed input and an injected publication conflict: either the old complete library remains or the new complete library and provenance land together. A retry must observe concurrently introduced claims.

Use the focused native and service-worker commands from task 1. Keep the existing static Hub contract as a separate guard, running `cargo test -p tonk-worker --test standard_library it_builds_one_centered_hub_launcher_with_a_settings_route -- --exact`. It supplements the migration test rather than replacing it.

### 3. Reconcile after both account pull paths and at local boot

- [x] Initial account hydration, a later ready-account pull, profile switching, and an offline restart all reach the defined reconciliation outcome without losing account state.

Replace the bootstrap seed call with the new operation. In `account_state.rs`, invoke reconciliation after successful hydration and after a successful `sync_ready` pull, before each path's push. Keep it outside the wasm-only account convergence implementation so native integration tests exercise the same behavior. Respect trusted-account and access adoption ordering; no definitions fetched from an untrusted account may trigger external commands during reconciliation.

Keep successful account hydration and library readiness separate. A library failure must not change an authenticated account back to unhydrated or prevent unrelated valid account writes from being pushed. Preserve a pending repair and return or combine a retryable sweep error after the push; retain both errors if reconciliation and push fail. Confirm the worker's sync scheduler actually retries the reported failure. Drain scheduled polls so mounted displays receive the repaired facet.

Extend the account fixtures in `account_state.rs` to publish stale profile definitions from one disposable device and pull them from another. Assert current templates after both hydration and ready sweeps, then repeat without upstream changes and verify no additional library commit or full evaluation. Test account A to B switching to ensure cached reconciliation state cannot leak between profiles. Inject fetch failure, restore availability, and verify repair on the next sweep without logout.

Run `cargo test -p tonk-worker profile_library -- --nocapture` and the focused Wasm selection from task 1. At this integration checkpoint run `nix develop -c test:native:debug -E 'package(tonk-worker) & (test(profile_library) | test(account_state) | test(profiles))'`.

### 4. Verify persisted-browser upgrades and document compatibility

- [x] An existing disposable profile upgrades across a worker update, retains its spaces, and repairs stale claims after another device publishes them; remaining mixed-version limits are recorded.

Add an upgrade scenario to the real-browser suite in `rust/tonk-ui/tests`, using the existing test environment and artifact-serving helpers. Prepare an old populated profile in an isolated browser, serve the current build, perform the normal worker update/reload, and inspect the Hub inside its rendered frame. Assert no obsolete sentence, current space names and working space links, working create/settings controls, and unchanged sentinel account data. Do not clear storage between builds. Capture the resolved template or installation digest as well as visible output.

Repeat with a new browser joining the existing account, then let a second disposable device publish a stale template after the current device has already reconciled. After the next successful sweep the current device must repair it. Stop the old writer and prove subsequent unchanged sweeps settle without repeated library commits. Document that active old writers may continue competing; this test does not establish permanent downgrade prevention.

Test a warm offline worker restart and a failed library acquisition followed by reconnection. Verify the running asset generation and profile-library input together. If the existing fetch path serves mismatched generations, fix that specific asset boundary with its own failing test before claiming update correctness; do not assume profile reconciliation alone fixes asset selection.

Run `nix develop -c test:e2e -E 'package(tonk-ui) & test(profile_library)'` for the new browser test. Final integration checks are `cargo fmt --all -- --check`, `nix develop -c test:native:debug -E 'package(tonk-worker)'`, and `nix develop -c test:web:debug -E 'package(tonk-worker) & (test(profile_library) | test(account_state) | test(profiles))'`. If service-worker asset handling changes, also run `nix develop -c test:sw`. Report browser/platform coverage explicitly; Chrome evidence does not establish Safari behavior.

## Handoff status

Implemented on 2026-09-14 and strengthened on 2026-09-15. Profile reconciliation now derives and validates a complete desired manifest, enforces exact values for owned cardinality-one fields, atomically replaces recorded definitions and provenance, repairs attributable legacy fields, and runs at boot and after both account pull paths. The active worker caches prepared input separately from per-profile validated revisions; retained production workers acquire profile bytes from their own immutable generation cache, while development hot swap replaces that input explicitly. Native and Wasm fixtures cover recorded, legacy, conflict-retry, hydration, ready-account, profile-switching, acquisition failure/retry, competing singleton values, concurrent acquisition, and no-op behavior.

Real Chrome scenarios cover a persisted worker upgrade, warm offline restart and reconnect, and a second account writer pinned to generation-A profile bytes. The mixed-writer test verifies the generation-A digest and stale rendering after a successful push, then verifies generation-B repair and preservation of account and space data. It does not independently inspect the remote head between writers, assert the published upstream revision, or count profile-library downloads across the final sweeps. A continuously active old writer can publish stale claims again. The browser evidence is Chrome-only; Safari and deployed-host behavior have not been verified.

The implemented correctness boundary is covered by fixture-backed reconciliation, account lifecycle tests, the persisted-browser upgrade without storage reset, and worker-scoped acquisition counters. Remaining follow-up evidence is tracked in [profile-library-review-followups.md](profile-library-review-followups.md): a dedicated profile publication-failure receipt test, a singleton conflict formed by a real divergent account merge, independent upstream inspection, and browser request counts. Completion does not mean every historical unowned definition has been removed or every deployed browser has already updated.
