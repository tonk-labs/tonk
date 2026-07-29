# Implementation notes — session delegations and the revoke ceremony

Running log of deviations from the two plans
(`2026-07-24-session-delegations.md`, `2026-07-24-signed-revocation-artifacts.md`)
and the edge cases behind them.

## Session delegations Task 3 was written against a client seam that does not exist

The plan says to call `extend_with_session` "where the client currently
loads its `root → device` grant for sync". There is no such place. The
presign invocation is signed by dialog's **operator** key, and its proof
chain is assembled by a `CertificateStore::prove` BFS from the operator
toward the subject — the client never hands a grant to a signing call.

The `device → session` hop the plan wants already exists structurally:
it is `profile → operator`, minted by `OperatorBuilder::build` when the
caller passes `.allow(...)`, and unexpiring. Bounding *that* is Task 3.

Two upstream facts shaped how:

- `prove` filters candidates with `range.covers(requested)`, and the
  requested range is always `TimeRange::unbounded()` (`UcanFork::authorize`
  builds `Authorize::new`, never `.during`). `covers` is vacuously true
  when both required bounds are `None`, so **an expired certificate is
  still selected**. Nothing in the walk consults the clock.
- Certificates are keyed by content hash (`{audience}/{subject}/{issuer}.{hash}`)
  and the store has no delete.

Together those mean re-minting a delegation under the same audience
leaves an expired certificate beside the fresh one, identical in
issuer/audience/subject, and `prove` picks between them in hash order.
So **renewal has to rotate the operator key**, giving each session its
own audience prefix. Dead certificates under retired audiences are never
selected because nothing proves *to* those audiences again.

The seam that makes rotation possible: `derive_operator` keys off the
caller-supplied context (`profile.derive(b"worker")`), so a random
context yields a fresh operator key per session.

## Sign-out revokes, so it must also rotate the profile

`mint_self_revocation` revokes the device's `root → device` grant, and
the access-service screen matches revoked rows on `device_did` **unscoped
by account** (`revoked_query`). Every presign chain this browser produces
carries the profile DID as a delegation issuer, so a self-revoke without
rotation permanently refuses presigns for that profile — local spaces
included — with no un-revoke anywhere.

Rotation is safe because nothing outside the device is keyed to the
device DID: `member_did` returns the account root when linked, and
`restore_spaces` re-mounts every escrowed `space → root` chain against a
fresh `root → newdevice` link.

Known gap, not closed here: `back_up_owned_space` is hooked only into
`enable_sync_inner`, so a space that was never sync-enabled is never
escrowed and does not survive rotation.

## Renewal needed an upstream change, so the pin moved to a rev

Rotating the operator means building a replacement, and
`OperatorBuilder::build` consumes the `Storage` it is handed. `Storage`
was not `Clone`, so the replacement could only get a fresh
`Storage::new()` — a second pool of connections over the same databases,
while every repository and branch handle the reactor cached still talked
to the first. Divergence with no error surface, and dropping the reactor
cache to compensate would have meant hand-migrating live subscriptions.

