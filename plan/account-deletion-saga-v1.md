# Replay-safe whole-account deletion implementation plan

**Goal:** Re-enable whole-account deletion only after a passkey-root-authorized,
restart-safe saga can prove what was approved, deny access before purging hosted
state, finish or resume every remote effect idempotently, and retain an
acknowledgement long enough for a lost response to be recovered.

**Current safety state:** PR #834 routes `POST /api/account/delete` to a fixed
503 response before extractors, profile state, or remote effects run. Keep that
refusal authoritative through protocol and worker deployment. The final UI
enablement is the last slice, not part of the protocol foundations.

## Non-negotiable invariants

- A device delegation is not deletion authority. The terminal authorization is
  a fresh invocation signed by the passkey-derived account root after the user
  reviews the exact plan.
- The invocation is a one-operation capability, not an open-ended delegation:
  issuer and subject are the account root; audience is the exact worker device;
  command is `account/deletion/authorize/v1`; arguments bind the operation ID,
  challenge nonce, plan hash, provider identities, source profile DID, source
  profile generation, and every sorted deletion subject.
- The worker accepts an authorization only for its current active root, device,
  profile, generation, provider configuration, unexpired challenge, and exact
  freshly reconstructed plan. No request field can redirect deletion to
  another root, account row, profile, or provider.
- Access denial is committed before any hosted object is purged. Once the local
  marker reaches `RemoteMayHaveStarted`, boot must never open the source profile
  as ordinary active state again.
- Every provider transition is idempotent by operation ID and authorization
  digest. Conflicting replay is a hard error; identical replay returns the
  stored state or receipt.
- Account-service commit deletes the account ID captured during reserve. It
  must not re-resolve an email or root after access-service work has begun.
- The account row is removed only after the account service verifies the exact
  signed access-service completion receipt. Provider tombstones and operation
  receipts outlive the account row so a lost final response remains queryable.
- The worker persists the next saga phase before initiating a remote effect
  whose response may be lost. A response receipt is persisted before any
  reload, profile rotation, or acknowledgement navigation.
- Corrupt, unsupported, or contradictory saga state fails closed. Recovery may
  repair a landing profile or repeat an idempotent provider call; it may not
  infer that an uncertain destructive call did not happen.
- No test or recovery path clears browser storage, unregisters a service
  worker, deletes passkeys, or opens a real user profile.

## Shared protocol types

Place provider-neutral domain structures and canonical hashes in
`rust/tonk-account/src/deletion.rs` so neither service depends on the worker.
Place UCAN verification and signed receipt helpers in
`rust/tonk-identity/src/deletion.rs`.

### `DeletionPlanV1`

Encode the plan as canonical DAG-CBOR and hash the encoded bytes with a
domain-separated BLAKE3 digest. The plan contains:

- version and operation ID;
- account root DID and confirmed provider email;
- exact account-service and access-service identities;
- source profile DID and worker generation;
- a sorted, duplicate-free list of owned hosted-space subjects;
- the account-owned custody subject, always ordered last;
- the account record as the terminal provider target.

Reject unsorted subjects, duplicates, unexpected providers, guest/shared
spaces, missing custody, unknown versions, and any decoded value whose
re-encoding is not canonical. Pin one cross-crate byte/hash test vector.

### Root authorization

The UI obtains a challenge from the worker only after rendering the plan. The
custody relay derives the root through a fresh WebAuthn assertion and returns
only the signed authorization bytes. It never returns, stores, or logs the root
signer, PRF output, KEK, or account secret.

The worker verifies:

1. exactly one invocation, valid signature, root issuer equals root subject;
2. exact worker-device audience and exact command;
3. exact operation ID, nonce, plan hash, source profile DID/generation,
   providers, and subjects;
4. current time inside the challenge window;
5. the challenge row is pending and bound to the same operation;
6. a freshly rebuilt canonical plan has the same bytes and hash.

The authorization digest is the canonical invocation CID. Persist it with the
operation and require it on every later reserve/status/commit call.

## Provider protocol

### Account service migration `0009_deletion_operations.sql`

Add an append-preserving operation/tombstone table with unique operation ID and
authorization digest. Store captured account ID, root DID, normalized email,
plan hash, access provider, state, access receipt bytes/digest, created/updated
times, and terminal receipt. Do not cascade this table when the account row is
deleted.

Commands:

- `account/deletion/reserve/v1`: verify the root authorization, resolve and
  capture the active account row once, and return the same reservation for an
  identical replay. No account/device row changes yet.
- `account/deletion/status/v1`: proof-bound lookup by operation and digest;
  return `Absent`, `Reserved`, `AccessCompleted`, or `Completed` without
  accepting an arbitrary root/email lookup.
- `account/deletion/commit/v1`: require the stored reservation and a valid
  access-service receipt for the same operation/root/plan; atomically delete
  devices and the captured account ID, then retain a terminal receipt.

The legacy `/account/delete` endpoint remains a stable refusal throughout the
rollout. New commands use new routes or exact command dispatch so an older
worker cannot accidentally invoke them.

### Access service migration `0006_account_deletion_operations.sql`

Add an operation table keyed by operation ID/digest with plan hash, root,
subject bitmap, denial/commit state, and signed terminal receipt. Retain the
existing per-consumer `active -> deleting -> deleted` projection.

Commands:

- `customer/deletion/reserve/v1`: verify the root authorization and account
  reservation facts, atomically move every planned consumer to denial state,
  and return the same bitmap for an identical replay. Any subject/provider
  mismatch refuses before the first transition.
- `customer/deletion/status/v1`: return the exact bitmap and stored receipt for
  this operation/digest.
