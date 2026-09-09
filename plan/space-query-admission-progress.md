# Space query admission implementation

Implemented against `d4a7ad5eace82c89568eeb90855d246981bc652d`.
The admission plan and investigation were untracked at implementation start.

## Checkpoints

- Task 1 complete: strict meta-only configuration projection. Its content-read
  regression failed on the original broad information builder and passed after
  narrowing. Missing remotes, tracking mismatch, stale cached upstreams,
  first-use adoption and unchanged-meta idempotence are covered.
- Task 2 complete: worker-owned weak receipts, strict directory reads,
  cancellation-safe mutation generations, and the writer inventory below.
  Known local replicas remain usable after failed reconciliation without a
  receipt. Negative results are not cached.
- Task 3 complete: per-subject slow-path mutex and recheck. Controlled tests
  cover sixteen callers, cancellation, failed leaders, independent spaces,
  profile changes, eviction, and one mount plus one read-only verification on
  concurrent first use. Profile freshness starts before replica lookup; its
  controlled regression first failed with the later stamp and then passed.
- Task 4 local fixture complete; real Helium issue-tracker comparison pending
  a changed build in that environment. No deployment or user-worker stop was
  performed. Temporary phase headers/counters were removed from source, and
  both dedicated browser/server sessions were stopped.

## Final verification

- `nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker router::adopt::tests -- --nocapture`: 15 passed.
- `nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker`: 378 passed; integration binaries/doctests had no runnable tests.
- `nix develop -c cargo test --locked -p tonk-schema directory`: 2 passed.
- `nix develop -c cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

The first full run exposed a pre-existing fixed-identity collision: join's
invite-reopen test and a member-removal test both used seeds `(90, 91)` and
therefore shared subject-keyed storage. The test passed alone. Its fixture now
uses distinct `(218, 219)` seeds, and the complete suite passes.

The initial removal regression assumed immediate re-adoption after deleting an
owned space's storage; that path returned `Space already exists`, including
with original membership-before-open ordering restored. Coverage now asserts
reconciliation runs and the old receipt cannot be reused, without changing
removal/adoption policy.

Final local Chrome 152 measurements used a directory-configured loopback
fixture: width-16 space formula medians 3.3/3.4 ms, profile medians 2.0/3.8 ms,
zero additional warm directory/configuration reads, and identical results.
See [the investigation](space-reload-cache-investigation.md) for both artifacts,
phase timings, outliers and limits, and [raw samples](space-query-admission-browser.json).

## Configuration writer inventory

- `repository::record_replica_local_meta`: shared create/join/restore metadata
  writer; guarded before opening durable meta and setting direct upstreams.
- `repository::ensure_remote_config`: shared attachment/directory repair;
  guarded, preserves existing addresses, refreshes cached upstreams. A slow
  projection reconciles a stale cached meta revision before receipt publication.
- `join::mount_replica_with_configuration`: guarded across mount setup.
- `repository::remove_space_inner`, `bail_if_space_removed`: guarded profile
  retraction/eviction with existing outer state-lock ordering preserved.
- Generic `transact`, `evaluate`, `claim` writes and `transfer::import` use
  reactor sessions. Cached durable meta revisions invalidate these changes.
  The direct branch open in claim is a read-only artifact selector.
- Sync pulls/refreshes and evaluator conflict recovery refresh cached sessions;
  profile-main/meta revisions cover those changes. Content-only updates and
  UI/status overlays do not invalidate receipts.
- `account_state` writes account configuration directly; account repositories
  remain excluded before receipt lookup. Provider unlink retracts profile facts
  and invalidates account-key resolution without rewriting space remotes.
- Profile switching builds a fresh `TonkState` through worker startup; all
  constructors initialize a new admission cache alongside the new reactor.
- `create_invite` reads meta for remote execution. Its bare-operator remote
  address recovery does not rewrite the meta/upstream facts admission compares.
- Rotation's direct upstream write is a test fixture; production rotation
  changes content/account facts through reactor sessions.

No generic reactor policy, sync locking, schema storage, dependencies or
lockfiles changed.

## PR #917 lint follow-up

CI run `34260573705` rejected two dead-code warnings in native Clippy:
`reconcile_mounted_space_from_directory` and `Observations::counts` have
Wasm-only callers. Their compile guards now match those callers rather than
suppressing warnings. The remotely rebased admission patch at `950290935`
was verified equivalent before applying this fix.

- `nix develop -c cargo clippy --locked -p tonk-worker --all-targets --all-features -- -D warnings`: passed natively.
- `nix develop -c cargo test --locked --target wasm32-unknown-unknown -p tonk-worker router::adopt::tests`: 15 passed.
- `nix develop -c cargo fmt --all -- --check` and `git diff --check`: passed.
- The complete Linux Nix lint job awaits CI on the follow-up commit.