The fix upstream is small because the sharing is already there:
`Router` was `Clone` and holds an `Arc<Pool>`; only `Loader.mounts` was
unshared. dialog-db `feat/storage-clone` (#407) wraps it in an `Arc` and
derives the two impls, cut from the `tonk-2026-07-17` tag rather than
`main` so it carries none of the reserved-namespace or version-control
work that tonk #635 exists to absorb. tonk pins that rev until #407
lands and the pin can move to a tag.

## Two things only the wasm leg catches

Both were found by running the wasm suites locally rather than waiting
on CI (Chrome 150 at the default path plus nixpkgs chromedriver 150 —
`CHROMEDRIVER=… cargo test -p tonk-worker --target wasm32-unknown-unknown`).

- `it_refuses_to_revoke_without_a_signed_revocation` had never passed.
  It asserted a `Conflict` for an empty revocation, but the check sat
  behind `linked_service`, whose service lookup casts the global to a
  `ServiceWorkerGlobalScope` and fails outside a real worker — so the
  call returned `NotFound` and the assertion never got what it tested.
- `bootstrap_profile` seeds the standard library over a service-worker
  fetch, so calling it from sign-out made sign-out fail in any harness.
  It is now logged rather than fatal, which is also the right posture:
  by then the device is revoked and the key rotated, so there is nothing
  left to retry.

`Directory::Temp` is a stable path, so a per-run counter is unique only
*within* a run — the `device` tests name their scratch registries
randomly, or the next run inherits the last run's rotated pointer and a
profile that never rotated reads as one that did. Same trap
`router.rs`'s `session_nonce` documents.

## Identity and sharing E2E hardening

The accepted dialog prerequisite was already present in the worktree at
`25fac91eaa4d23fc220721df19f9e2593be618d7`. All workspace `dialog-*`
dependencies and `Cargo.lock` use that one revision; no mixed pin was
introduced.

The Darwin fix is the narrow top-level `remarshal` overlay described by the
plan. `nix-store -qR` resolves it to `python3.13-remarshal-1.3.0`; unrelated
Python packages remain unchanged.

Two defects were visible only in the aggregate Web gate:

- `tonk-fab` used `HtmlInputElement` for targeted invitations without enabling
  its `web-sys` feature.
- a `tonk-identity` revocation test passed `std::time::SystemTime` to dialog's
  Wasm `web_time::SystemTime`, and `tonk-account-service` used
  `#[dialog_common::test]` without its test-only `wasm-bindgen-test`
  dependency.

The targeted invitation copy cannot call `clipboard.writeText` after awaiting
the mint response because transient user activation has expired. It now shares
the existing promise-backed `ClipboardItem` seam: the form submit opens the
write synchronously and resolves it with the returned URL.

Native HTTP and credential tests cannot bind loopback ports or save temporary
signing sessions in the managed sandbox. The same binaries pass outside the
sandbox. No product error was hidden by those permission failures.

`path:.` is required while the new modules are untracked, but it also snapshots
the ignored 32 GB Cargo `target/` directory. The gates temporarily moved that
regenerable cache outside the source and restored it on exit. The
`test:native:debug` shell wrapper then invoked `git+file:.` internally and
omitted the untracked modules, so the equivalent `tests-native-debug` and
`tests-web-debug` derivations were built directly from `path:.`.

No account/access/UI deployment or staging mutation was performed. The session
had no staging ownership or deployment provenance, and the smoke requires
disposable account data plus controlled D1/R2 state. The implementation and
local/Nix gates are complete; the serial deployment and non-destructive staging
smoke remain an operator-owned release step.

Root-first account tests must derive provider ceremony metadata from the root
already persisted in the test profile. Supplying a second deterministic root is
not a harmless fixture shortcut: the account boundary correctly rejects it as a
ceremony/root mismatch. Invitation attribution likewise asserts the persisted
root entity, not the profile device DID.

The hardened sync contract distinguishes orchestration from an attempted
operation. A background sweep filters out branches with no upstream and
resolves as a no-op; a direct `POST .../sync` for that same branch is an
operational failure and returns non-2xx `SYNC_UNAVAILABLE`. Tests now pin both
sides so a manual request cannot claim reconciliation merely because there was
nothing configured to reconcile.

The crate-wide Wasm test harness runs in a browser document even when one module
requests a service-worker harness. The outbound HTTP helper therefore calls the
same `globalThis.fetch`, `setTimeout`, and `clearTimeout` functions dynamically
on both targets. Production still supplies the service-worker global; the
browser test can replace those functions to verify bytes, media type, structured
status, and abort timeout without maintaining a second transport.

The staging pre-deploy audit found that `delegation_hex` had been added to the
already-applied `0001_init.sql`, so Wrangler reported no pending migration while
the live `devices` table still lacked the column. The signed path cannot be
reconstructed from its CID or from account space backups. Migration 0003
therefore adds a nullable column: new registrations always populate it, while
legacy rows expose absent evidence explicitly and disable only cross-device
revocation. Self-revocation remains possible from the device's local grant.
