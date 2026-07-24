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