- `customer/deletion/commit/v1`: idempotently purge planned hosted state, with
  the account custody subject last; mark each bitmap entry only after its purge
  succeeds; finish consumer deletion; and sign a receipt over operation ID,
  root, plan hash, authorization digest, and completed bitmap.

The access service is denial-first: after reserve, normal read/write/provision
routes for a deleting consumer refuse. Rollback never changes `deleting` back
to `active`; recovery completes the operation.

## Worker saga

Persist a versioned, secret-free marker in the device registry rather than the
source profile. The marker must be readable before `Registry::open_active` and
updated with compare-and-swap semantics under a device-wide deletion lock.

Phases:

1. `Authorized`
2. `AccountReserved`
3. `AccessReserved`
4. `LandingCreated`
5. `LandingPointerCommitted`
6. `LocalFinalizing { completed_bitmap }`
7. `LocalFinalized`
8. `AccessCommitPrepared`
9. `RemoteMayHaveStarted`
10. `AccessCompleted { signed_receipt }`
11. `AccountMayHaveCommitted`
12. `CompletedAwaitingAcknowledgement { signed_receipt }`
13. `Acknowledged`

Each record contains version, revision, operation ID, authorization digest,
canonical plan bytes/hash, source and landing profile names/DIDs, providers,
phase, bitmap, and provider receipts. It contains no passkey material or root
signer.

Execution order:

1. verify/consume the challenge and persist `Authorized`;
2. reserve account service, then access service;
3. create and boot a fresh landing profile;
4. durably point the registry at the landing profile before source teardown;
5. retract local source-profile credentials, bindings, roster facts, and local
   space material one item at a time, persisting the bitmap after each item;
6. persist `RemoteMayHaveStarted` before the first access commit attempt;
7. repeat/status-reconcile access commit until the signed receipt is durable;
8. persist `AccountMayHaveCommitted` before account commit;
9. repeat/status-reconcile account commit until its receipt is durable;
10. publish the safe terminal response from the landing profile, then mark
    `Acknowledged` only after the UI acknowledges it.

Boot recovery rules:

- Before `RemoteMayHaveStarted`, quarantine the source profile and resume from
  the recorded phase; never expose normal APIs while a reservation is active.
- At or after `RemoteMayHaveStarted`, open only the landing profile and status
  reconcile remote services. An absent response is uncertainty, not rollback.
- Repair a missing landing profile from marker facts; refuse a mismatching one.
- Unknown version, invalid canonical plan, impossible phase/receipt combination,
  or unreadable marker enters a fixed safe-boot screen with exportable
  diagnostics and no destructive retry button.

Integrate with profile generations: authorization binds the source generation;
the foreground request promotes its existing generation permit into exclusive
activation, drains child/system work, commits the landing pointer, resets
profile-scoped streams, and publishes only the landing generation.

## UI contract

The review screen shows the provider account, exact owned hosted spaces,
custody/account consequences, replication limits, and unchanged-state promise
before authorization. Arming requires the exact normalized email plus an
explicit irreversible-action checkbox.

After the passkey prompt:

- cancellation/time-out before authorization says nothing changed and permits
  retry;
- accepted authorization closes dismissal and shows durable phase progress;
- provider uncertainty says Tonk is checking the existing operation, never
  invites a second deletion;
- terminal success names what Tonk deleted and what it cannot delete on other
  devices;
- corrupt/unsupported recovery says what remains quarantined, that Tonk has
  not assumed completion, and how to export diagnostics/contact support.

Do not re-enable the button until the worker advertises the exact saga protocol
and both providers advertise reserve/status/commit capability.

## Reviewable implementation stack

### Slice A: shared protocol and inert providers

- Add canonical plan, authorization, and receipt types/tests.
- Add both migrations and reserve/status/commit handlers behind new commands.
- Keep legacy account deletion refused and UI disabled.
- Prove native/Cloudflare route parity, D1/SQLite parity, replay conflicts,
  denial-first behavior, custody-last ordering, and receipt verification.

### Slice B: worker saga and safe boot

- Add registry marker/CAS, challenge endpoint, operation status, landing profile
  rotation, local bitmap finalization, provider reconciliation, and safe boot.
- Keep UI action disabled except in test harnesses.
- Test a crash/reload before and after every phase write and every remote call;
  test corrupt markers, provider timeouts, duplicate fetches, two tabs, worker
  update during each phase, pointer failure, and profile-generation drain.

### Slice C: custody relay and UI enablement

- Add the exact root-signed terminal authorization ceremony and reviewed UI.
- Gate enablement on worker/provider protocol discovery.
- Update Storybook source/generated data and run the complete account browser
  matrix against new services, old services, an old worker, offline/reload,
  passkey cancel, response loss, and multi-tab contention.

## Required release order and rollback boundary

1. Deploy account/access services with inert new commands and migrations.
2. Deploy the marker-reading worker with the saga still disabled.
3. Deploy the UI capability gate and enable the action.

Once any production marker reaches `RemoteMayHaveStarted`, rolling the worker
back below the marker reader is unsafe. Roll forward with recovery fixes; do
not clear the marker or restore denied consumers manually. The fail-closed
legacy route may remain indefinitely as a compatibility boundary.

## Completion evidence

- [ ] Shared fixed vectors and malformed/cross-provider authorization matrix.
- [ ] Store transaction, idempotence, conflicting replay, and tombstone tests
  for SQLite and D1 adapters.
- [ ] Access denial precedes purge; custody is last; signed receipt is exact.
- [ ] Account commit accepts only that receipt and captured account ID.
- [ ] Worker crash/reload matrix covers every phase and lost response boundary.
- [ ] Two-tab, stale generation, old worker/service, offline, and corrupt-marker
  behavior is fail-closed and actionable.
- [ ] Storybook source/generated links and account browser journeys pass.
- [ ] Whole-account UI remains disabled until all preceding items are green.
