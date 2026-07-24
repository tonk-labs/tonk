# Signed Revocation Artifacts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a revocation a signed, verifiable, portable artifact instead of a database flag. `POST /devices/revoke` records a `ucan/revoke` invocation alongside the status write, so revocation carries a cryptographic trail and a second enforcement point becomes possible without handing out a database credential.

**Architecture:** The account service gains a revocation artifact written to the existing R2 chain store (`ChainStore`, namespaced by account root DID), under a `revocations/` key prefix. D1 `status = 'revoked'` stays exactly as it is and remains what the access-service presign screen reads — this stage adds an artifact, it does not move the hot path. A `GET /devices/revocations` read endpoint exposes the set so a future non-worker enforcement point can sync it.

**Tech Stack:** `dialog-ucan-core` (`InvocationBuilder`, `Invocation`, `Container`), `dialog-credentials` (`Ed25519Signer`), the crate's existing `ChainStore`/`Store` traits, `dialog_common::test`.

## Authority — decided

**Who signs the revocation?** UCAN semantics: the issuer of a delegation, or a principal holding a delegated `ucan/revoke` capability, may revoke it. The issuer of `root → device` is the root, and the account service does not hold the root key — it is derived from the passkey PRF on the user's device and never leaves it. So the service cannot mint the artifact itself; a client must.

**Decision: authority scales with blast radius.**

| | Self-revoke (device revokes itself) | Cross-device revoke |
|---|---|---|
| Signed by | the device's own key | the account root |
| Available when | always, offline, no prompt | a passkey-holding device is at hand |
| What it means | "sign this account out of this device" | "that device is no longer me" |
| Can a stolen device do it | yes, to itself only | **no** |

The artifact records whichever authority actually authorized the action — a root-signed artifact standing in for a device-authorized revoke would be a fiction, and a verifier that trusts it learns something false. So both shapes are legitimate and the artifact carries its attestation level; consumers set their own bar. A second enforcement point deciding "root-attested only" is making a policy choice, not detecting a defect.

Consequences this plan must carry:

- **The current policy is the permissive one and is a live irreversible DoS.** `handle_revoke_inner` authorizes any active device of the account and takes the target `did` as a free argument, and there is no un-revoke anywhere in the store. Any device can therefore lock out every sibling, permanently. Tightening cross-device revoke to root is what closes it.
- **Tightening depends on stage C.** If the lost device *is* the passkey device, its owner cannot derive the root and so cannot revoke it — the ceremony fails in its own primary scenario. Do not land the cross-device tightening before surviving-device recovery exists, or ship it with an explicit escape hatch. Until then, cross-device revoke stays device-signed and records a device-attested artifact.
- **Passkey syncability decides the felt cost.** A synced passkey (iCloud Keychain, a password manager) derives the same root on every device in the sync group, so the ceremony is one prompt anywhere. A device-bound passkey lives on exactly one device. Same feature, very different UX; the device-management copy should not promise the easy case.
- **Delegated signing needs no migration.** `root → device` is minted `command(vec![])`, and an empty command is a prefix of every command (`Command::starts_with` is vacuously true), so every linked device already holds authority to sign `ucan/revoke` under the root. Nothing to issue, nothing to re-link.
- **Revocation is permanent for a key.** The access-service screen matches `device_did`, so re-registering the same device key after a mistaken revoke cannot restore access. Recovery is a fresh key, delegation and row — not a flag flip. Stage D's copy must say so.

## Why this shape (decisions)

- **Artifact next to the index, not instead of it.** D1 `status` is what the
  presign screen reads (`tonk-access-service/src/revocation.rs`), and it is
  one indexed lookup on the hot path. The artifact is written on the cold
  path and read by anything that is not that worker. Replacing the index
  with artifact verification on every presign would be a large latency
  regression for no security gain — the worker already trusts D1 for the
  account data it serves.
- **Reuse `ChainStore`, do not add a table.** Chain backup already stores
  content-addressed UCAN bytes namespaced by root DID
  (`core/backup.rs:chain_key` hashes the bytes). Revocations are the same
  shape of thing. A `revocations/` key prefix keeps them enumerable
  separately from delegation backups.
- **Append-only, never delete.** The revocation set is monotone — this is
  what makes it safe to cache and to gossip. There is no un-revoke today
  (`store.rs` has `revoke_device` and no inverse) and this stage must not
  introduce one.
- **Verify on read, not on write.** The service checks the artifact is
  well-formed and issued by the account root before storing it, so garbage
  cannot be parked in a user's namespace; consumers re-verify
  independently, because a consumer that trusts the store has gained
  nothing over trusting D1.

## Global Constraints

