# Disposable worker operators with bounded in-memory grants

Every worker boot should create an operator with a matching, expiring profile-to-operator grant held in memory. Membership and invitation authority remain attached to accounts and profiles. Replacing a worker therefore changes only the final session hop and requires no invitation replay. This fixes Safari sync after worker replacement while avoiding a durable delegation commit on every boot. The tradeoff is one fresh session-grant signature per boot; Safari latency must be measured rather than assumed negligible.

The evidence and reproduction are in [mobile-sync-diagnosis.md](mobile-sync-diagnosis.md). Safari returned different Ed25519 signatures for identical inputs. Dialog's non-extractable-key derivation hashes such a signature, so reconstruction changed the operator DID while `session::open` reused a persisted grant addressed to the previous operator. The phone's delegation metadata confirmed that mismatch. Current `prepare_join` claims invitations for `current_account`, including onboarding accounts. `Invite::visit` has only test callers in this repository; the session module's guest-replay commentary describes an obsolete flow.

Scope is the browser worker and the narrow Dialog API it needs. Keep the existing 12-hour session TTL, one-hour renewal margin, device/profile and account identities, invitation proofs, revocation behavior, non-extractable browser keys, and local/offline space contents. CLI session persistence is a separate follow-up. No pruning system or stable operator identity across boots is required. Expiration bounds authorization; dropping a worker releases its operator and in-memory grant without a cleanup transaction.

## 1. Dialog can build a bounded in-memory session

- [x] A bounded operator proves a retained space grant with the correct expiry, without writing session facts or changing the access-branch revision.

This prerequisite belongs in Dialog, currently pinned by Tonk's `Cargo.toml` and `Cargo.lock` to `tonk-2026-09-02`, commit `805151c678c516edc03c2825171ed4b54adf37ab`. Work in an isolated Dialog checkout and read its repository instructions. Do not edit Cargo's dependency cache. The relevant files are `rust/dialog-operator/src/operator/builder.rs` and `rust/dialog-operator/src/operator/access.rs`.

Add `OperatorBuilder::allow_until<T, C>(self, capability: C, expiration: Timestamp) -> Self`, with the same capability bounds as `allow`. Internally retain each allowed scope with its optional expiration. Preserve `allow`'s existing unbounded behavior for existing callers. During `build`, apply the expiration to the `DelegationBuilder` before signing the profile-to-operator certificate. Install that certificate in the existing in-memory session collection before installing `WalkReach`; its recursion-bounding operator clone must carry the same grant. No `Retain` operation is needed for session authority.

Start with behavioral tests beside the existing session-composition and no-session-residue tests. Retain a space-to-profile grant, snapshot the access-branch revision, construct a bounded operator, and prove a concrete storage capability as that operator. Check the final audience, chain continuity, and effective expiration (the earlier of the upstream grant and session bounds). Check that the branch revision and stored delegation count remain unchanged. Exercise proof-cache hits as well as a fresh walk, and requests with explicit time ranges within and beyond the session window. A request for a current invocation must not gain an unbounded or already-expired session proof. Use explicit timestamps/windows rather than sleeps; distinguish proof construction for a historical window from authorization of a current invocation. With the existing builder the bounded API is absent and `allow` yields an unbounded proof, which is the initial failing boundary.

Run `cargo test -p dialog-operator --locked` in the Dialog checkout, plus its focused WASM session tests using its configured runner. Keep the change additive and independently reviewable. Pin Tonk's Dialog dependency family to the resulting immutable revision, updating only associated lock entries and required dependency hashes in `nix/rust.nix`. `flake.nix` also has a separate Dialog source input used by `nix/wbg-pool.nix`; do not conflate a test-runner update with the Rust-library pin. Any temporary local dependency override must be removed before handoff.

## 2. Worker boot and renewal create matching disposable sessions

- [x] Opening or renewing a worker session changes its operator identity, preserves joined-space access, and leaves profile session storage unchanged.

In `rust/tonk-worker/src/session.rs`, make `open` create a fresh session on every call. Use a new random 32-byte derivation context, propagate entropy failures, calculate the existing TTL expiration once, and build with `allow_until(Subject::any(), expiration)`. A random context makes disposability explicit on native and Chromium too; the code no longer relies on reconstructing signature-derived identities. Keep building over the caller's cloned storage pool so existing repository handles retain their storage configuration. `Session` still returns the operator and the exact expiration used for its grant.

