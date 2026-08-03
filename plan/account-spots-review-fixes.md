# Account spots review fixes implementation plan

**Goal:** Keep account spot inventory available when one indexed artifact becomes unusable, make failed pulls retryable, and support documented local aliases deterministically.

**Approach:** Treat an indexed head as authoritative even when its blob can no longer be read or validated: log and omit only that subject, suppressing stale legacy fallback for the same subject while returning all healthy rows. Wrap creation of a pull target in a cleanup guard that is disarmed only after atomic registry persistence, then perform the best-effort initial sync after registration. Preserve aliases as valid registry state and consistently choose the lexicographically first usable registry name rather than rejecting duplicates.

**Constraints:**
- This plan fixes review findings on PR #674 (`feat/account-spots-cli`) without changing the account spot wire format, routes, UCAN commands, capability negotiation, or storage layout.
- A corrupt, missing, expired, or newly-invalid indexed artifact must not make `/chains/spots` fail for unrelated subjects.
- A head remains authoritative for its hashed subject slot even when unusable. Do not silently resurrect an older legacy blob or the lower-priority unnamed head for that subject; omit the subject and log enough storage identifiers to diagnose it.
- Failures listing the head/blob namespaces remain request-level storage failures. Only failures attributable to one enumerated head—blob fetch, missing blob, JSON decoding, backup validation, or subject-key mismatch—are isolated.
- Pull must remove a target that did not exist before the command when mounting, remote creation, upstream setup, canonicalization, or registry persistence fails. Once registry persistence succeeds, retain the site; an initial sync failure remains a warning and leaves a registered, retryable spot.
- Cleanup must never remove a path that existed before pull. If cleanup itself fails, preserve the primary error and print the exact remaining path in a warning so the user has an actionable recovery path.
- Multiple registry names for one repository are valid. Iteration over the registry's `BTreeMap` defines the winner: the lexicographically first usable name is used for account inventory display, already-local pull results, and backup fallback metadata.
- Preserve best-effort backup semantics and synced `RepositoryName` precedence over every CLI registry alias.
- Do not address the non-blocking R2 N+1, capability round trip, or per-eval validation performance suggestions in this change.
- Preserve all unrelated local account-logout work currently present in the worktree, including `plan/account-logout-cli.md`, the account source/documentation edits, and the new account tests.

## File map

- `rust/tonk-account-service/src/core/backup.rs`: isolate unusable indexed heads, suppress stale fallback for their subject keys, and add core regression coverage.
- `rust/tonk-cli/src/account_spots.rs`: own fresh pull-target cleanup, reorder registration before best-effort sync, and deterministically collapse local aliases for list, pull, current-site backup, and registry sweeps.
- `rust/tonk-cli/tests/account_spots.rs`: cover alias selection, deduplicated backup metadata, and retention after post-registration sync failure.

### Task 1: Isolate unusable indexed heads without reviving stale backups

**Files:**
- Modify: `rust/tonk-account-service/src/core/backup.rs:list_account_spots`
- Test: `rust/tonk-account-service/src/core/backup.rs:tests`

**Interfaces:**
- Consumes: existing `ChainStore::list_spot_heads`, `ChainStore::get`, `AccountSpotBackup::validate_for`, `subject_key`, and `crate::core::log_detail`.
- Produces: the existing `list_account_spots<C: ChainStore>(...) -> Result<Vec<AccountSpotSummary>, CeremonyError>` signature and response shape; no public interface changes.