- Lint gate: `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo fmt --check`.
- Tests: `#[dialog_common::test]`, names `it_does_x`, native tests gated `#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]`.
- No `mod.rs`: `foo.rs` + `foo/` form.
- No stage/phase/PR references in code or doc comments.
- Conventional commits, scope `tonk-account-service` (client work: scope `tonk-identity`).
- The access-service is not modified by this stage. If a task wants to touch it, stop — that is stage-creep and belongs to whatever builds the second enforcement point.

---

### Task 1: Verify the invariants this plan stands on

**Files:** none modified — read-only verification.

- [ ] **Step 1: dialog has no revocation type to conflict with**

```bash
grep -rn "Revocation\|ucan/revoke" ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-ucan-core/src/
```
Expected: no matches. The revocation is modelled as an ordinary `Invocation` with command `["ucan", "revoke"]`; nothing upstream competes with or verifies that shape.

- [ ] **Step 2: `InvocationBuilder` takes an arbitrary command and arguments**

```bash
grep -n "pub fn command\|pub fn arguments\|pub fn subject\|pub fn proofs" ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-ucan-core/src/invocation/builder.rs
```
Expected: builder accepts `Vec<String>` command and a `BTreeMap` of arguments, so `["ucan","revoke"]` with `{"revoke": <cid string>}` needs no upstream change.

- [ ] **Step 3: the chain store is reachable from the revoke handler's context**

```bash
grep -n "chain_store\|ChainStore\|R2" rust/tonk-account-service/src/handlers/devices.rs rust/tonk-account-service/src/handlers.rs
```
Expected: `handle_revoke_inner` currently builds only a `Store`. Note what the chains handlers do to obtain a `ChainStore` from `RouteContext` — Task 3 mirrors it. If they use a different binding pattern, follow theirs.

- [ ] **Step 4: `revoke_device` is the only status writer and there is no inverse**

```bash
grep -rn "status = 'revoked'\|DeviceStatus::Active" rust/tonk-account-service/src/store/
```
Expected: one `UPDATE ... SET status = 'revoked'`, no path back to `active`. If an un-revoke has appeared, STOP: the monotonicity argument the cache and this stage both rest on is void, and the access-service grace window needs re-examining first.

**STOP conditions:**
- The root key is reachable server-side by any path → the blocking decision above is moot and the plan needs rewriting around service-side signing.
- `ChainStore` has grown a delete → revocations must not be stored somewhere erasable; use a separate append-only prefix with no delete path.

- [ ] **Step 5: Nothing to commit** — verification only.

---

### Task 2: Mint the revocation client-side

**Files:**
- Modify: `rust/tonk-identity/src/revocation.rs` (new), `rust/tonk-identity/src/lib.rs`

**Interfaces:**
- Produces: `mint_root_revocation(root: Ed25519Signer, delegation_cid: &Cid)` and `mint_self_revocation(device: Ed25519Signer, root_delegation: &DelegationChain, delegation_cid: &Cid)`, both returning container bytes, consumed by Task 3's handler and Task 5's client call.

- [ ] **Step 1: Write the failing tests first**

`it_mints_a_root_signed_revocation_naming_the_delegation`: mint a `root → device` delegation, take its CID, mint a revocation, assert the invocation's issuer is the root DID, its command is `["ucan","revoke"]`, and its arguments carry the delegation CID as a string.

`it_mints_a_device_signed_self_revocation`: same, issued by the device and carrying the `root → device` delegation as its proof, so the chain shows the authority used.

- [ ] **Step 2: Implement**

Mirror `mint_device_delegation` in the same crate: `InvocationBuilder::new().issuer(...).audience(&root_did).subject(&root_did).command(vec!["ucan".into(), "revoke".into()]).arguments(...)`, serialize through a container. Subject is the account root in both cases: the revocation is an act on the account, not on a space. The self-revocation additionally carries its `root → device` proof; the root-signed one needs no proof (issuer equals subject).

- [ ] **Step 3: Third test** — `it_distinguishes_a_root_signed_revocation_from_a_self_revocation`, asserting the two are separable by issuer without parsing arguments. Task 4 depends on that being cheap.

- [ ] **Step 4: Commit** `feat(tonk-identity): mint signed device revocations`

---

### Task 3: Store the artifact on revoke

**Files:**
- Modify: `rust/tonk-account-service/src/core/devices.rs`, `rust/tonk-account-service/src/handlers/devices.rs`, `rust/tonk-account-service/src/helpers/server.rs`

**Interfaces:**
- Consumes: Task 2's container bytes, arriving as a new required field on the revoke request body.
- Produces: an object at `revocations/{chain_key(bytes)}` in the account's namespace.

- [ ] **Step 1: Extend `revoke_device` to take the artifact**