Remove the persisted-session reuse path, `PersistedSession`, `SESSION_SITE`, load/save helpers, and the explicit durable `.access().save(...)` session grant. `rotate` can own the common fresh-session construction and `open` call it. Leave old `tonk-session-v1` credential data and historical session delegations untouched: new code does not consult them, and fresh random contexts avoid deliberately reusing their audiences. This is also recovery for affected Safari installations—next boot uses the existing profile and invitation authority to authorize its newly created operator. No storage clearing or rejoin is part of migration.

Update `ensure_session_authority` in `rust/tonk-worker/src/router/sync.rs` to describe the actual lifecycle. Preserve construction outside the state write lock, the post-construction comparison against the observed `session_expires_at`, and atomic replacement of operator plus expiry. Concurrent callers may construct candidates; only a candidate for the still-current expired generation is installed. Losing candidates have no durable session side effects. A construction error retains the current state and surfaces through the existing error path. Requests already holding the state read lock finish before replacement. Keep renewal timing and revocation policy unchanged.

Replace stale stable-DID/guest-replay comments in both files. Inspect `worker::boot_state`'s session-open error recovery under the new path and retain any recovery still needed for opening profile storage; do not turn an unrelated read failure into a reason to discard profile data.

First extend `session` tests to open twice and assert distinct operators, unchanged profile identity, valid bounded proofs through both, and unchanged profile-main revision after each session construction. These assertions fail against current code: native operators reuse the fixed-context identity and fresh session creation persists a grant. Add an upgrade fixture containing a fresh legacy session record and a grant to a different operator; opening must ignore that record and prove through the new in-memory grant without writing replacement session metadata. Add a storage-close/reopen fixture, releasing handles before reopening, rather than testing only a second operator over the same live pool.

Replace `renewal_tests::it_keeps_the_operator_did_across_renewal`, which currently does not force renewal, with a test that actually marks the session due. Assert a changed DID, a fresh bounded proof, no profile-main commit, and no further replacement when another renewal check runs before the margin. Exercise concurrent due checks and failed candidate construction with focused test hooks if needed. Do not weaken grant expiry or add production timing knobs for tests.

Run `cargo test -p tonk-worker --lib --locked session::tests` while iterating. At this checkpoint also run `cargo test -p tonk-worker --lib --locked router::join::tests` and the WASM renewal tests. Keep the dependency integration and worker adaptation reviewable as one working Tonk increment after the Dialog prerequisite.

## 3. Prove joined-space recovery across actual worker lifetimes

- [ ] A durable onboarding member syncs after worker replacement on macOS Safari and iOS Safari, without rejoining or accumulating session grants.

Add regression coverage alongside `rust/tonk-worker/src/router/join.rs` tests: join a disposable fixture space through an onboarding account, prove a real storage capability with the resulting operator, rebuild the worker from the same durable profile/storage, then prove and sync again. Assert the account/member/profile identities and membership count survive while the operator changes. Compare session delegation records and profile-main revisions around session construction only; joining and genuine content work legitimately create commits. Include a profile with existing invitation grants and no completed email setup, matching the reported case.

Use the repository's WASM runner for automated browser coverage. Focused commands are:

```sh
cargo test -p tonk-worker --lib --locked --target wasm32-unknown-unknown session::tests
cargo test -p tonk-worker --lib --locked --target wasm32-unknown-unknown router::sync::renewal_tests
cargo test -p tonk-worker --lib --locked --target wasm32-unknown-unknown router::join::tests
```

The configured runner is `wbg-pool`; ordinary Chromium coverage does not establish Safari coverage. On the corrected build, use a disposable invite in macOS Safari: join and sync, enter Settings without submitting email, return through Spaces, then explicitly unregister only that origin's service worker and reload. Preserve IndexedDB and all space data. Confirm the profile remains the same, the new operator differs, and a remote change made by another member is pulled; also push a small change back. Repeat worker replacement several times and confirm no new profile-to-operator delegation rows accumulate. Repeat on iOS Safari using worker replacement where available and the original navigation sequence. Neither metadata HTTP 200 nor a changed indicator alone proves synchronization.

Check an offline worker restart can open already-local space content without an account-service request. Restore connectivity and confirm sync resumes with the fresh session. This tests offline session construction, not complete offline replication of untouched space content.

Measure repeated session construction on macOS Safari and Chromium, recording operator-build and bounded-grant signing time separately from total worker boot and first successful pull. Use a disposable profile; report sample count and median/tail measurements. Confirm no durable session-retention work occurs. The measurement answers the remaining performance question; there is no invented millisecond acceptance threshold.