- [ ] Add `it_omits_unusable_heads_without_poisoning_healthy_rows_or_reviving_legacy_rows`. Arrange one healthy indexed subject, then arrange another subject with an older valid blob written through `put_chain` and a named head manually redirected to an expired or malformed artifact. Assert `list_account_spots` succeeds, returns the healthy subject, and does not return the broken subject through either the bad head or its older valid legacy blob. Run `cargo test -p tonk-account-service --features helpers core::backup::tests::it_omits_unusable_heads_without_poisoning_healthy_rows_or_reviving_legacy_rows`; expect the current implementation to fail with `CeremonyError::Internal`.
- [ ] In `list_account_spots`, add every enumerated head's blob key to the selected-key set before attempting any fetch. Because named heads are iterated first, claim each stored subject key on first sight and skip a later unnamed head for the same key even if the named artifact is unusable. This ensures neither a failed selected blob nor a lower-priority head is reconsidered by the legacy scan.
- [ ] Replace per-head `?`/early returns for `get`, missing blobs, JSON parsing, `validate_for`, and subject-key mismatch with one log-and-continue path. Log through `crate::core::log_detail`, including account root DID, slot, stored subject key, blob key, and the concrete failure reason, but never artifact bytes or credentials.
- [ ] During legacy grouping, hash each validated candidate's subject and discard it when that hash belongs to any claimed head, including an unusable head. This prevents an old unindexed artifact from silently becoming current merely because validation rules tightened or the selected blob expired.
- [ ] Keep namespace-level failures from `list_spot_heads` and `list` as `CeremonyError::Internal`; they are not attributable to one row and may mean the inventory is incomplete globally.
- [ ] Extend the focused test with a missing blob head and a head whose valid artifact subject does not match its stored subject key. Assert both are omitted while the healthy row remains available. Rerun the focused test; expect success.
- [ ] Run `cargo test -p tonk-account-service --features helpers`; expect all core and HTTP service tests to pass.
- [ ] Run `cargo check -p tonk-account-service --target wasm32-unknown-unknown`; expect the Worker build, including `worker::console_error!` logging through `log_detail`, to compile.

### Task 2: Roll back every pull that fails before registry persistence

**Files:**
- Modify: `rust/tonk-cli/src/account_spots.rs:pull and private tests`
- Test: `rust/tonk-cli/src/account_spots.rs:tests`
- Test: `rust/tonk-cli/tests/account_spots.rs:pull_retains_an_unbound_canonical_spot_when_initial_sync_is_offline`

**Interfaces:**
- Consumes: an already-validated unused canonical target, `site::mount_delegated_at`, `remote::add_with_revocation`, `remote::set_upstream`, and atomic `spot::register_existing_unbound`.
- Produces: a private fresh-target guard with `new(path)` and `commit()` behavior; `pull` retains its existing public signature and `PullOutcome`.

- [ ] Add a private test `fresh_pull_target_removes_partial_state_unless_committed`. In a temp directory, create the guarded path after constructing the guard and assert dropping an uncommitted guard recursively removes it; repeat after `commit()` and assert the path remains. Run `cargo test -p tonk-cli account_spots::tests::fresh_pull_target_removes_partial_state_unless_committed`; expect a compile failure because the guard does not exist.
- [ ] Implement a private `FreshPullTarget` guard owned by `pull`. Construct it only after the existing `target.exists()` rejection, so it can prove the command owns any later directory. On `Drop`, remove the target recursively when it exists and the guard is uncommitted. If removal fails, print `warning: failed to clean up incomplete account spot at <path>: <error>` while leaving the original operation error unchanged. `commit()` only marks the guard as retained; it performs no I/O.
- [ ] Create the guard immediately before `mount_delegated_at`, keeping it alive across mount, remote creation, upstream setup, target canonicalization, and `register_existing_unbound`. Every `?` in those stages must therefore remove partial profile, repository, remote, and sync metadata written below the fresh target.
- [ ] Canonicalize the successfully mounted target before registration, register it atomically, and then immediately commit the guard. Do not perform another fallible local setup step between registry persistence and `commit()`.
- [ ] Move the initial `sync::pull` after successful registration and guard commit. Preserve its existing warning text and successful `PullOutcome`: an offline initial sync must leave the canonical directory and unbound registry entry available to `tonk pull` and to a second `tonk account spots pull` call.
- [ ] Rerun the focused guard test and `cargo test -p tonk-cli --features integration-tests --test account_spots pull_retains_an_unbound_canonical_spot_when_initial_sync_is_offline`; expect both to pass and the existing integration test to prove the post-registration retention boundary.
- [ ] Run `cargo test -p tonk-cli --features integration-tests --test account_spots`; expect all account spot pull/list/backup integration tests to pass.

### Task 3: Make local aliases deterministic across inventory, pull, and backup

**Files:**
- Modify: `rust/tonk-cli/src/account_spots.rs:local_subjects, back_up_current, back_up_registered, private helpers/tests`
- Test: `rust/tonk-cli/tests/account_spots.rs`

**Interfaces:**
- Consumes: lexicographically ordered `Registry::spots: BTreeMap<String, SpotEntry>`, opened repository subject DIDs, canonical site paths, and existing backup marker semantics.
- Produces: one selected `LocalSpot` and at most one backup attempt per repository subject during a sweep; no registry schema or public API changes.

