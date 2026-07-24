# Recovery and Rotation Ceremonies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the three missing identity-recovery ceremonies — deliberate passkey rotation (succession), surviving-device recovery, and total-loss re-anchor — so an account is no longer welded to a single passkey credential with no exit.

**Architecture:** Three PR-sized sub-stages, in order. Rotation mints a subject-open `oldRoot → newRoot` succession delegation and flips the account row under old-root authority; surviving-device recovery flips it under device + new-root two-container authority; total-loss re-anchor flips it under email-code authority with no delegation bridge (space access intentionally lost). All three converge rosters and capabilities with the already-landed migration machinery (`migrate.rs`), generalized from `device → root` to `from → to`.

**Tech Stack:** workers-rs (tonk-account-service), dialog-ucan-core (delegations/invocations), tonk-identity (ceremony builders + wasm bindings), tonk-worker (axum-wasm router), tonk-ui (custom element panels), rusqlite/D1 dual store.

## Global Constraints

- No `mod.rs` — `foo.rs` + `foo/` form everywhere.
- Tests: `#[dialog_common::test]`, names `it_does_x`, grouped by behaviour.
- No stage/phase/RFC references in code or doc comments.
- Lint gate: `cargo clippy --workspace --all-targets --all-features` + `cargo fmt --check` (native); wasm-gated code must also pass `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests`.
- Account-service tests: `cargo test -p tonk-account-service --features helpers`.
- Identity tests: `cargo test -p tonk-identity`.
- Conventional commits, subject imperative, no trailing period.
- wasm/service-worker tests hang locally — compile them (`--target wasm32-unknown-unknown`), rely on the CI web leg to execute, and say so in the PR body.
- The root key exists in memory only for the seconds a ceremony needs it (`Zeroizing` seeds, signers dropped at ceremony end).

## Chain shapes (load-bearing — read before any task)

Delegation arrows point issuer → audience. All identity links are
subject-open (`Subject::Any`).

```
Before rotation:
  space chains:   space → … → oldRoot          (claimed/owned, subject-specific)
  device links:   oldRoot → deviceN            (subject-open)
  presentation:   [space → … → oldRoot, oldRoot → deviceN]

Rotation mints:   oldRoot → newRoot            (subject-open succession)
                  newRoot → ceremonyDevice     (fresh link for the device in hand)

After rotation:
  old devices:    [space → … → oldRoot, oldRoot → deviceN]        — still valid, untouched
  new devices:    [space → … → oldRoot, oldRoot → newRoot, newRoot → deviceM]
  new claims by re-linked devices anchor at newRoot directly.
  Old devices CANNOT use newRoot-anchored chains (no newRoot → oldDevice link);
  they re-link lazily: their next device-signed service call fails with
  "unknown account" (subject = oldRoot no longer in the registry), which the
  UI answers with the existing self-link ceremony (synced passkey, one prompt).

Surviving-device recovery mints:
                  newRoot → survivingDevice    (fresh link, new root key in hand)
  and the convergence sweep re-anchors every space the surviving device holds:
                  space → … → survivingDevice → newRoot   (claim().delegate(newRoot))
  so newRoot (and devices later linked under it) reach everything the
  surviving device could reach. No oldRoot key is ever needed.

Total-loss re-anchor mints NOTHING. The account row points at a fresh root;
space access is gone by design; founders re-invite (rosters keyed on the old
root make affected spaces discoverable).
```

Registry consequences of a root flip (all three ceremonies): device rows
other than the ceremony device keep their `oldRoot → device` delegation
CIDs. They stay `active` in rotation and recovery (their space chains stay
valid; only their *service* auth breaks until re-link). Total-loss revokes
every device row. The passkey credential is "revoked" by replacement:
`accounts.credential_id` changes and `accounts.root_did` no longer matches
anything the old credential can derive, so the old passkey can never
authorize another ceremony.

## Decisions locked by this plan

