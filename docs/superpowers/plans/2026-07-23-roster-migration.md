# Roster Migration (stage 3B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a device links to an account, converge its existing device-keyed spaces onto the root DID — re-key each roster row (retract device, re-stamp root, first-wins preserved), re-anchor the capability chain to the root, back up the re-anchored chain, and fix the profile-rename no-op.

**Architecture:** A wasm-only migration sweep over `profile_space_keys` runs fire-and-forget on link, beside `restore_spaces`. Per space: an atomic content-branch transaction asserts the root-keyed roster rows and retracts the device-keyed ones; then (PR 2) a `try_access()`-discriminated re-anchor mints `space -> root` (owned) or `space -> eph -> device -> root` (claimed) and backs it up. Rename is fixed to key on the resolved member DID and retract the orphaned device name.

**Tech Stack:** Rust (edition 2024). `dialog-*` pinned `tonk-2026-07-17`. Reuses 3A (`account::{member_did, account_root_did, account_link}`, the `/chains/put` backup client) and the restore branch (`back_up_owned_space`).

## Global Constraints

- Branch `feat/roster-migration` (cut from `feat/cross-device-restore`; carries the design doc). Rebase onto `staging` as 3A/restore land.
- Do NOT bump the pinned dialog tag `tonk-2026-07-17`.
- `Subject::Any` is only the `root -> device` link. Space delegations (`space -> root`, `space -> eph -> device -> root`) are subject-SPECIFIC. Never `Subject::Any` for a space.
- Migration is BEST-EFFORT and FAIL-OPEN: it never fails a link or blocks local work; one space's failure is logged and skipped. Fire-and-forget so a slow account service can't stall link.
- Re-key is ATOMIC per space: one transaction asserts root rows + retracts device rows. First-wins stamps (`MemberRole`, `InvitedVia`) are copied from the device row's read values onto the new entity — never assumed to carry over.
- Idempotent: a space with no device-keyed `Membership` row is skipped; re-running is safe.
- Migration is wasm-only (reuses `profile_space_keys`, `#[cfg(target_arch = "wasm32")]`).
- Tests `#[dialog_common::test]`; `tonk-worker` wasm mods carry `wasm_bindgen_test_configure!(run_in_service_worker)`. No `mod.rs`. Conventional Commits, no emojis. Self-contained comments (no "3b"/spec references).
- **Wasm tests HANG in the sandbox** — verify wasm-gated code by COMPILING (`cargo clippy -p tonk-worker --all-targets` + `cargo build -p tonk-worker --target wasm32-unknown-unknown`), never `test:web:debug`. Native tests run normally. CI's `web` leg executes the wasm tests.
- Lint gate before each PR: `cargo clippy --workspace --all-targets --all-features` + `cargo fmt --check`.

## File Structure

- `rust/tonk-worker/src/router/migrate.rs` — **new**: `migrate_space_roster` (facts), `migrate_rosters` (sweep + guard), and (PR 2) `reanchor_space`.
- `rust/tonk-worker/src/router.rs` — declare `pub(crate) mod migrate;`.
- `rust/tonk-worker/src/router/account.rs` — link handler also fires `migrate_rosters`.
- `rust/tonk-worker/src/router/profile_name.rs` — `restamp_member_name` keys on the member DID + retracts the device name.

---

## PR 1 — Facts: roster re-key + rename (Tasks 1-3)

### Task 1: `migrate_space_roster` — atomic per-space re-key

**Files:**
- Create: `rust/tonk-worker/src/router/migrate.rs`
- Modify: `rust/tonk-worker/src/router.rs` (`pub(crate) mod migrate;`)

**Interfaces:**
- Consumes: `crate::router::account::member_did`, the reactor content-branch acquire/transaction pattern, `tonk_schema::{Membership, MemberRole, MemberName, InvitedVia}`, `Query`/`Term`.
- Produces: `pub(crate) async fn migrate_space_roster(tonk: &TonkState, key: &str) -> Result<bool, RepositoryError>` — returns `true` if it migrated a device-keyed row, `false` if the space was already root-keyed / not a member / unlinked. Consumed by Task 2 and (PR 2) Task 4.

- [ ] **Step 1: Write the module + the re-key function**

