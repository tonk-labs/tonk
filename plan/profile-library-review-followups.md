# Profile-library review verification and repair plan

Date: 2026-09-15. Source reviewed: `2d8ff66c84ce2ca7c88b45e9c0b3ad18726714a3`.

Status: implementation complete for desired-manifest validation, singleton repair, worker-scoped input caching, generation-pinned acquisition, development refresh, and a generation-A stale writer. Focused native, Wasm, and Chrome regressions pass. Independent upstream inspection, per-sweep download counting, Safari, deployed-host behavior, and production bandwidth remain unverified.

## Verified findings

### 1. P1: Installation history is not a complete desired manifest — accurate

`rust/tonk-worker/src/router/repository.rs:4958-4966` accepts the target installation when the assertions at its recorded version still stand. `assertions_at_version` at line 3530 reads only that version's assertion records and discards cardinality information (`unique: false`). It does not enumerate all desired definitions.

The evaluator queues prior retractions before evaluating the desired document (`router/evaluate.rs:530-542`). In the locked dialog dependency (`Cargo.lock:989`, commit `b553b466e24ac9f3c577026b3b7c10edc42f443d`), `rust/dialog-artifacts/src/artifacts/update.rs:161-165` replaces the entire queued change list for an entity/attribute with `Replace`. `rust/dialog-artifacts/src/tree.rs:1472-1479` skips a replacement when the identical value already stands without competitors, recording no history. Thus unchanged cardinality-one definitions survive an upgrade but are absent from its installation delta. A later mutation of one of those definitions can pass validation and receive a cached revision.

History remains useful for attributable cleanup; it must not double as the desired-state specification. A vacuously empty delta also cannot establish library correctness.

### 2. P2: Competing values are accepted — accurate defect, inaccurate rendering explanation

`repository.rs:4843-4845` breaks on the desired value. A stale value at the same entity/attribute does not make the check fail, irrespective of whether it appears before or after the desired value. System-owned cardinality-one fields require the desired value to be the sole distinct standing value.

However, the stated list-to-fallback mechanism is not supported by this checkout. `rust/tonk-core/src/conclusion.rs:51-66` projects keyed entries as maps. `rust/tonk-template/src/fold.rs:89-94` merges these maps with `entries.extend`, so the last row for a duplicate facet key wins. `show_template` at line 31 then receives that string. Ordinary non-map fields do accumulate lists, but `show` is keyed. The supported consequence is unresolved competing templates and potentially stale rendering depending on row order, not a proven fallback. Keep the P2 fix; correct its rationale.

### 3. P2: Unchanged sweeps acquire the library before consulting the cache — accurate

`repository.rs:4743-4746` always acquires bytes before `reconcile_profile_library_from` checks the receipt at line 4954. `ProfileLibraryCache` at line 2974 stores only a digest/revision pair. Browser acquisition at lines 3670-3704 uses `RequestCache::NoStore` and the worker-global fetch. Both account paths await reconciliation before pushing (`router/account_state.rs:739-746` and `855-856`).

The foreground worker loop has a 2,000 ms tick (`worker.rs:2309`); its actual activity is gated by subscribers, connectivity, enabled sync, and scheduler state (`worker.rs:2214` onward). Each successful ready-account sweep that reaches reconciliation incurs acquisition, even if the branch is unchanged. This is a source-level finding; download counts, payload cost, and latency have not been measured in this review.

### 4. P2: The browser test does not establish a stale competing write — accurate

`rust/tonk-ui/src/service_worker_upgrade.rs:484-566` prepares two generations from the same built implementation, changing generation A's profile document. Promotion at line 567 switches the shared served tree to generation B. The old worker fetches the currently served library through its own global fetch; its generation-A build identifier does not pin that input. Its account sweep reconciles before pushing.

`account_flow.rs:3928-3935` calls `/api/sync` and labels it a stale publication without asserting stale claims locally or upstream. The call can repair the old device with B bytes before pushing. The test proves an upgrade scenario, but its later passing Hub assertions do not establish repair after a stale upstream publication. No browser run or network trace was performed for this review.

## Implementation sequence

Keep each increment independently reviewable. First demonstrate each failure with the focused test, then fix it and rerun that test. If commits are requested, commit each proven increment separately.

### 1. Validate complete desired definitions

- [x] Add a repository regression starting with a populated library A, then install B that changes one facet while retaining another cardinality-one field. Verify the retained field is absent from B's installation history; mutate that field while leaving B's installation record intact. Assert reconciliation repairs it and the next call does not advance the revision.
- [x] Derive a complete desired assertion set from the shipped document using the existing parser/evaluator and schema lowering. Preserve replacement/cardinality intent before storage deduplication. Prove the smallest extraction seam on an isolated empty transaction/repository first; do not derive desired assertions from an evaluation against already-populated state or from a committed delta. Include concepts, rules, routes, view facets, and event bindings, including generated identities.
- [x] Cache this prepared desired input by asset digest. Validate it against the live branch whenever the branch receipt changes. Keep installation history solely for attributable retractions and metadata retirement.
- [x] Preserve existing exact-source ownership, legacy deferred-cleanup reporting, unrelated account/authored facts, atomic publication, and refresh/replan on CAS retry. Do not broaden cleanup to namespaces or all account history.
- [ ] Store a checked revision only after complete validation or after proving the published result satisfies the same invariant. Malformed input and acquisition failure are covered and leave no successful receipt; a dedicated injected publication-failure receipt test is still absent.

Acceptance: damage to any retained shipped definition is detected even when the latest install did not write it; intact state is a no-op. Include worker restart/empty-cache coverage so correctness does not depend on a previous in-memory receipt.

### 2. Enforce uniqueness for owned cardinality-one fields