1. Two-container wire shape for rotation and recovery: JSON body with two
   hex-encoded invocation containers; each is verified independently and
   cross-checked by arguments (each container names the other's principal).
2. `check_device_delegation` is renamed `check_subject_open_delegation`
   (it is shape-generic and now validates root→device, oldRoot→newRoot,
   and newRoot→device links; the old name would lie at two new call sites).
3. Devices stay `active` across rotation and recovery; only total-loss
   revokes them. Rationale in the chain-shape block above.
4. The ceremony device is re-registered in the same service call that flips
   the root (upsert of its `delegation_cid`), so at least one device can
   always authorize device-signed calls immediately after a flip.
5. The worker replaces a stored account link only with proof of continuity:
   a succession delegation whose issuer equals the currently stored root
   (rotation), or internally after a service-confirmed recovery. A bare
   "overwrite my account" request still gets `Conflict`.
6. Convergence after a flip reuses `migrate.rs`: `migrate_space_roster` is
   generalized to `rekey_space_roster(tonk, key, from, to)` and the
   existing `reanchor_space` runs unchanged (it reads the *new* root from
   the freshly stored link).

---

## Sub-stage 1 — Succession / deliberate rotation (one PR)

### Task 1: Rename the delegation check to its real shape

**Files:**
- Modify: `rust/tonk-account-service/src/core/delegation.rs`
- Modify: `rust/tonk-account-service/src/core/accounts.rs` (call site)
- Modify: `rust/tonk-account-service/src/core/devices.rs` (call site)

**Interfaces:**
- Produces: `pub async fn check_subject_open_delegation(delegation_hex: &str, issuer_did: &str, audience_did: &str) -> Result<String, CeremonyError>` — identical behavior to today's `check_device_delegation`; later tasks call it for succession (`oldRoot → newRoot`) and recovery (`newRoot → device`) links.

- [ ] **Step 1: Rename fn and parameters**

In `rust/tonk-account-service/src/core/delegation.rs`, rename
`check_device_delegation` → `check_subject_open_delegation`, and its
parameters `root_did` → `issuer_did`, `device_did` → `audience_did`.
Update the module doc and the two error strings that mention root/device:

```rust
//! Checking a subject-open, single-hop delegation chain presented during
//! account ceremonies: `root → device` at creation and linking,
//! `oldRoot → newRoot` at rotation, `newRoot → device` at recovery.

/// Parse and check a hex-encoded subject-open delegation chain.
///
/// Requires exactly one proof, issued by `issuer_did` to `audience_did`,
/// subject-open, with a valid signature. Returns the delegation's CID,
/// stringified — the key `devices.delegation_cid` is stored under.
pub async fn check_subject_open_delegation(
    delegation_hex: &str,
    issuer_did: &str,
    audience_did: &str,
) -> Result<String, CeremonyError> {
```

and inside, the two identity checks become:

```rust
    if chain.issuer().to_string() != issuer_did {
        return Err(CeremonyError::Invalid(
            "delegation issuer does not match the expected principal".to_string(),
        ));
    }
    if chain.audience().to_string() != audience_did {
        return Err(CeremonyError::Invalid(
            "delegation audience does not match the expected principal".to_string(),
        ));
    }
```

- [ ] **Step 2: Update the two call sites**

`core/accounts.rs` line ~44 and `core/devices.rs` line ~57: change
`check_device_delegation(` to `check_subject_open_delegation(` (imports:
`use crate::core::delegation::check_subject_open_delegation;`). Argument
order and meaning are unchanged.

- [ ] **Step 3: Run the crate tests**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: PASS (pure rename; existing delegation-shape tests still cover it)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-account-service/src/core/delegation.rs rust/tonk-account-service/src/core/accounts.rs rust/tonk-account-service/src/core/devices.rs
git commit -m "refactor(tonk-account-service): name the delegation check by its shape"
```

### Task 2: Store support for flipping an account root

**Files:**
- Modify: `rust/tonk-account-service/src/store.rs`
- Modify: `rust/tonk-account-service/src/store/sqlite.rs`
- Modify: `rust/tonk-account-service/src/store/d1.rs`

**Interfaces:**
- Produces on trait `Store`:
  - `async fn rotate_root(&self, account_id: i64, new_root_did: &str, new_credential_id: &str) -> Result<(), StoreError>` — flips `accounts.root_did` + `credential_id`; `Conflict` if the new root DID is already registered.
  - `async fn update_device_delegation(&self, account_id: i64, device_did: &str, delegation_cid: &str) -> Result<bool, StoreError>` — repoints one device row's delegation CID; `false` when no row matched.

- [ ] **Step 1: Add SQL consts and trait methods in `store.rs`**

After `UPDATE_DEVICE_REVOKE`:

```rust
/// SQL: flip an account's root DID and passkey credential.
pub const UPDATE_ACCOUNT_ROOT: &str =
    "UPDATE accounts SET root_did = ?2, credential_id = ?3 WHERE id = ?1";

/// SQL: repoint one device row at a fresh delegation.
pub const UPDATE_DEVICE_DELEGATION: &str =
    "UPDATE devices SET delegation_cid = ?3 WHERE account_id = ?1 AND device_did = ?2";
```

Trait additions (after `revoke_device`):

```rust
    /// Flip the account's root DID and passkey credential in one
    /// statement. Returns `StoreError::Conflict` if the new root DID is
    /// already registered to any account.
    async fn rotate_root(
        &self,
        account_id: i64,
        new_root_did: &str,
        new_credential_id: &str,
    ) -> Result<(), StoreError>;

    /// Repoint one device row at a fresh delegation CID. Returns `false`
    /// if no matching device was found.
    async fn update_device_delegation(
        &self,
        account_id: i64,
        device_did: &str,
        delegation_cid: &str,
    ) -> Result<bool, StoreError>;
```

- [ ] **Step 2: Write failing store tests**

In `rust/tonk-account-service/src/store/sqlite.rs`'s existing test module,
following its fixture style:

```rust
    #[dialog_common::test]
    async fn it_rotates_the_root_and_keeps_the_row_id() {
        let store = SqliteStore::in_memory().unwrap();
        let id = store
            .create_account("a@x.com", "did:key:zOld", "cred-old", 100)
            .await
            .unwrap();
        store.rotate_root(id, "did:key:zNew", "cred-new").await.unwrap();
        assert!(store.account_by_root("did:key:zOld").await.unwrap().is_none());
        let account = store.account_by_root("did:key:zNew").await.unwrap().unwrap();
        assert_eq!((account.id, account.credential_id.as_str()), (id, "cred-new"));
    }

    #[dialog_common::test]
    async fn it_refuses_rotating_onto_a_registered_root() {
        let store = SqliteStore::in_memory().unwrap();
        let id = store
            .create_account("a@x.com", "did:key:zA", "cred-a", 100)
            .await
            .unwrap();
        store
            .create_account("b@x.com", "did:key:zB", "cred-b", 100)
            .await
            .unwrap();
        assert!(matches!(
            store.rotate_root(id, "did:key:zB", "cred-x").await,
            Err(StoreError::Conflict(_))
        ));
    }

    #[dialog_common::test]
    async fn it_repoints_a_device_delegation() {
        let store = SqliteStore::in_memory().unwrap();
        let id = store
            .create_account("a@x.com", "did:key:zA", "cred", 100)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: id,
                device_did: "did:key:zDev".into(),
                delegation_cid: "bafyOld".into(),
                name: "laptop".into(),
                status: DeviceStatus::Active,
                created_at: 100,
            })
            .await
            .unwrap();
        assert!(store
            .update_device_delegation(id, "did:key:zDev", "bafyNew")
            .await
            .unwrap());
        let device = store.device_by_did("did:key:zDev").await.unwrap().unwrap();
        assert_eq!(device.delegation_cid, "bafyNew");
        assert!(!store
            .update_device_delegation(id, "did:key:zGhost", "bafyNew")
            .await
            .unwrap());
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p tonk-account-service --features helpers rotates`
Expected: FAIL — `rotate_root` not a member of trait `Store`

- [ ] **Step 4: Implement in `sqlite.rs`**

Mirroring `revoke_device`'s body style (`self.0.lock()`, `map_err`;
uniqueness violations already map to `Conflict` via the existing
`map_err`):

```rust
    async fn rotate_root(
        &self,
        account_id: i64,
        new_root_did: &str,
        new_credential_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            UPDATE_ACCOUNT_ROOT,
            params![account_id, new_root_did, new_credential_id],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn update_device_delegation(
        &self,
        account_id: i64,
        device_did: &str,
        delegation_cid: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                UPDATE_DEVICE_DELEGATION,
                params![account_id, device_did, delegation_cid],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }
```

Verify sqlite's `map_err` maps `SQLITE_CONSTRAINT_UNIQUE` to
`StoreError::Conflict` (it must, since `create_account` relies on it); if
it maps by message substring, confirm the UPDATE unique-violation message
matches. If it does not, extend `map_err`, not the method.

- [ ] **Step 5: Implement in `d1.rs`**

Mirroring the existing `revoke_device` D1 body: `prepare(...).bind(&[...])`
with `JsValue::from_f64(account_id as f64)` for the id,
`JsValue::from(...)` for strings; `rotate_root` uses `.run()` and maps a
D1 uniqueness error to `Conflict` the same way `create_account` does in
that file (read its error mapping and reuse it verbatim);
`update_device_delegation` reads `meta().rows_written` (same accessor
`revoke_device` uses) for the `bool`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: PASS, including the three new tests

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-account-service/src/store.rs rust/tonk-account-service/src/store/sqlite.rs rust/tonk-account-service/src/store/d1.rs
git commit -m "feat(tonk-account-service): store support for flipping an account root"
```

### Task 3: Rotation core ceremony

**Files:**
- Create: `rust/tonk-account-service/src/core/rotation.rs`
- Modify: `rust/tonk-account-service/src/core.rs` (add `pub mod rotation;`)

**Interfaces:**
- Consumes: `check_subject_open_delegation` (Task 1), `rotate_root` / `update_device_delegation` (Task 2), `Account` from `crate::store`.
- Produces: `pub struct RotateAccount { pub new_root_did: String, pub new_credential_id: String, pub succession_hex: String, pub device_did: String, pub device_delegation_hex: String }` and `pub async fn rotate_account<S: Store>(store: &S, account: &Account, request: &RotateAccount) -> Result<(), CeremonyError>`.

- [ ] **Step 1: Write the failing tests**

`rust/tonk-account-service/src/core/rotation.rs`, tests first (same
fixture idioms as `core/devices.rs`):

```rust
#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus, Store};
    use dialog_varsig::Principal;

    const OLD_ROOT_PRF: [u8; 32] = [7u8; 32];
    const NEW_ROOT_PRF: [u8; 32] = [9u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn fixture(store: &SqliteStore) -> (crate::store::Account, RotateAccount, String) {
        let old_root = tonk_identity::derive::derive_root_signer(&OLD_ROOT_PRF)
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&DEVICE_SEED)
            .await
            .unwrap();
        let old_root_did = old_root.did().to_string();
        let new_root_did = new_root.did().to_string();
        let device_did = device.did().to_string();

        let id = store
            .create_account("a@x.com", &old_root_did, "cred-old", 100)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: id,
                device_did: device_did.clone(),
                delegation_cid: "bafyOld".into(),
                name: "laptop".into(),
                status: DeviceStatus::Active,
                created_at: 100,
            })
            .await
            .unwrap();

        let succession =
            tonk_identity::delegation::mint_root_succession(old_root, &new_root.did())
                .await
                .unwrap();
        let device_link =
            tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
                .await
                .unwrap();
        let account = store.account_by_root(&old_root_did).await.unwrap().unwrap();
        let request = RotateAccount {
            new_root_did: new_root_did.clone(),
            new_credential_id: "cred-new".into(),
            succession_hex: hex::encode(succession.to_bytes().unwrap()),
            device_did,
            device_delegation_hex: hex::encode(device_link.to_bytes().unwrap()),
        };
        (account, request, new_root_did)
    }

    #[dialog_common::test]
    async fn it_flips_the_root_and_repoints_the_ceremony_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, request, new_root_did) = fixture(&store).await;

        rotate_account(&store, &account, &request).await.unwrap();

        let rotated = store.account_by_root(&new_root_did).await.unwrap().unwrap();
        assert_eq!(rotated.id, account.id);
        assert_eq!(rotated.credential_id, "cred-new");
        let device = store
            .device_by_did(&request.device_did)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(device.delegation_cid, "bafyOld");
        assert_eq!(device.status, DeviceStatus::Active);
    }

    #[dialog_common::test]
    async fn it_rejects_a_succession_not_issued_by_the_account_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, mut request, _) = fixture(&store).await;
        // Succession minted by an unrelated key: issuer check must fail.
        let stranger = tonk_identity::derive::derive_root_signer(&[13u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let bogus = tonk_identity::delegation::mint_root_succession(stranger, &{
            use dialog_varsig::Principal;
            new_root.did()
        })
        .await
        .unwrap();
        request.succession_hex = hex::encode(bogus.to_bytes().unwrap());

        assert!(matches!(
            rotate_account(&store, &account, &request).await,
            Err(CeremonyError::Invalid(_))
        ));
        // Nothing flipped.
        assert!(store
            .account_by_root(&account.root_did)
            .await
            .unwrap()
            .is_some());
    }

    #[dialog_common::test]
    async fn it_rejects_a_ceremony_device_unknown_to_the_account() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, mut request, _) = fixture(&store).await;
        request.device_did = "did:key:zGhost".into();
        assert!(matches!(
            rotate_account(&store, &account, &request).await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}
```

(`mint_root_succession` does not exist yet — Task 4 adds it to
tonk-identity first if you are executing tasks strictly in order, swap
Tasks 3 and 4; they are written in this order because the service core is
the riskier surface. Both orders work; the test file compiles only when
both tasks are done.)

- [ ] **Step 2: Implement the core**

```rust
//! The account rotation ceremony: flip the account onto a new root DID
//! under authority of the old root, keeping every registered device.

use crate::core::CeremonyError;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::{Account, Store};

/// A verified request to rotate an account onto a new root.
pub struct RotateAccount {
    /// The DID the account rotates onto.
    pub new_root_did: String,
    /// The passkey credential backing the new root.
    pub new_credential_id: String,
    /// Hex-encoded subject-open `oldRoot → newRoot` succession chain.
    pub succession_hex: String,
    /// The ceremony device re-registering under the new root.
    pub device_did: String,
    /// Hex-encoded subject-open `newRoot → device` delegation chain.
    pub device_delegation_hex: String,
}

/// Rotate `account` onto `request.new_root_did`.
///
/// Verifies the succession chain (old root delegates to the new root) and
/// the ceremony device's fresh delegation before touching the registry.
/// Devices other than the ceremony device keep their existing rows: their
/// old-root delegations remain cryptographically valid for space access,
/// and they re-link on their next service ceremony.
pub async fn rotate_account<S: Store>(
    store: &S,
    account: &Account,
    request: &RotateAccount,
) -> Result<(), CeremonyError> {
    check_subject_open_delegation(
        &request.succession_hex,
        &account.root_did,
        &request.new_root_did,
    )
    .await?;
    let delegation_cid = check_subject_open_delegation(
        &request.device_delegation_hex,
        &request.new_root_did,
        &request.device_did,
    )
    .await?;

    let repointed = store
        .update_device_delegation(account.id, &request.device_did, &delegation_cid)
        .await?;
    if !repointed {
        return Err(CeremonyError::Invalid(
            "ceremony device is not registered under this account".to_string(),
        ));
    }
    store
        .rotate_root(account.id, &request.new_root_did, &request.new_credential_id)
        .await?;
    Ok(())
}
```

Note the order: device repoint first, root flip second, so a bogus device
DID fails before the flip. The two statements are not one transaction; a
crash between them leaves a repointed device under the old root, which the
next rotation attempt repairs (both operations are idempotent re-runs).
Register `pub mod rotation;` in `core.rs`.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p tonk-account-service --features helpers rotation`
Expected: PASS (after Task 4 lands `mint_root_succession`)

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-account-service/src/core/rotation.rs rust/tonk-account-service/src/core.rs
git commit -m "feat(tonk-account-service): rotation ceremony core"
```

### Task 4: Succession delegation and rotation builder in tonk-identity

**Files:**
- Modify: `rust/tonk-identity/src/delegation.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs`

**Interfaces:**
- Consumes: `DelegationBuilder`/`InvocationBuilder` exactly as the existing fns use them.
- Produces:
  - `pub async fn mint_root_succession(old_root: Ed25519Signer, new_root: &Did) -> Result<DelegationChain>` in `delegation.rs`.
  - `pub struct RotationCeremony { pub old_root_did: String, pub new_root_did: String, pub succession_hex: String, pub device_delegation_hex: String, pub rotation_hex: String, pub confirmation_hex: String }` and `pub async fn rotate_account(old_root: Ed25519Signer, new_root: Ed25519Signer, new_credential_id: String, device_did: dialog_varsig::Did) -> Result<RotationCeremony>` in `ceremony.rs`.

- [ ] **Step 1: Extract the subject-open builder and add succession**

In `delegation.rs`, extract the shared body and keep both public names
honest:

```rust
async fn mint_subject_open(issuer: Ed25519Signer, audience: &Did) -> Result<DelegationChain> {
    let delegation = DelegationBuilder::new()
        .issuer(issuer)
        .audience(audience)
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the delegation: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

/// Mint the `root → device` delegation: subject-open, audience-specific —
/// "this device may act as me, for anything". Deliberately the opposite
/// shape from space invites, which are subject-specific and must stay so.
pub async fn mint_device_delegation(root: Ed25519Signer, device: &Did) -> Result<DelegationChain> {
    mint_subject_open(root, device).await
}

/// Mint the `oldRoot → newRoot` succession delegation. Same subject-open
/// shape as a device link: every chain anchored at the old root extends
/// through it to the new root, so rotation never rewrites space chains.
pub async fn mint_root_succession(
    old_root: Ed25519Signer,
    new_root: &Did,
) -> Result<DelegationChain> {
    mint_subject_open(old_root, new_root).await
}
```

- [ ] **Step 2: Write the failing ceremony test**

In `ceremony.rs` tests:

```rust
    #[dialog_common::test]
    async fn it_builds_a_two_container_rotation() {
        let old_root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let new_root = crate::derive::derive_root_signer(&[9u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let old_did = old_root.did().to_string();
        let new_did = new_root.did().to_string();

        let ceremony = rotate_account(old_root, new_root, "cred-new".into(), device.did())
            .await
            .unwrap();
        assert_eq!(ceremony.old_root_did, old_did);
        assert_eq!(ceremony.new_root_did, new_did);

        let rotation =
            InvocationChain::try_from(hex::decode(&ceremony.rotation_hex).unwrap().as_slice())
                .unwrap();
        rotation
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(rotation.issuer().to_string(), old_did);
        assert_eq!(
            rotation.command().0,
            vec!["account".to_string(), "rotate".to_string()]
        );
        assert_eq!(
            rotation.arguments().get("newRootDid"),
            Some(&Promised::String(new_did.clone()))
        );

        let confirmation = InvocationChain::try_from(
            hex::decode(&ceremony.confirmation_hex).unwrap().as_slice(),
        )
        .unwrap();
        confirmation
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(confirmation.issuer().to_string(), new_did);
        assert_eq!(
            confirmation.command().0,
            vec![
                "account".to_string(),
                "rotate".to_string(),
                "confirm".to_string()
            ]
        );
        assert_eq!(
            confirmation.arguments().get("oldRootDid"),
            Some(&Promised::String(old_did))
        );
    }
```

(Add `use dialog_ucan_core::promise::Promised;` to the test imports if not
already in scope.)

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p tonk-identity it_builds_a_two_container_rotation`
Expected: FAIL — `rotate_account` not found

- [ ] **Step 4: Implement the builder**

In `ceremony.rs` (reusing the private `build` helper for both containers —
note `build` returns an `AccountCeremony`, whose `invocation_hex` is the
piece each call contributes):

```rust
/// Output of the two-container rotation ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationCeremony {
    /// The account's current (old) root DID.
    pub old_root_did: String,
    /// The root DID the account rotates onto.
    pub new_root_did: String,
    /// Hex-encoded `oldRoot → newRoot` succession chain.
    pub succession_hex: String,
    /// Hex-encoded `newRoot → device` delegation for the ceremony device.
    pub device_delegation_hex: String,
    /// Hex-encoded old-root-signed rotation container.
    pub rotation_hex: String,
    /// Hex-encoded new-root-signed confirmation container.
    pub confirmation_hex: String,
}