Create `migrate.rs` (wasm-only — it reuses wasm-only enumeration and mirrors `restamp_member_name`'s reactor pattern at `profile_name.rs:217`):

```rust
//! Converge a linked device's existing device-keyed spaces onto the
//! account root DID.

#[cfg(target_arch = "wasm32")]
use tonk_common::log;

#[cfg(target_arch = "wasm32")]
use crate::router::account;
#[cfg(target_arch = "wasm32")]
use crate::router::repository::{CONTENT_BRANCH, RepositoryError};
#[cfg(target_arch = "wasm32")]
use crate::worker::TonkState;

/// Re-key one space's roster from the device DID to the account root DID,
/// atomically. Returns `Ok(true)` when a device-keyed row was migrated,
/// `Ok(false)` when the space is already root-keyed, the profile isn't a
/// member, or the profile is unlinked.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn migrate_space_roster(
    tonk: &TonkState,
    key: &str,
) -> Result<bool, RepositoryError> {
    use tonk_schema::{InvitedVia, MemberName, MemberRole, Membership};
    use tonk_schema::prelude::*;
    use dialog_reactor::query::{Query, Term};

    let member = account::member_did(tonk).await;
    let device = tonk.profile.did();
    // Unlinked: no root to migrate to. (member_did == device DID.)
    if member == device {
        return Ok(false);
    }

    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("acquire content '{key}': {e}")))?;
    let subject = session.handle().of().clone();

    let device_membership = Membership::new(device.clone(), subject.clone());
    let device_entity = device_membership.this().clone();

    // Is there a device-keyed row to migrate?
    let memberships: Vec<Membership> = session
        .handle()
        .query()
        .select(Query::<Membership> {
            this: Term::var("this"),
            subject: Term::from(subject.this()),
            member: Term::var("member"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("membership query '{key}': {e:?}")))?;
    let Some(device_row) = memberships.iter().find(|m| m.this == device_entity).cloned() else {
        return Ok(false);
    };

    // Read the device row's stamps so they can be copied and retracted.
    let roles: Vec<MemberRole> = session
        .handle()
        .query()
        .select(Query::<MemberRole> { this: Term::var("this"), role: Term::var("role") })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("role query '{key}': {e:?}")))?;
    let device_role = roles.iter().find(|r| r.this == device_entity).cloned();

    let names: Vec<MemberName> = session
        .handle()
        .query()
        .select(Query::<MemberName> { this: Term::var("this"), name: Term::var("name") })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("name query '{key}': {e:?}")))?;
    let device_name = names.iter().find(|n| n.this == device_entity).cloned();

    let stamps: Vec<InvitedVia> = session
        .handle()
        .query()
        .select(Query::<InvitedVia> { this: Term::var("this"), invitation: Term::var("invitation") })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| RepositoryError::Internal(format!("invited-via query '{key}': {e:?}")))?;
    let device_stamp = stamps.iter().find(|s| s.this == device_entity).cloned();

    // Build the root-keyed rows and one atomic assert+retract transaction.
    let root_membership = Membership::new(member.clone(), subject.clone());
    let root_entity = root_membership.this().clone();

    let mut txn = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(root_membership.clone())
        .retract(device_row);

    if let Some(role) = device_role {
        let root_role = if role.role.0.to_string() == MemberRole::FOUNDER {
            MemberRole::founder(root_entity.clone())
        } else {
            MemberRole::member(root_entity.clone())
        };
        txn = txn.assert(root_role).retract(role);
    }
    if let Some(name) = device_name {
        txn = txn
            .assert(MemberName::new(root_entity.clone(), name.name.0.clone()))
            .retract(name);
    }
    if let Some(stamp) = device_stamp {
        txn = txn
            .assert(InvitedVia::new(root_entity.clone(), stamp.invitation.0.clone()))
            .retract(stamp);
    }

    txn.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("commit migration '{key}': {e}")))?;
    log!("migrated roster for space '{key}' to the account root");
    Ok(true)
}
```

Declare in `router.rs`: `pub(crate) mod migrate;`.

Confirm against `rust/tonk-schema/src/membership.rs` the exact field accessors: `role.role.0` (role URI value), `name.name.0` (String), `stamp.invitation.0` (invitation `Entity`), and that `MemberRole::FOUNDER` is the founder URI constant. Confirm `CONTENT_BRANCH` and `RepositoryError` are importable from `router::repository` (they're used by `profile_name.rs`). Adjust the `Query`/`Term` import path to match `profile_name.rs`/`join.rs` (they `use` them from the same place).

- [ ] **Step 2: Write the failing test**

Add a wasm test mod (mirrors `join.rs`'s roster tests — an account is linked, a device-keyed membership is written, migration runs, the root row is asserted and the device row gone). Use the existing test helpers (`test_state`, the account `link` handler with a `request_for`-style `root -> device`, `content_memberships`/`content_member_roles`/`content_memberships` readers from `crate::router::tests`). Assert after `migrate_space_roster`:
- a `Membership` with `member.0 == root_did.this()` exists;
- no `Membership` with `member.0 == device_did.this()`;
- the `MemberRole` (founder if the seeded row was founder) is present on the root entity;
- a second `migrate_space_roster` call returns `Ok(false)` (idempotent).

(Model the setup on `join.rs`'s `it_keys_membership_on_the_root_did_for_an_account_holder`: link an account via `crate::router::account::link`, then seed a device-keyed membership by claiming/creating a space BEFORE... simpler: seed the device row directly with a content-branch transaction asserting `Membership::new(device_did, subject)` + `MemberRole::member(...)` + `MemberName::new(...)`, then run migration.)

- [ ] **Step 3: Run compile-verify (wasm tests can't execute here)**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -40` (clean)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
Note: the test is `run_in_service_worker`; compiled here, executes in CI.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-worker/src/router/migrate.rs rust/tonk-worker/src/router.rs
git commit -m "feat(tonk-worker): re-key a space roster from device to account root"
```

---

### Task 2: The migration sweep + link trigger

**Files:**
- Modify: `rust/tonk-worker/src/router/migrate.rs`
- Modify: `rust/tonk-worker/src/router/account.rs` (link handler tail)

**Interfaces:**
- Consumes: `migrate_space_roster` (Task 1), `crate::router::profile_name::profile_space_keys` (make it `pub(crate)` — currently private at `profile_name.rs:130`), `crate::router::account::account_link`.
- Produces: `pub(crate) async fn migrate_rosters(tonk: &TonkState)` — best-effort sweep; consumed by the link handler.

- [ ] **Step 1: Add the sweep with an in-flight guard**

In `migrate.rs` (mirror `restore.rs`'s guard + best-effort loop):

```rust
#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_arch = "wasm32")]
static MIGRATE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Converge every existing device-keyed space onto the account root.
/// Best-effort: no-op when unlinked; one space's failure is logged and
/// skipped. A concurrent run is skipped (the guard), since both would
/// migrate the same spaces.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn migrate_rosters(tonk: &TonkState) {
    if account::account_link(tonk).await.is_none() {
        return; // unlinked
    }
    if MIGRATE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    for key in crate::router::profile_name::profile_space_keys(tonk).await {
        if let Err(error) = migrate_space_roster(tonk, &key).await {
            log!("roster migration for space '{key}' skipped: {error}");
        }
    }
    MIGRATE_IN_FLIGHT.store(false, Ordering::SeqCst);
}
```

Make `profile_space_keys` `pub(crate)` in `profile_name.rs:130` (change `async fn` to `pub(crate) async fn`).

- [ ] **Step 2: Fire migration on link, fire-and-forget**

In `account.rs` `link`, in the same post-persist tail where `restore_spaces` is dispatched (the `#[cfg]` wasm `spawn_local` block), add a migration dispatch beside it (a slow account service must not stall the link response). Both can share one spawned task:

```rust
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let app_state = app_state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let tonk = app_state.read().await;
            crate::router::migrate::migrate_rosters(&tonk).await;
            crate::router::restore::restore_spaces(&tonk).await;
        });
    }
```

(If `restore_spaces` is already spawned in its own block, fold both into that single spawned task in migrate-then-restore order rather than spawning twice — migration converges local spaces, restore pulls remote ones; running migration first means restore won't re-touch a space migration just re-keyed. Confirm the existing `app_state` clone from Task-6 of restore is reused, not double-moved.)

- [ ] **Step 3: Compile-verify**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -40` (clean; the `migrate_rosters`/`profile_space_keys` visibility change resolves)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
Run: `cargo fmt -p tonk-worker -- --check`

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-worker/src/router/migrate.rs rust/tonk-worker/src/router/profile_name.rs rust/tonk-worker/src/router/account.rs
git commit -m "feat(tonk-worker): sweep and migrate rosters on device link"
```

---

### Task 3: Fix the profile-rename no-op

**Files:**
- Modify: `rust/tonk-worker/src/router/profile_name.rs` (`restamp_member_name`, 217-245)

**Interfaces:**
- Consumes: `crate::router::account::member_did`.

- [ ] **Step 1: Key the rename on the member DID + retract the device name**

Replace the body of `restamp_member_name` (currently keys `Membership::new(tonk.profile.did(), repo_did)` and only asserts). Read the device-keyed `MemberName` (to retract it by full fact), assert the new name on the member-DID entity:

```rust
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("acquire content branch '{key}': {e}")))?;
    let repo_did = session.handle().of().clone();

    let member = crate::router::account::member_did(tonk).await;
    let membership = Membership::new(member.clone(), repo_did.clone());

    let mut txn = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(MemberName::new(membership.this().clone(), name.to_string()));

    // Linked profiles: a rename must also clear the orphaned device-keyed
    // name row (cardinality-one on a different entity, so the assert above
    // won't overwrite it).
    let device = tonk.profile.did();
    if member != device {
        let device_membership = Membership::new(device, repo_did);
        let device_entity = device_membership.this().clone();
        let names: Vec<MemberName> = session
            .handle()
            .query()
            .select(Query::<MemberName> { this: Term::var("this"), name: Term::var("name") })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|e| RepositoryError::Internal(format!("name query '{key}': {e:?}")))?;
        for stale in names.into_iter().filter(|n| n.this == device_entity) {
            txn = txn.retract(stale);
        }
    }

    txn.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("restamp member name for '{key}': {e}")))?;
    Ok(())
```

Add the `Query`/`Term` imports to the file if not already present (they're used by `profile_space_keys` above in the same file, so they are).

- [ ] **Step 2: Compile-verify**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -40` (clean)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
Run: `cargo fmt -p tonk-worker -- --check`

Existing rename tests in `profile_name.rs`'s test mod (if any) must still compile; for an unlinked profile `member == device`, so the behavior is the old behavior (assert on the device entity, no retract).

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-worker/src/router/profile_name.rs
git commit -m "feat(tonk-worker): rename updates the account-root roster row"
```

**End of PR 1.** Open a PR (base = `feat/cross-device-restore` while it's unmerged, else `staging`).

---

## PR 2 — Capabilities: re-anchor + backup (Tasks 4-5)

### Task 4: Re-anchor migrated spaces and back them up

**Files:**
- Modify: `rust/tonk-worker/src/router/migrate.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs` (expose a claimed re-anchor backup if needed)

**Interfaces:**
- Consumes: `back_up_owned_space` (restore branch, `account_backup.rs:330`), `tonk.profile.repository(key).load()`, `repository.try_access()`, `tonk.profile.access().claim(&repository).delegate(root_did)`, the `/chains/put` backup client.
- Produces: `pub(crate) async fn reanchor_space(tonk: &TonkState, key: &str)` — best-effort; mints and backs up the space's `... -> root` chain. Called from the sweep after a successful `migrate_space_roster`.

- [ ] **Step 1: VERIFY-FIRST — probe `remote_url` recovery**

Before writing backup, determine whether a loaded space's sync URL is cleanly recoverable from its stored remote config. Load a space (`tonk.profile.repository(key).load().perform(&operator)`), read its remote configuration, and check whether the `SiteAddress`/`UcanAddress` exposes the URL string (a `Display`, `as_str`, or `to_string` that yields the `remote=` URL that `space_config`/the invite fed to `UcanAddress::new`). This is the same recovery the `put_repository` gap needs.
- **If recoverable:** proceed to Step 2 with the recovered `remote_url`.
- **If NOT cleanly recoverable:** STOP and report. Migration ships re-key + rename (PR 1) and re-anchor *without* cross-device backup, and the backup half is a documented follow-up. Do not force a brittle extraction.

- [ ] **Step 2: Implement `reanchor_space` (both cases)**

In `migrate.rs`:

```rust
#[cfg(target_arch = "wasm32")]
pub(crate) async fn reanchor_space(tonk: &TonkState, key: &str) {
    if let Err(error) = try_reanchor_space(tonk, key).await {
        log!("re-anchor of space '{key}' skipped: {error}");
    }
}

#[cfg(target_arch = "wasm32")]
async fn try_reanchor_space(tonk: &TonkState, key: &str) -> Result<(), RepositoryError> {
    let Some(root_did) = account::account_root_did(tonk).await else {
        return Ok(());
    };
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| RepositoryError::Internal(format!("load space '{key}': {e}")))?;
    let remote_url = /* the recovered sync URL from Step 1, read off `repository`'s config */;

    match repository.try_access() {
        // Created/owned: space -> root, and back it up (reuses restore's helper).
        Some(_) => {
            crate::router::account_backup::back_up_owned_space(tonk, &repository, &remote_url).await;
        }
        // Claimed: profile re-delegates its held capability to the root,
        // composing space -> eph -> device -> root; save + back up.
        None => {
            let chain = tonk
                .profile
                .access()
                .claim(&repository)
                .delegate(root_did)
                .perform(&tonk.operator)
                .await
                .map_err(|e| RepositoryError::Internal(format!("re-anchor '{key}': {e}")))?;
            tonk.profile
                .access()
                .save(chain.clone())
                .perform(&tonk.operator)
                .await
                .map_err(|e| RepositoryError::Internal(format!("save re-anchor '{key}': {e}")))?;
            crate::router::account_backup::back_up_reanchored(tonk, chain, &remote_url).await;
        }
    }
    Ok(())
}
```

`back_up_reanchored(tonk, chain, remote_url)` is a thin addition to `account_backup.rs`: it takes the composed `DelegationChain` (extract its bytes the same way `back_up_claim`/`run_backup` does), wraps `{chain_hex, remote_url}` in a `ClaimBackup`, and dispatches the same fire-and-forget `/chains/put`. Reuse `dispatch_backup`. Confirm the type `access().claim().delegate().perform()` returns (`UcanDelegation(DelegationChain)`, per the invite-mint path) and extract the chain via `.into_chain()`/`.to_bytes()` exactly as the restore-branch backup does.

- [ ] **Step 3: Call the re-anchor from the sweep**

In `migrate_rosters`, after a successful `migrate_space_roster(tonk, &key)` returns `Ok(true)`, call `reanchor_space(tonk, &key).await` for that key (best-effort; it has its own error swallow). A space that returned `Ok(false)` (already root-keyed) was migrated on an earlier link or claimed post-account — its chain already terminates at root, so it needs no re-anchor.

- [ ] **Step 4: Compile-verify + native backup tests**

Run: `cargo clippy -p tonk-worker --all-targets 2>&1 | tail -40` (clean)
Run: `cargo build -p tonk-worker --target wasm32-unknown-unknown 2>&1 | tail -20` (clean)
Run: `cargo test -p tonk-account-service --features helpers 2>&1 | tail -15` (the backup wire contract stays green)
Run: `cargo fmt -p tonk-worker -- --check`

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/src/router/migrate.rs rust/tonk-worker/src/router/account_backup.rs
git commit -m "feat(tonk-worker): re-anchor and back up migrated spaces"
```

---

### Task 5: Full gates + PRs

**Files:** none (verification only).

- [ ] **Step 1: Workspace gate**

```bash
cargo clippy --workspace --all-targets --all-features
cargo fmt --check
cargo test -p tonk-account-service --features helpers
```
Expected: all green.

- [ ] **Step 2: Note deferred wasm + staging verification in the PR body**

The migration sweep, re-anchor, and rename fix are `run_in_service_worker`/wasm — executed by CI's `web` leg, not locally. For a human/staging pass: (a) join a space as a device-only user, then create an account/link → the roster row converges on the root DID; (b) rename → the name updates on the root row in every space; (c) a second device links → migrated spaces restore (depends on PR 2's backup + the restore feature).

- [ ] **Step 3: Push and open PR 2**

```bash
git push
gh pr create --base <PR1 base> --title "feat(account): re-anchor and back up migrated spaces (3b)" --body "<summary + verification checklist + design link>"
```

---

## Out of scope (later)

Revocation-list awareness (access-service concern, pairs with billing/access); live migration propagation (runs on link, like restore); the `put_repository` one-shot create-with-remote backup gap (tracked from the restore work).