- [ ] Add `list_and_pull_choose_the_first_alias_for_a_local_subject`. Create one site, then directly save two `SpotEntry` values named `alpha` and `zeta` that both reference its canonical path (matching the state produced by repeated `spot new --site` adoption). Back up that repository subject and assert list reports `local_name == Some("alpha")`; pulling the same subject must return `already_local == true`, `name == "alpha"`, and the existing site without mounting a second copy. Run `cargo test -p tonk-cli --features integration-tests --test account_spots list_and_pull_choose_the_first_alias_for_a_local_subject`; expect the current `local_subjects` corruption error.
- [ ] Change `local_subjects` to retain the first successful registry entry for each subject rather than replacing it and bailing. Since the source is a `BTreeMap`, use entry insertion without overwrite so selection is lexicographically stable. Print one warning for each ignored later alias naming both aliases and the shared subject; listing and pull must still succeed.
- [ ] Extract a pure helper used by `back_up_current` that chooses the first registry name whose canonicalized site path equals the already-open site's root. Add `current_site_aliases_choose_the_first_registry_name` with two ordered aliases for one canonical path. Run `cargo test -p tonk-cli account_spots::tests::current_site_aliases_choose_the_first_registry_name`; expect the current duplicate-name rejection, then replace that rejection with the helper's first-name result.
- [ ] Add `backup_sweep_uses_the_first_alias_once`. Directly save `alpha` and `zeta` registry entries pointing at one unnamed repository with an upstream, run `back_up_registered`, and assert the semantic account inventory contains one row whose fallback `remote_name` is `alpha`; run a second sweep, assert it is warning-free, and assert inventory still names the row `alpha`. Run `cargo test -p tonk-cli --features integration-tests --test account_spots backup_sweep_uses_the_first_alias_once`; expect the current sweep to process both aliases and leave `zeta` as the named head.
- [ ] Update `back_up_registered` to iterate names in registry order, open each entry, and maintain a set of successfully inspected repository subject DIDs. Insert the subject before attempting backup and skip later aliases, so one failed upload does not trigger duplicate attempts under alternate names. If an earlier alias cannot be opened, retain its existing per-name warning and allow a later usable alias to represent the subject because the failed entry's DID is unknown.
- [ ] Preserve `repository_name(site)` precedence: deterministic alias selection changes only fallback metadata when synced content has no `RepositoryName`. Do not rewrite `spots.json`, delete aliases, or add uniqueness checks to `spot::create` or `register_existing_unbound`.
- [ ] Run the three focused alias tests, then `cargo test -p tonk-cli --features integration-tests --test account_spots`; expect all account spots integration tests to pass.
- [ ] Run `cargo test -p tonk-cli`; expect the broader native CLI suite to pass.

### Task 4: Verify the combined review fixes

**Files:**
- Modify: none unless a check exposes a defect in the files above.

**Interfaces:**
- Consumes: completed resilient inventory, pull cleanup, and alias handling.
- Produces: fresh merge evidence for PR #674.

- [ ] Run `cargo fmt --all -- --check`; expect no output.
- [ ] Run `git diff --check`; expect no whitespace errors.
- [ ] Run `cargo test -p tonk-account-service --features helpers`; expect all account-service tests to pass.
- [ ] Run `cargo check -p tonk-account-service --target wasm32-unknown-unknown`; expect success.
- [ ] Run `cargo test -p tonk-cli`; expect all native CLI tests to pass.
- [ ] Run `cargo clippy -p tonk-account-service -p tonk-cli --all-targets --no-deps -- -D warnings`; if target-specific account-service code requires a separate invocation, run its wasm target separately. Report unrelated pre-existing lint failures without changing unrelated files.
- [ ] Inspect `git diff -- rust/tonk-account-service/src/core/backup.rs rust/tonk-cli/src/account_spots.rs rust/tonk-cli/tests/account_spots.rs`; confirm no wire DTO, route, authorization command, storage key, registry schema, or successful pull output changed.
- [ ] Manually check the three recovery boundaries: an omitted bad head does not hide a healthy subject, a pre-registration pull error leaves no canonical directory, and an offline initial sync leaves a registered directory that a retry recognizes as already local.