/// Build both rotation containers, the succession chain, and a fresh
/// device link. The old root signs the rotation (account authority); the
/// new root signs the confirmation (proof the new DID is controllable, so
/// a typo cannot strand the account on an inert root).
pub async fn rotate_account(
    old_root: Ed25519Signer,
    new_root: Ed25519Signer,
    new_credential_id: String,
    device_did: dialog_varsig::Did,
) -> Result<RotationCeremony> {
    let old_root_did = old_root.did().to_string();
    let new_root_did = new_root.did().to_string();

    let succession =
        crate::delegation::mint_root_succession(old_root.clone(), &new_root.did()).await?;
    let succession_hex = hex::encode(
        succession
            .to_bytes()
            .context("failed to serialize the succession delegation")?,
    );
    let device_link =
        crate::delegation::mint_device_delegation(new_root.clone(), &device_did).await?;
    let device_delegation_hex = hex::encode(
        device_link
            .to_bytes()
            .context("failed to serialize the device delegation")?,
    );

    let rotation = build(
        old_root,
        vec!["account".into(), "rotate".into()],
        strings([
            ("newRootDid", new_root_did.clone()),
            ("newCredentialId", new_credential_id),
            ("succession", succession_hex.clone()),
            ("deviceDid", device_did.to_string()),
            ("deviceDelegation", device_delegation_hex.clone()),
        ]),
        device_did.to_string(),
        device_delegation_hex.clone(),
    )
    .await?;
    let confirmation = build(
        new_root,
        vec!["account".into(), "rotate".into(), "confirm".into()],
        strings([("oldRootDid", old_root_did.clone())]),
        device_did.to_string(),
        device_delegation_hex.clone(),
    )
    .await?;

    Ok(RotationCeremony {
        old_root_did,
        new_root_did,
        succession_hex,
        device_delegation_hex,
        rotation_hex: rotation.invocation_hex,
        confirmation_hex: confirmation.invocation_hex,
    })
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p tonk-identity`
Expected: PASS, including the new rotation test and existing delegation tests

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-identity/src/delegation.rs rust/tonk-identity/src/ceremony.rs
git commit -m "feat(tonk-identity): succession delegation and rotation ceremony builder"
```

### Task 5: `POST /accounts/rotate` handler + wire

**Files:**
- Create: `rust/tonk-account-service/src/handlers/rotate.rs`
- Modify: `rust/tonk-account-service/src/handlers.rs` (add `pub mod rotate;`)
- Modify: `rust/tonk-account-service/src/lib.rs` (routes)
- Modify: `rust/tonk-account-service/tests/service.rs` (integration coverage)

**Interfaces:**
- Consumes: `authorize_root` (both containers), `rotate_account` core (Task 3), `required_string`.
- Produces: `POST /accounts/rotate`, JSON body `{ "rotation": "<hex>", "confirmation": "<hex>" }`, `200 {}` on success. All other request data rides inside the signed rotation container's arguments.

- [ ] **Step 1: Write the handler**

`rust/tonk-account-service/src/handlers/rotate.rs`, following
`handlers/accounts.rs` structure exactly (`handle` + `handle_inner` +
`handle_options`, `with_cors_headers`, `build_store`, `ceremony_error`):

```rust
//! `POST /accounts/rotate`: flip an account onto a new root DID under
//! old-root authority, with new-root proof of control.

use serde::Deserialize;
use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::rotation::{RotateAccount, rotate_account};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, with_cors_headers};

#[derive(Deserialize)]
struct RotateBody {
    rotation: String,
    confirmation: String,
}

/// `OPTIONS /accounts/rotate` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /accounts/rotate` → rotate an account onto a new root.
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body: RotateBody = req.json().await.map_err(|err| {
        ServiceError::new(ErrorCode::InvalidArgument, format!("bad body: {err}"))
    })?;
    let rotation_bytes = hex::decode(&body.rotation).map_err(|err| {
        ServiceError::new(ErrorCode::InvalidArgument, format!("bad rotation hex: {err}"))
    })?;
    let confirmation_bytes = hex::decode(&body.confirmation).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("bad confirmation hex: {err}"),
        )
    })?;

    let old = authorize_root(&rotation_bytes, &["account", "rotate"])
        .await
        .map_err(ceremony_error)?;
    let new = authorize_root(&confirmation_bytes, &["account", "rotate", "confirm"])
        .await
        .map_err(ceremony_error)?;

    // Each container must name the other's principal.
    let claimed_new = required_string(&old.arguments, "newRootDid").map_err(ceremony_error)?;
    let claimed_old = required_string(&new.arguments, "oldRootDid").map_err(ceremony_error)?;
    if claimed_new != new.root_did || claimed_old != old.root_did {
        return Err(ServiceError::new(
            ErrorCode::Forbidden,
            "rotation and confirmation containers do not name each other",
        ));
    }

    let store = build_store(ctx)?;
    let account = store
        .account_by_root(&old.root_did)
        .await
        .map_err(|err| ceremony_error(err.into()))?
        .ok_or_else(|| ServiceError::new(ErrorCode::Unauthorized, "unknown account"))?;

    let request = RotateAccount {
        new_root_did: new.root_did,
        new_credential_id: required_string(&old.arguments, "newCredentialId")
            .map_err(ceremony_error)?,
        succession_hex: required_string(&old.arguments, "succession").map_err(ceremony_error)?,
        device_did: required_string(&old.arguments, "deviceDid").map_err(ceremony_error)?,
        device_delegation_hex: required_string(&old.arguments, "deviceDelegation")
            .map_err(ceremony_error)?,
    };
    rotate_account(&store, &account, &request)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}