After the final implementation, run `cargo fmt --all -- --check`, the native worker library suite (`cargo test -p tonk-worker --lib --locked`), and `git diff --check`. Run the relevant repository Nix dependency/build check after the Dialog pin; format any changed Nix file with the repository formatter. Remove temporary instrumentation and dependency overrides. Report native, Chromium, macOS Safari, iOS Safari, and deployed-build evidence separately, including any unavailable checks.

## Planning evidence

This document describes work to implement, not completed validation. The earlier temporary native proof-reopen test passed over a shared storage pool; it did not cover disposable in-memory grants or Safari worker replacement. The Safari signature nondeterminism and mismatched saved/current operator identities were observed on the affected device. At that planning checkpoint, only planning/diagnosis Markdown files had been added in this worktree.

## Implementation progress (2026-09-09)

The implementation is built and tested against the isolated Dialog prerequisite.
The Dialog prerequisite is published, and native, Chromium, and Nix validation
against the published immutable pin passed. Device acceptance remains pending.

Dialog commit `314e49926eb5bca58bacdcf9f2123ccb4422ea35` is preserved in
`/Users/jackdouglas/tonk/dialog-db/.wt/fix/bounded-worker-sessions` on branch
`fix/bounded-worker-sessions`. It starts at Tonk's exact previous pin and leaves
the supplied checkout's unrelated work untouched. `allow_until` applies an
optional expiration before signing the in-memory grant. Session selection also
checks the requested time window so a lapsed overlapping grant cannot mask a
valid one. The new bounded-session test first failed at the missing API.

Worker boot and renewal use random 32-byte contexts and bounded in-memory grants.
Legacy session metadata and grants remain untouched. Tests cover distinct opens,
proofs through both operators, access-branch revision stability, storage close
and reopen with a mismatched legacy grant, forced renewal, failed construction,
and a deliberately losing concurrent candidate. An onboarding membership test
boots both lifetimes through `boot_state`, releases the old state/storage, and
checks profile/account/member identity, local content, and storage proof expiry.
Its fixture has no remote and cannot establish network replication.

Boot's old hydration fallback was removed: the new builder resolves reference
cells without walking or retaining content. Hydration cannot repair its entropy,
signing, or local reference failure modes; these now surface without touching
profile contents.

Validation:

- Dialog native: 33 passed with `cargo test -p dialog-operator --locked`.
- Dialog Chromium: four focused session tests passed with the configured
  `wbg-pool` runner.
- Worker native: 121 passed both with the initial local Dialog override and
  against the published immutable pin using `--locked`. The
  sandboxed run initially had 21 filesystem/loopback permission failures; the
  unchanged suite passed with fixture access.
- Worker Chromium: 11 tests matched `session::tests` (eight signing-session tests
  plus three router-session tests); three renewal tests and 26 join tests passed.
  All 40 tests passed again against the published pin without overrides.
- The join restart test initially compared a minimal unbootstrapped fixture with
  a real boot; fixing both lifetimes to run real boot eliminated that fixture
  mismatch without changing product behavior.
- `nix build .#checks.aarch64-darwin.sharedWorkspaceDeps --no-link` passed,
  including the published Dialog source/vendor and workspace dependency build.
- Rust/Nix formatting, Nix source-reference, and diff whitespace checks passed.
- macOS Safari, iOS Safari, actual remote pull/push after worker replacement,
  network-offline restart, and signing/boot/pull latency measurements are unrun.
  Chromium unit tests are not evidence of those device or hosted behaviors.

The user approved publication on 2026-09-09. Dialog commit
`314e49926eb5bca58bacdcf9f2123ccb4422ea35` was pushed to
`https://github.com/dialog-db/dialog-db.git`, branch `fix/bounded-worker-sessions`.
A fresh `git ls-remote` verified the exact remote SHA. No PR was merged.

The prepared dependency update changes all 16 workspace Dialog declarations and
21 lockfile source entries to that immutable revision, without changing the
separate `flake.nix` runner input. The source archive's locally computed NAR hash
is `sha256-V3ZKANhlwG2MLTW6A4c/GS76k1bVB0OusFLGrNj4t9Q=`. The manifest, lockfile,
and Nix hash edits are applied locally. Cargo fetched the published pin, and locked native and Chromium tests passed
without local overrides. The Nix `sharedWorkspaceDeps` check also passed. The dependency integration,
worker adaptation, and task records form one local Tonk increment.

The temporary Cargo path override was removed after initial validation. Ordinary
locked builds now resolve the published immutable Dialog commit.