- [ ] Add a fixture with both desired and stale values at a shipped facet, first through raw non-unique assertions, then through an actual divergent account merge. The raw fixture proves both insertion orders with target metadata retained; an account merge that proves both raw values stand simultaneously is still absent.
- [x] Group expected claims by entity/attribute. For owned cardinality-one fields, require exactly the desired distinct value and consume the full claim stream, propagating read errors. Preserve legitimate cardinality-many semantics rather than imposing singleton checks universally.
- [x] Reuse evaluator replacement to remove competing values at known owned fields. Confirm replacement clears competitors even when the desired value already stands. Preserve foreign facets and authored entities.
- [x] Query the resolved template and a subscribed view after repair; assert the exact desired template and singleton raw claim set. Do not use a fallback expectation based on the incorrect list explanation.

Acceptance: both stale-only and desired-plus-stale states repair, remain repaired on repeat, and leave unrelated content intact.

### 3. Cache acquired input within the worker asset lifecycle

- [x] Add an acquisition counter/failure seam around the production wrapper, not just tests of `reconcile_profile_library_from`. Demonstrate repeated unchanged calls currently reacquire bytes.
- [x] Add a worker-scoped acquired-input cache separate from the branch validation receipt. Reuse bytes and prepared definitions when only profile state changes; invalidate the receipt on profile/revision changes. Coalesce concurrent acquisition attempts without holding a synchronous mutex across await.
- [x] Tie input to the running asset generation. Inspect/reuse the generation-aware asset mechanisms in `rust/tonk-ui/assets/service_worker.js`; do not assume the current origin URL identifies the running generation. A new worker must use its own generation's input. If persistent generation-cache access is necessary for offline restart, cover that boundary explicitly.
- [x] Wire explicit development invalidation/update through `rust/tonk-ui/assets/hot-swap.js:480-526` and the worker. Today this path evaluates the new document into mounted profile contexts; a byte cache must not let the next sweep restore the old input. Prefer passing the newly acquired document/digest through a scoped development path. Ensure an older in-flight fetch cannot overwrite a newer invalidation result.
- [x] Failed acquisition must remain retryable and must not erase usable definitions or cache success. Verify cached-input sweeps do not wait on network; initial failures retain the existing push/error-combination behavior.

Acceptance: first acquisition once, repeated unchanged sweeps zero additional acquisitions/evaluations/writes; a profile-only mutation revalidates using existing bytes; development changes refresh input once; worker-generation changes use the correct document. Include failure/retry, concurrent requests, profile switching, and warm offline restart.

### 4. Establish real stale upstream publication in the browser test

- [x] Make the writer adversarial explicitly. The test pins the old writer to generation A's cached profile-library bytes, asserts their digest before login and after publication, and verifies generation-A rendering before the current writer repairs it.
- [ ] Control automatic sweeps so the current device cannot repair upstream before the assertion. The test observes the old writer's stale local rendering immediately after its successful push, but it does not inspect the remote through an independent non-reconciling client or assert the published revision directly.
- [ ] Verify the current device pulls that state, restores the exact current facet, publishes repair upstream, renders the Hub, and preserves account name, space entries, and authored sentinels. Current rendering, account name, and space preservation pass; the test does not yet count library commits or profile-library downloads across the final repeated sweeps.
- [x] Keep generation upgrade/offline tests separate from stale-writer injection so the asset-cache fix cannot silently turn the adversarial test into an ordinary upgrade test. Retain the limitation that a continuously active old writer can compete again.

## Validation commands and completion gates

### Fresh evidence from this implementation

- Before the desired-manifest fix, the retained-definition regression failed because reconciliation returned `Unchanged` after a shipped definition absent from the latest install delta was damaged.
- `cargo test -p tonk-worker profile_library -- --nocapture`: 10 unit tests passed; the filtered standard-library lowering test also passed.
- `cargo test -p tonk-worker --test standard_library it_builds_one_centered_hub_launcher_with_a_settings_route -- --exact`: passed.
- `cargo check -p tonk-worker --target wasm32-unknown-unknown --features helpers`: passed with six existing dead-code warnings.
- Focused Wasm profile-library selection: 8 tests passed.
- Broader Wasm profile-library, account-state, and profiles selection: 36 tests passed.
- Full native `tonk-worker` checkpoint: 208 tests passed after correcting the new route entry's lexical order. The initial run stopped at that route-table assertion after 139 passes.
- Service-worker lifecycle suite: 132 tests passed.
- The persisted-generation Chrome test passed. The mixed-writer Chrome test passed in 325.520 seconds after it was changed to wait for a complete generation-A cache and assert the exact cached profile-library digest before login, after login, and after the stale push.

The browser evidence proves that the old writer ran and rendered generation A after a successful account push, followed by repair and preserved account/space data on generation B. It does not independently read the remote head between those actions, count final-sweep profile-library requests, or prove a singleton conflict produced by an account merge.

Use focused commands while iterating:

```sh
cargo test -p tonk-worker profile_library -- --nocapture
nix develop -c test:web:debug -E 'package(tonk-worker) & test(profile_library)'
cargo test -p tonk-worker --test standard_library it_builds_one_centered_hub_launcher_with_a_settings_route -- --exact
nix develop -c test:e2e -E 'package(tonk-ui) & test(profile_library)'
```

At the combined integration checkpoint:

```sh
cargo fmt --all -- --check
nix develop -c test:native:debug -E 'package(tonk-worker)'
nix develop -c test:web:debug -E 'package(tonk-worker) & (test(profile_library) | test(account_state) | test(profiles))'
nix develop -c test:sw
```

Record fresh failing/passing evidence, request counts, branch revisions, exact facet values, and upstream state. Update the original plan's completion claims only after these gates pass. Safari, deployed hosts, and production bandwidth impact remain unverified unless separately exercised.