```

Check `handlers.rs` for whether `Store` needs importing for
`account_by_root` (`use crate::store::Store;`) — match how
`handlers/devices.rs` does it.

- [ ] **Step 2: Register routes in `lib.rs`**

In the wasm router, after the `/accounts` pair:

```rust
        .options_async("/accounts/rotate", handlers::rotate::handle_options)
        .post_async("/accounts/rotate", handlers::rotate::handle)
```

(Match the exact `options_async` registration style used for `/accounts` —
read the neighboring lines and mirror them.)

- [ ] **Step 3: Extend the HTTP integration test**

In `rust/tonk-account-service/tests/service.rs`, add a test (reusing that
file's server/client helpers — read `it_drives_the_full_ceremony_over_http`
and copy its setup verbatim) that: creates an account, calls
`tonk_identity::ceremony::rotate_account(...)` to build the containers,
POSTs `/accounts/rotate`, expects 200, then asserts a `/devices/list`
device-signed invocation under the NEW root succeeds and under the OLD
root gets 401. Note: the native binary serves only `/` and `/health` —
this test file drives the axum-free native router; if `/accounts/rotate`
is not reachable natively (wasm-only route registration), register it in
the native test router the same way the existing ceremony routes are
(follow whatever mechanism `tests/service.rs` already uses to reach
`/accounts`; it demonstrably does, since the full-ceremony test passes).

- [ ] **Step 4: Run everything**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-account-service/src/handlers/rotate.rs rust/tonk-account-service/src/handlers.rs rust/tonk-account-service/src/lib.rs rust/tonk-account-service/tests/service.rs
git commit -m "feat(tonk-account-service): rotate an account onto a new root"
```

### Task 6: Worker relink + convergence sweep

**Files:**
- Modify: `rust/tonk-worker-api/src/lib.rs` (find `AccountLinkRequest`; add field)
- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-worker/src/router/migrate.rs`

**Interfaces:**
- Consumes: `persist_link`, `account_link`, `migrate_space_roster`, `reanchor_space`, `profile_space_keys` — all existing.
- Produces:
  - `AccountLinkRequest.succession_hex: Option<String>` (serde `#[serde(default)]`, camelCase per that crate's convention — check the struct's existing rename attributes and match).
  - `pub(crate) async fn rekey_space_roster(tonk: &TonkState, key: &str, from: &dialog_varsig::Did, to: &dialog_varsig::Did) -> Result<bool, RepositoryError>` — generalization of `migrate_space_roster`.
  - `pub(crate) async fn converge_after_rotation(tonk: &TonkState, old_root: &dialog_varsig::Did)` — sweep: rekey old→new + re-anchor + back up, fire-and-forget from the link handler when a succession replaced the root.

- [ ] **Step 1: Generalize the re-key**

In `migrate.rs`, rename `migrate_space_roster`'s body into
`rekey_space_roster(tonk, key, from, to)`: replace the two derived DIDs —
`let member = account::member_did(tonk).await;` becomes parameter `to`,
`let device = tonk.profile.did();` becomes parameter `from` — and the
early-return guard `if member == device` becomes `if from == to`. Every
subsequent use of `device`/`member` in that function becomes `from`/`to`
(`Membership::new(from.clone(), …)` / `Membership::new(to.clone(), …)`,
etc.). Then reintroduce the old entry point as a thin wrapper so all
existing tests and `migrate_rosters` compile unchanged:

```rust
/// Re-key one space's roster from the device DID to the account root DID,
/// atomically. Returns `Ok(true)` when a device-keyed row was migrated,
/// `Ok(false)` when the space is already root-keyed, the profile isn't a
/// member, or the profile is unlinked.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn migrate_space_roster(
    tonk: &TonkState,
    key: &str,
) -> Result<bool, RepositoryError> {
    let to = account::member_did(tonk).await;
    let from = tonk.profile.did();
    rekey_space_roster(tonk, key, &from, &to).await
}
```