Signature becomes `revoke_device<S: Store, C: ChainStore>(store, chains, account, device_did, revocation_bytes)`. Verify before storing (Task 4), then write the artifact **before** flipping the status — a stored artifact with no flag is a recoverable inconsistency; a flag with no artifact is a silent gap in the audit trail.

- [ ] **Step 2: Tests**

`it_stores_the_artifact_and_flips_the_status`, `it_does_not_flip_the_status_when_the_artifact_is_rejected`, `it_keeps_earlier_revocations_when_a_second_device_is_revoked` (monotonicity).

- [ ] **Step 3: Mirror the wiring in the native helpers server** so the HTTP integration test covers the same path.

- [ ] **Step 4: Commit** `feat(tonk-account-service): record a signed artifact on device revoke`

---

### Task 4: Verify the artifact before accepting it

**Files:**
- Modify: `rust/tonk-account-service/src/core/revocation.rs` (new)

- [ ] **Step 1: Write the tests first** — a revocation issued by a foreign root is rejected; one naming a delegation CID that is not the target device's registered `delegation_cid` is rejected; a device-signed revocation naming *another* device is rejected (this is the authority rule, and it is the test that matters); a device-signed revocation naming itself is accepted; a root-signed revocation of any device of that account is accepted. Reuse the fixture style in `core/devices.rs` tests.

- [ ] **Step 2: Implement** `check_revocation(bytes, root_did, target_device) -> Result<Attestation>` mirroring `core/delegation.rs:check_device_delegation` — parse, verify signature and chain, check the named CID matches the target's row, then classify: issuer equals the account root → `Attestation::Root`; issuer equals the target device and the chain proves `root → that device` → `Attestation::Device`; anything else is rejected. The returned attestation is stored with the artifact so consumers can filter on it.

- [ ] **Step 3: Commit** `feat(tonk-account-service): verify revocation artifacts before storing them`

---

### Task 5: Expose the revocation set

**Files:**
- Modify: `rust/tonk-account-service/src/handlers/devices.rs`, `rust/tonk-account-service/src/lib.rs`, `rust/tonk-account-service/src/helpers/server.rs`, `rust/tonk-account-service/README.md`

- [ ] **Step 1: `GET`-shaped read endpoint** `POST /devices/revocations` (device-signed invocation, command `["account","device","revocations"]` — the crate's endpoints are all POST-with-invocation; follow that, do not invent a bearer route). Returns the stored artifacts as an array of hex-encoded containers.

- [ ] **Step 2: Tests** — a caller sees only their own account's revocations (mirror the cross-account `/chains/get` isolation test added in the hardening batch).

- [ ] **Step 3: README** — document the endpoint and state plainly that consumers MUST verify artifacts themselves rather than trusting the service.

- [ ] **Step 4: Commit** `feat(tonk-account-service): serve the signed revocation set`

---

### Task 6: Client plumbing and the revoke ceremony

**Files:**
- Modify: `rust/tonk-worker/` account routes, `rust/tonk-ui/` account element (whatever D built)

Depends on D's device-management surface existing. If D has not landed, stop after Task 5 and let D pick this up — the endpoints are complete and testable without a UI.

- [ ] **Step 1:** Wire "sign out on this device" through self-revocation (device key, no prompt) and the device-list revoke button through the root ceremony (passkey assertion → derive root → sign). Copy must state that revoking is permanent for that device's key: coming back means re-linking as a new device.
- [ ] **Step 2:** Manual staging smoke: self-revoke from a device and confirm the artifact is device-attested; revoke a second device from a passkey-holding device and confirm the artifact is root-attested; confirm presigns from both 403 within 60s; confirm both artifacts verify standalone against the account root DID.
- [ ] **Step 3: Commit** and update the completion spec's stage S section to "shipped".

---

### Task 7: Tighten cross-device revoke authority — GATED ON STAGE C

**Do not start this task before surviving-device recovery exists.** Until then, a user whose passkey lived on the lost device cannot derive the root, and tightening would leave them unable to revoke the very device they lost — strictly worse than today.

**Files:**
- Modify: `rust/tonk-account-service/src/handlers/devices.rs`, `rust/tonk-account-service/src/auth.rs`

- [ ] **Step 1:** Require a root-attested artifact when the target device is not the caller; accept device-attested only for self-revoke. This is the change that closes the live DoS where any active device can permanently lock out its siblings.
- [ ] **Step 2:** Tests — cross-device revoke with a device-attested artifact is rejected; with a root-attested artifact is accepted; self-revoke is unaffected.
- [ ] **Step 3:** Note in the completion spec's risk list that the DoS is closed, and drop it from the live-issues list.
- [ ] **Step 4: Commit** `feat(tonk-account-service): require root attestation to revoke another device`