- [ ] **Step 2: Add the rotation sweep**

Below `migrate_rosters` (same in-flight guard — rotation and link never
race in practice, but the guard is free):

```rust
/// Converge every space keyed on a superseded root onto the current one.
/// Runs after a succession replaced this profile's account root:
/// re-keys rosters `old → new` and re-anchors + backs up each space so
/// devices linked under the new root can restore it.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn converge_after_rotation(tonk: &TonkState, old_root: &dialog_varsig::Did) {
    let Some(new_root) = account::account_root_did(tonk).await else {
        return;
    };
    if MIGRATE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    for key in crate::router::profile_name::profile_space_keys(tonk).await {
        match rekey_space_roster(tonk, &key, old_root, &new_root).await {
            Ok(_) => reanchor_space(tonk, &key).await,
            Err(error) => log!("rotation convergence for space '{key}' skipped: {error}"),
        }
    }
    MIGRATE_IN_FLIGHT.store(false, Ordering::SeqCst);
}
```

(`reanchor_space` runs on `Ok(false)` too, unlike the link sweep: a space
claimed under the old root after this device linked has no roster row to
re-key but still needs its chain re-anchored to the new root.)

- [ ] **Step 3: Teach `persist_link` succession-authorized replacement**

In `account.rs`: add `succession_hex: Option<String>` to
`AccountLinkRequest` in `rust/tonk-worker-api` (with `#[serde(default)]`
and the struct's existing casing attribute so old callers need no change).
In `persist_link`, replace the conflict arm:

```rust
    if let Some(existing) = load_link(state).await? {
        let existing = DelegationChain::try_from(existing.as_slice()).map_err(|error| {
            TonkWorkerError::Internal(format!("stored account delegation is invalid: {error}"))
        })?;
        if existing.issuer() != chain.issuer() {
            let Some(succession_hex) = &request.succession_hex else {
                return Err(TonkWorkerError::Conflict(
                    "profile is already linked to another account root".to_string(),
                ));
            };
            let succession = validate_succession(succession_hex, existing.issuer(), chain.issuer())
                .await?;
            state
                .profile
                .access()
                .save(UcanDelegation(succession))
                .perform(&state.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to save succession delegation: {error}"
                    ))
                })?;
        }
    }
```

with the validator mirroring `validate_link`'s checks (one proof,
subject-open, issuer = the currently stored root, audience = the new
root, valid signature):

```rust
async fn validate_succession(
    succession_hex: &str,
    old_root: &dialog_varsig::Did,
    new_root: &dialog_varsig::Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let bytes = hex::decode(succession_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid succession hex: {error}")))?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("invalid succession chain: {error}")))?;
    if chain.proof_cids().len() != 1 {
        return Err(TonkWorkerError::Router(
            "succession must contain exactly one proof".to_string(),
        ));
    }
    if chain.issuer() != old_root {
        return Err(TonkWorkerError::Forbidden(
            "succession issuer is not the linked account root".to_string(),
        ));
    }
    if chain.audience() != new_root {
        return Err(TonkWorkerError::Forbidden(
            "succession audience is not the new account root".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(TonkWorkerError::Router(
            "succession must be subject-open".to_string(),
        ));
    }
    let proof = chain
        .proofs()
        .next()
        .expect("a one-proof chain contains one proof");
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid succession signature: {error}"))
        })?;
    Ok(chain)
}
```

In the `link` handler's wasm dispatch block, capture the pre-link root
first and run the rotation sweep when it changed:

```rust
    let previous_root = account_root_did(&state).await;
    persist_link(&state, &request).await?;
```

and inside the spawned task, before `migrate_rosters`:

```rust
            if let Some(old_root) = previous_root.filter(|old| old.to_string() != request.root_did)
            {
                let tonk = app_state.write().await;
                crate::router::migrate::converge_after_rotation(&tonk, &old_root).await;
            }
```

(clone `previous_root` into the task alongside the existing captures; the
non-rotation link path is unchanged — `previous_root` is either `None` or
equals the incoming root, so the filter drops it).

- [ ] **Step 4: wasm tests (compile locally, execute in CI)**

In `account.rs` tests add:

```rust
    #[dialog_common::test]
    async fn it_replaces_the_root_when_a_succession_authorizes_it() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let _ = link(State(state.clone()), Json(first.clone())).await.unwrap();

        let old_root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&[8u8; 32])
            .await
            .unwrap();
        let succession =
            tonk_identity::delegation::mint_root_succession(old_root, &new_root.did())
                .await
                .unwrap();
        let mut second = request_for(&[8u8; 32], device_did).await;
        second.succession_hex = Some(hex::encode(succession.to_bytes().unwrap()));

        let Json(status) = link(State(state.clone()), Json(second.clone())).await.unwrap();
        match status {
            AccountStatus::Linked { root_did, .. } => assert_eq!(root_did, second.root_did),
            AccountStatus::Unlinked { .. } => panic!("relink did not persist"),
        }
    }

    #[dialog_common::test]
    async fn it_rejects_a_succession_from_the_wrong_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let _ = link(State(state.clone()), Json(first)).await.unwrap();

        // Succession issued by an unrelated key, not the linked root.
        let stranger = tonk_identity::derive::derive_root_signer(&[13u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&[8u8; 32])
            .await
            .unwrap();
        let succession =
            tonk_identity::delegation::mint_root_succession(stranger, &new_root.did())
                .await
                .unwrap();
        let mut second = request_for(&[8u8; 32], device_did).await;
        second.succession_hex = Some(hex::encode(succession.to_bytes().unwrap()));

        assert!(matches!(
            link(State(state), Json(second)).await,
            Err(TonkWorkerError::Forbidden(_))
        ));
    }
```

(`request_for` needs `succession_hex: None` added to its struct literal.)
Also add a `rekey_space_roster` test in `migrate.rs` mirroring
`it_rekeys_a_device_membership_onto_the_account_root` but seeding an
old-root-keyed membership and re-keying `old → new` between two account
roots (seed with `link_account`'s pattern for the from-DID, assert rows
land on the to-DID).

- [ ] **Step 5: Compile both targets, run native lint gate**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean. (wasm tests execute in the CI web leg — note it in the PR.)

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker-api rust/tonk-worker/src/router/account.rs rust/tonk-worker/src/router/migrate.rs
git commit -m "feat(tonk-worker): succession-authorized relink and rotation convergence"
```

### Task 7: Browser rotation ceremony (binding + panel)

**Files:**
- Modify: `rust/tonk-identity/src/install.rs`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/account.html`
- Modify: `rust/tonk-ui/src/account.css` (only if the new panel needs a class that doesn't exist)

**Interfaces:**
- Consumes: `tonk_identity::ceremony::rotate_account` (Task 4), service `POST /accounts/rotate` (Task 5), worker `POST /api/account/link` with `succession_hex` (Task 6).
- Produces: `window.tonkIdentity.rotateAccount(newPasskeyName)` returning `{ oldRootDid, newRootDid, newCredentialId, successionHex, deviceDelegationHex, rotationHex, confirmationHex }`; a `rotate` panel on `<tonk-account>`.

- [ ] **Step 1: STOP-and-verify passkey ordering**

Read `rust/tonk-identity/src/passkey.rs` in full. The rotation closure
must derive the OLD root before the new passkey exists, because
`prf_output()` performs a `get()` and the browser's credential picker will
offer every resident credential for the RP — after the new passkey is
created there are two. Confirm:
(a) `prf_output()` lets the user pick a credential (acceptable: the
    ceremony derives old first, when only the old exists), and
(b) `create_passkey` + follow-up `get()` (the `prf_output` fallback in
    `install.rs` line ~100) can target the JUST-created credential
    specifically (e.g. via `allowCredentials` with the returned id). If it
    cannot — if the fallback `get()` is also picker-ambiguous — STOP and
    report: the rotation closure then needs an `allowCredentials`
    parameter added to `prf_output` first, and that change must be
    designed against `passkey.rs` as it actually is.

- [ ] **Step 2: Add the `rotateAccount` binding**

In `install.rs`, following the existing closure pattern for
`createAccount` (read lines ~85–125 for the exact `Reflect::set` /
`Closure` idiom and error helper), add a closure registered as
`"rotateAccount"` that:

1. `let old_prf = crate::passkey::prf_output().await?` (old credential —
   the only one that exists yet);
2. `let old_root = crate::derive::derive_root_signer(&old_prf).await?`;
3. `let created = crate::passkey::create_passkey(&name).await?`; derive
   the new root from `created.prf_output` or the follow-up `get()` exactly
   as the `createAccount` closure does;
4. read the device DID the same way the existing closures obtain it
   (whatever source `createAccount` uses — mirror it);
5. `let ceremony = crate::ceremony::rotate_account(old_root, new_root, created.credential_id, device_did).await?`;
6. `Reflect::set` the seven fields named in **Interfaces** onto the result
   object (camelCase), plus `prfAtCreate` like the others.

Register `"rotateAccount"` in the install list at the bottom of the file
(the array at line ~226).

- [ ] **Step 3: Add the API wrapper**

In `rust/tonk-ui/src/api.rs`, next to `submit_account_ceremony` (read it
and mirror its reqwest/fetch idiom exactly), add:

```rust
pub async fn rotate_account(
    service_url: &str,
    rotation_hex: &str,
    confirmation_hex: &str,
) -> Result<(), ApiError>
```

which POSTs `{service_url}/accounts/rotate` with JSON
`{"rotation": rotation_hex, "confirmation": confirmation_hex}` and treats
non-2xx like the sibling fns do. Also extend the existing
`save_account_link` wrapper (which POSTs `/api/account/link`) with an
optional `succession_hex: Option<&str>` parameter serialized into the
body's new field.

- [ ] **Step 4: Add the panel**

In `account.html`: a `#account-rotate` panel with a passkey-name input
(`#account-rotate-name`), a submit button (`#account-rotate-submit`), and
one paragraph of copy: "Creates a new passkey for this account. Your
devices keep working and re-connect to the new passkey the next time they
sync in." Reachable via a "Rotate passkey" button (`#account-rotate-open`)
added to the `success` panel. In `account.rs`:

1. add `("rotate", "#account-rotate")` to the `set_mode` list;
2. add `#account-rotate-submit` to `set_busy`'s selector list;
3. wire `#account-rotate-open` → `set_mode(&host, "rotate")`;
4. wire submit: `identity_call("rotateAccount", &name)` → deserialize the
   seven-field object → `crate::api::rotate_account(&service_url, &rotation_hex, &confirmation_hex)` →
   `crate::api::save_account_link(&new_root_did, &device_delegation_hex, Some(&succession_hex))` →
   `show_success(&host)`; any error → `set_busy(false, "")` +
   `show_error`. Follow the exact `spawn_local` + error-string plumbing of
   the existing create flow (lines ~440–480).

- [ ] **Step 5: wasm panel test (compile locally)**

In `account.rs`'s `run_in_browser` test module, mirror the existing
panel-visibility test style: assert `set_mode(host, "rotate")` hides the
others and reveals `#account-rotate`.

- [ ] **Step 6: Compile gates**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown --tests && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-identity/src/install.rs rust/tonk-ui/src/api.rs rust/tonk-ui/src/account.rs rust/tonk-ui/src/account.html rust/tonk-ui/src/account.css
git commit -m "feat(tonk-ui): passkey rotation ceremony"
```

---

## Sub-stage 2 — Surviving-device recovery (one PR)

### Task 8: Recovery core + `POST /accounts/recover`

**Files:**
- Create: `rust/tonk-account-service/src/core/recovery.rs`
- Create: `rust/tonk-account-service/src/handlers/recover.rs`
- Modify: `rust/tonk-account-service/src/core.rs`, `src/handlers.rs`, `src/lib.rs`
- Modify: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- Consumes: `authorize` (device container — subject is the CURRENT root, issuer an ACTIVE device), `authorize_root` (confirmation container), `check_subject_open_delegation`, `rotate_root`, `update_device_delegation`.
- Produces: `POST /accounts/recover`, JSON `{ "recovery": "<hex device-signed>", "confirmation": "<hex new-root-signed>" }`. Device container: command `["account","recover"]`, args `newRootDid`, `newCredentialId`, `deviceDelegation` (newRoot → survivingDevice). Confirmation: command `["account","recover","confirm"]`, args `oldRootDid`. Core: `pub async fn recover_account<S: Store>(store: &S, caller: &crate::auth::Caller, new_root_did: &str, new_credential_id: &str, device_delegation_hex: &str) -> Result<(), CeremonyError>`.

- [ ] **Step 1: Write the failing core tests**

`core/recovery.rs` tests build the device container with the same
`container(...)` helper shape as `auth.rs`'s test module (root PRF
`[7u8; 32]`, device seed `[8u8; 32]`), seed the account + active device,
then:

```rust
    #[dialog_common::test]
    async fn it_flips_the_root_under_device_and_new_root_authority() { /* asserts:
        account_by_root(new).id == old id, credential replaced, surviving
        device delegation_cid repointed, device still Active */ }

    #[dialog_common::test]
    async fn it_rejects_a_device_delegation_not_issued_by_the_new_root() { /* bogus
        deviceDelegation minted by a third key → CeremonyError::Invalid,
        account row untouched */ }

    #[dialog_common::test]
    async fn it_rejects_a_revoked_surviving_device() { /* authorize() itself
        rejects — cover via the handler-level integration test in
        tests/service.rs; here assert recover_account is never reached by
        constructing the Caller manually is NOT possible (Caller fields are
        pub) — so instead assert the core succeeds only for the device the
        Caller names: pass a Caller whose device row was revoked after
        construction and expect the update_device_delegation repoint to
        return false → CeremonyError::Invalid */ }
```

Write these three tests out fully in the file — the comments above are
their assertions, the bodies follow Task 3's fixture idiom line for line
(create store → seed account/device → build request → call → assert).

- [ ] **Step 2: Implement the core**

```rust
//! Surviving-device recovery: a linked device plus a freshly created
//! passkey re-anchor the account when the old passkey is gone.

use crate::auth::Caller;
use crate::core::CeremonyError;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::Store;

/// Flip `caller.account` onto `new_root_did` under the authority of one
/// of its active devices plus proof of control of the new root.
///
/// The surviving device's row is repointed at its fresh
/// `newRoot → device` delegation so it can keep making device-signed
/// calls; every other device keeps its old-root delegation (still valid
/// for space access) and re-links on its next ceremony. The old passkey
/// credential is superseded: the registry no longer honors the root it
/// derives.
pub async fn recover_account<S: Store>(
    store: &S,
    caller: &Caller,
    new_root_did: &str,
    new_credential_id: &str,
    device_delegation_hex: &str,
) -> Result<(), CeremonyError> {
    let delegation_cid = check_subject_open_delegation(
        device_delegation_hex,
        new_root_did,
        &caller.device.device_did,
    )
    .await?;
    let repointed = store
        .update_device_delegation(caller.account.id, &caller.device.device_did, &delegation_cid)
        .await?;
    if !repointed {
        return Err(CeremonyError::Invalid(
            "surviving device is not registered under this account".to_string(),
        ));
    }
    store
        .rotate_root(caller.account.id, new_root_did, new_credential_id)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Handler**

`handlers/recover.rs` is Task 5's handler with these differences: the
first container goes through `authorize(&store, &recovery_bytes, &["account", "recover"])`
(device-signed — note `authorize` needs the store, so `build_store` runs
before authorization here), the second through
`authorize_root(&confirmation_bytes, &["account", "recover", "confirm"])`;
cross-check `required_string(&caller.arguments, "newRootDid") == confirm.root_did`
and `required_string(&confirm.arguments, "oldRootDid") == caller.account.root_did`;
then call `recover_account`. Routes:

```rust
        .options_async("/accounts/recover", handlers::recover::handle_options)
        .post_async("/accounts/recover", handlers::recover::handle)
```

- [ ] **Step 4: Integration test**

In `tests/service.rs`: full flow — create account (device A), link device
B, "lose" the passkey, build recovery via the Task 9 builder signed by
device B + a fresh root, POST `/accounts/recover`, expect 200; then
device-B-signed `/devices/list` under the new root succeeds; a
device-signed call under the old root gets 401; device A's row still
lists as `active` with its old delegation CID.

- [ ] **Step 5: Run + commit**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: PASS

```bash
git add rust/tonk-account-service/src/core/recovery.rs rust/tonk-account-service/src/handlers/recover.rs rust/tonk-account-service/src/core.rs rust/tonk-account-service/src/handlers.rs rust/tonk-account-service/src/lib.rs rust/tonk-account-service/tests/service.rs
git commit -m "feat(tonk-account-service): surviving-device account recovery"
```

### Task 9: Recovery ceremony builder + binding

**Files:**
- Modify: `rust/tonk-identity/src/ceremony.rs`
- Modify: `rust/tonk-identity/src/request.rs`
- Modify: `rust/tonk-identity/src/install.rs`

**Interfaces:**
- Produces:
  - `pub struct RecoveryCeremony { pub new_root_did: String, pub new_credential_id: String, pub device_delegation_hex: String, pub confirmation_hex: String }` and `pub async fn recover_account(new_root: Ed25519Signer, new_credential_id: String, old_root_did: String, device_did: dialog_varsig::Did) -> Result<RecoveryCeremony>` in `ceremony.rs` — everything the NEW root signs. (The device-signed container is built worker-side, where the device key lives.)
  - `pub async fn build_recovery_invocation(device: Ed25519Signer, link: &DelegationChain, new_root_did: String, new_credential_id: String, device_delegation_hex: String) -> Result<Vec<u8>>` in `request.rs` — a thin wrapper over `build_device_invocation` with command `["account","recover"]` and the three args; write it as a real fn (it exists so the worker route and tests share one arg-name source of truth).
  - `window.tonkIdentity.recoverAccount(passkeyName, oldRootDid, deviceDid)` binding returning `{ newRootDid, newCredentialId, deviceDelegationHex, confirmationHex }`.

- [ ] **Step 1: Tests then implementation for both fns**

Follow Task 4's test/impl pattern exactly: a
`it_builds_the_new_root_half_of_a_recovery` test asserting the
confirmation container verifies, is issued by the new root, command
`["account","recover","confirm"]`, args carry `oldRootDid`; and a
`it_builds_a_device_signed_recovery_invocation` test in `request.rs`
asserting the invocation verifies with proof = the link delegation,
subject = old root, command `["account","recover"]`, and the three args
present. Implementation of `recover_account`: mint
`mint_device_delegation(new_root.clone(), &device_did)`, then `build(new_root,
vec!["account".into(), "recover".into(), "confirm".into()],
strings([("oldRootDid", old_root_did)]), …)`. Implementation of
`build_recovery_invocation`:

```rust
pub async fn build_recovery_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    new_root_did: String,
    new_credential_id: String,
    device_delegation_hex: String,
) -> Result<Vec<u8>> {
    let mut arguments = BTreeMap::new();
    arguments.insert("newRootDid".to_owned(), Promised::String(new_root_did));
    arguments.insert(
        "newCredentialId".to_owned(),
        Promised::String(new_credential_id),
    );
    arguments.insert(
        "deviceDelegation".to_owned(),
        Promised::String(device_delegation_hex),
    );
    build_device_invocation(
        device,
        link,
        vec!["account".into(), "recover".into()],
        arguments,
    )
    .await
}
```

- [ ] **Step 2: Binding**

`install.rs`: `"recoverAccount"` closure — `create_passkey(name)`, derive
the new root (create-or-follow-up-get, same as `createAccount`), call
`ceremony::recover_account(new_root, credential_id, old_root_did, device_did)`
with the two DIDs passed in from JS (the page reads them from
`GET /api/account` — it knows the old root and device DID even though the
passkey is gone, because the link is stored locally). Register in the
install list.

- [ ] **Step 3: Run + commit**

Run: `cargo test -p tonk-identity`
Expected: PASS

```bash
git add rust/tonk-identity/src/ceremony.rs rust/tonk-identity/src/request.rs rust/tonk-identity/src/install.rs
git commit -m "feat(tonk-identity): surviving-device recovery ceremony builder"
```

### Task 10: Worker recovery route

**Files:**
- Modify: `rust/tonk-worker/src/router/account.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs` (request/response types)
- Modify: `rust/tonk-worker/src/router.rs` (route registration — find where `/api/account/link` is registered and mirror)

**Interfaces:**
- Consumes: `account_link` (stored old link), profile device signer (STOP gate below), `build_recovery_invocation` (Task 9), service `POST /accounts/recover` (Task 8), `converge_after_rotation` (Task 6).
- Produces: `POST /api/account/recover`, body `AccountRecoverRequest { new_root_did: String, new_credential_id: String, confirmation_hex: String, device_delegation_hex: String }` → flips the service registry, replaces the local link, converges; responds with the new `AccountStatus::Linked`.

- [ ] **Step 1: STOP-and-verify the device signer**

`build_recovery_invocation` needs the device's `Ed25519Signer`. Find how
the worker signs device invocations today: `account_backup.rs` calls
`build_device_invocation` for `/chains/put` — read `account_backup.rs` and
identify exactly where its `Ed25519Signer` comes from (a
`tonk.profile`-derived signer, a credential-store load, or a
`dialog_operator` API). Reuse that exact mechanism. If it turns out the
signer is NOT reachable from `TonkState` (i.e. `account_backup.rs` gets it
some other way you cannot reuse), STOP and report before writing the
route.

- [ ] **Step 2: Implement the route**

`recover` handler in `account.rs` (wasm-gated like the sweep dispatchers,
since it drives service HTTP + convergence):

1. load the stored link; `Unlinked` → 404-equivalent
   (`TonkWorkerError::Router("profile has no account link")`).
2. `old_root = link.issuer()`; build the device container via
   `build_recovery_invocation` (Task 9) with the request fields.
3. POST both containers to `{service}/accounts/recover` (service URL
   resolution: reuse the exact helper `account_backup.rs` uses).
   Non-2xx → surface the service error text as `TonkWorkerError::Forbidden`.
4. On 200: validate + store the new link — call `persist_link_replacing`,
   a new `pub(crate)` fn that is `persist_link` minus the same-issuer
   guard (factor the shared tail out; the HTTP `link` route keeps the
   guarded path, only the recovery route may call the replacing variant).
5. Fire the convergence sweep exactly as the rotation arm of `link` does
   (`converge_after_rotation(&tonk, &old_root)` then `restore_spaces`).
6. Respond `AccountStatus::Linked { root_did: new, device_did }`.

- [ ] **Step 3: wasm test (compile locally)**

`it_refuses_recovery_when_unlinked`: call the handler on a fresh state,
expect `Err(TonkWorkerError::Router(_))`. (The happy path needs a live
account service — it is covered natively by Task 8's integration test and
by the manual staging pass; say so in a comment above the test module.)

- [ ] **Step 4: Compile gates + commit**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean

```bash
git add rust/tonk-worker/src/router/account.rs rust/tonk-worker/src/router.rs rust/tonk-worker-api
git commit -m "feat(tonk-worker): surviving-device account recovery route"
```

### Task 11: Recovery panel

**Files:**
- Modify: `rust/tonk-ui/src/account.rs`, `account.html`
- Modify: `rust/tonk-ui/src/api.rs`

**Interfaces:**
- Consumes: `window.tonkIdentity.recoverAccount` (Task 9), worker `POST /api/account/recover` (Task 10), `GET /api/account` (existing `account_status`).

- [ ] **Step 1: Panel + wiring**

`#account-recover` panel, entered from a "Lost your passkey?" link on the
`choice` panel — but ONLY shown when `account_status()` returns `Linked`
(a linked device is the prerequisite; an unlinked browser shows the
standard copy pointing at any surviving device instead — one static
paragraph in the panel handles both, toggled on `data-mode` +
link-status). Flow on submit: read `root_did` + `device_did` from
`account_status()`, `identity_call("recoverAccount", …)` with
`(name, root_did, device_did)`, POST the result to the worker recover
route via a new `api.rs` wrapper `recover_account(request: &AccountRecoverRequest)`,
then `show_success`. Copy requirement (verbatim in the HTML): "This
replaces the account's passkey. Other devices keep their data and
re-connect the next time they open tonk." Follow the create-flow error
plumbing.

- [ ] **Step 2: set_mode/set_busy registration + wasm panel test**

Add `("recover", "#account-recover")` to `set_mode`,
`#account-recover-submit` to `set_busy`, and a panel-visibility test.

- [ ] **Step 3: Compile gates + commit**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown --tests && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`

```bash
git add rust/tonk-ui/src/account.rs rust/tonk-ui/src/account.html rust/tonk-ui/src/api.rs
git commit -m "feat(tonk-ui): surviving-device recovery panel"
```

---

## Sub-stage 3 — Total-loss re-anchor (one PR)

### Task 12: Store lookup by email + bulk revoke

**Files:**
- Modify: `rust/tonk-account-service/src/store.rs`, `store/sqlite.rs`, `store/d1.rs`

**Interfaces:**
- Produces: `async fn account_by_email(&self, email: &str) -> Result<Option<Account>, StoreError>` and `async fn revoke_all_devices(&self, account_id: i64) -> Result<(), StoreError>` on `Store`.

- [ ] **Step 1: SQL + trait + tests + both impls**

```rust
/// SQL: look up an account by verified email.
pub const SELECT_ACCOUNT_BY_EMAIL: &str =
    "SELECT id, email, root_did, credential_id, created_at FROM accounts WHERE email = ?1";

/// SQL: revoke every device under an account.
pub const UPDATE_DEVICES_REVOKE_ALL: &str =
    "UPDATE devices SET status = 'revoked' WHERE account_id = ?1";
```

sqlite `account_by_email` mirrors `account_by_root`'s
`query_row(...).optional()` body; `revoke_all_devices` mirrors
`revoke_device` without the bool. D1 likewise. Tests:
`it_finds_an_account_by_its_lowercased_email` (create with mixed-case via
the core path is already lowercased; store-level test seeds lowercase and
asserts the miss on a different email) and
`it_revokes_every_device_at_once` (seed two devices, revoke all, list
shows both `revoked`).

- [ ] **Step 2: Run + commit**

Run: `cargo test -p tonk-account-service --features helpers`

```bash
git add rust/tonk-account-service/src/store.rs rust/tonk-account-service/src/store/sqlite.rs rust/tonk-account-service/src/store/d1.rs
git commit -m "feat(tonk-account-service): account lookup by email and bulk device revoke"
```

### Task 13: Re-anchor core + `POST /accounts/reanchor`

**Files:**
- Create: `rust/tonk-account-service/src/core/reanchor.rs`
- Create: `rust/tonk-account-service/src/handlers/reanchor.rs`
- Modify: `core.rs`, `handlers.rs`, `lib.rs`, `tests/service.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs` + `install.rs` (confirm builder + binding)

**Interfaces:**
- Produces: `POST /accounts/reanchor`, JSON `{ "email": …, "code": …, "confirmation": "<hex new-root-signed>" }`; confirmation command `["account","reanchor","confirm"]`, args `email`, `newCredentialId`. Core: `pub async fn reanchor_account<S: Store>(store: &S, email: &str, code: &str, new_root_did: &str, new_credential_id: &str, now: u64) -> Result<(), CeremonyError>`.

- [ ] **Step 1: Core with tests**

Core sequence: `verify_code(store, email, code, now)?` (consumes the code
— the existing cooldown + attempt cap ARE the rate limit) →
`account_by_email(email)?` else `Unauthorized("unknown account")` →
`rotate_root(account.id, new_root_did, new_credential_id)` →
`revoke_all_devices(account.id)`. Doc comment must state: no delegation
bridges anything — space access is intentionally severed; rosters keyed on
the old root make affected spaces discoverable so founders re-invite.
Tests: `it_reanchors_with_a_valid_code_and_revokes_every_device`,
`it_rejects_a_bad_code_before_touching_the_registry`,
`it_rejects_an_unknown_email` — all three follow Task 3's fixture idiom.

- [ ] **Step 2: Handler**

Follows Task 5's shape: JSON body, `authorize_root(&confirmation_bytes,
&["account", "reanchor", "confirm"])`, cross-check the container's `email`
arg equals the body email (binds the code to the root that requested it),
`new_credential_id` from the container args, `worker::console_log!` one
structured line on success (`"account reanchored"`, account id, old root,
new root — the "logged" requirement). Ceremony builder + binding: a
`reanchor_account(new_root, email, new_credential_id)` fn in `ceremony.rs`
(single container via `build(...)` with command
`["account","reanchor","confirm"]` and args `email`, `newCredentialId` —
mirror Task 4's test) and a `"reanchorAccount"` closure in `install.rs`
(create passkey → derive → build). Integration test in `tests/service.rs`:
request code → reanchor → old root 401s, new root's first `linkDevice`
self-link succeeds, `/devices/list` shows the pre-loss devices revoked.

- [ ] **Step 3: Run + commit**

Run: `cargo test -p tonk-account-service --features helpers && cargo test -p tonk-identity`

```bash
git add rust/tonk-account-service/src/core/reanchor.rs rust/tonk-account-service/src/handlers/reanchor.rs rust/tonk-account-service/src/core.rs rust/tonk-account-service/src/handlers.rs rust/tonk-account-service/src/lib.rs rust/tonk-account-service/tests/service.rs rust/tonk-identity/src/ceremony.rs rust/tonk-identity/src/install.rs
git commit -m "feat(tonk-account-service): email-authorized total-loss re-anchor"
```

### Task 14: Re-anchor panel (loud copy)

**Files:**
- Modify: `rust/tonk-ui/src/account.rs`, `account.html`, `api.rs`

- [ ] **Step 1: Panel**

`#account-reanchor`, reached from the recovery panel's "No device either?"
link. Two-step inside the panel (email+send-code → code+passkey-name+submit),
reusing `request_account_code`. REQUIRED copy, verbatim, above the submit:
"This creates a fresh identity for your account email. Your spaces do NOT
carry over — every space must re-invite you. Only continue if no signed-in
device exists anywhere." Submit: `identity_call("reanchorAccount", …)` →
new `api.rs` wrapper `reanchor_account(service_url, email, code, confirmation_hex)` →
on success, run the standard self-link (`linkDevice` ceremony + `/devices/link`
+ `save_account_link`) so the browser ends the flow linked — then
`show_success`. Panel registration + busy list + visibility test as before.

- [ ] **Step 2: Compile gates + commit**

Run: `cargo check -p tonk-ui --target wasm32-unknown-unknown --tests && cargo clippy --workspace --all-targets --all-features && cargo fmt --check`

```bash
git add rust/tonk-ui/src/account.rs rust/tonk-ui/src/account.html rust/tonk-ui/src/api.rs
git commit -m "feat(tonk-ui): total-loss re-anchor panel"
```

### Task 15: CDP e2e + staging verification notes

**Files:**
- Modify: `rust/tonk-ui/src/identity.rs`

- [ ] **Step 1: Rotation e2e against the virtual authenticator**

Extend the existing `web-integration-tests` harness (read
`it_builds_a_root_signed_account_creation_in_one_browser_ceremony` for the
`thirtyfour` + `WebAuthn.addVirtualAuthenticator` setup): one new test
`it_rotates_onto_a_second_virtual_credential` — create passkey A, derive
root A; call `rotateAccount` (the virtual authenticator auto-approves, so
the two-credential picker ambiguity from Task 7's STOP gate does not bite
here — note that in the test's doc comment); assert the returned
`oldRootDid` == root A's DID, `newRootDid` differs, and both containers +
the succession chain verify (decode with `InvocationChain::try_from` /
`DelegationChain::try_from` in the test body, same as the existing test
does).

- [ ] **Step 2: Manual staging checklist into the PR body**

The sub-stage PRs each carry (not in code, in the PR body): rotate on
staging.tonk.xyz with a real passkey → old device still opens its spaces →
second browser self-links with the new passkey and restores. Recovery and
re-anchor equivalents for their PRs.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-ui/src/identity.rs
git commit -m "test(tonk-ui): passkey rotation against the virtual authenticator"
```

---

## Self-review notes

- Spec coverage: master-design "Recovery and rotation" section — deliberate
  rotation (Tasks 1–7), surviving-device (Tasks 8–11), total-loss
  (Tasks 12–14), "no escrow / no new cryptographic artifacts" honored (the
  only new artifact classes are a succession delegation and standard
  invocation containers). Registry flip + credential replacement covers
  "the old credential is revoked".
- Type consistency: `check_subject_open_delegation(hex, issuer, audience)`
  used in Tasks 3, 8; `rotate_root` / `update_device_delegation`
  signatures identical in Tasks 2, 3, 8, 13; wire field names
  (`newRootDid`, `newCredentialId`, `succession`, `deviceDid`,
  `deviceDelegation`, `oldRootDid`) consistent between the ceremony
  builders (Tasks 4, 9, 13) and the handlers (Tasks 5, 8, 13).
- Known execution gates: Task 7 Step 1 (passkey picker ambiguity /
  `allowCredentials`), Task 10 Step 1 (worker device-signer source), Task
  5 Step 3 (native reachability of new routes in `tests/service.rs`),
  Task 2 Step 4 (sqlite `map_err` Conflict mapping on UPDATE). Each is a
  read-and-confirm against code that exists on this branch, with a STOP
  instruction when the read contradicts the plan.
