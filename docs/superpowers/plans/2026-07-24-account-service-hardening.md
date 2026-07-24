# Account Service Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the tracked hardening debt on the account registry: sanitize internal error text, enforce the sqlite foreign-key pragma, add the two missing negative tests, enforce expiry on device-signed invocations, and document the abuse-control runbook.

**Architecture:** All changes live in `rust/tonk-account-service` plus one client-side change in `rust/tonk-identity` (device invocations gain an expiration before the server starts requiring one). Error sanitization happens at `ceremony_error` — the single mapping shared by the wasm handlers and the native helpers server, whose docstring already reserves this follow-up. Expiry enforcement extracts the root path's existing five-minute-window check into a helper both `authorize` and `authorize_root` call.

**Tech Stack:** Rust, workers-rs (Cloudflare Worker, wasm32), rusqlite (native test store), dialog-ucan-core (`InvocationChain`, `Timestamp`), `#[dialog_common::test]`.

## Global Constraints

- Test idiom: `#[dialog_common::test]`, test names `it_does_x`, grouped by behaviour.
- No `mod.rs` files; `foo.rs` + `foo/` form.
- Conventional commits: `type(scope): subject`, imperative, lowercase, no trailing period. Scope is the crate short name (`tonk-account-service`, `tonk-identity`).
- Never reference plan stages, phases, or design docs inside code or code comments — code stands on its own.
- Lint gate: `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --check` must pass (the workspace gate compiles integration tests; per-crate clippy being green is not sufficient).
- Crate test command: `cargo test -p tonk-account-service --features helpers`.
- **Accepted decision (do not "fix"):** no nonce/replay table. Every device-authorized endpoint is an idempotent upsert or read, so replay inside the five-minute expiry window is harmless. Expiry bounds the window; a nonce cache would be YAGNI.
- **Accepted rollout note:** browsers running a stale cached service worker may send expiration-less chain-backup invocations for a while after deploy. Those calls are best-effort fire-and-forget (failures land in logs, no user-facing breakage), so enforcement does not wait for them.

---

### Task 1: Sanitize internal error text at the ceremony_error choke point

`ceremony_error` in `rust/tonk-account-service/src/handlers.rs` currently copies `CeremonyError::Internal` detail (store/R2/email library text) onto the wire. Its docstring explicitly reserves sanitization as a follow-up. Replace internal detail with a generic message and log the detail instead. The handful of direct `ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))` sites in handlers serialize our own response structs and carry no stored data — leave them.

**Files:**
- Modify: `rust/tonk-account-service/src/handlers.rs:29-54` (docstring + `ceremony_error`)
- Test: same file, new `tests` module at the bottom

**Interfaces:**
- Consumes: `crate::core::CeremonyError`, `crate::error::{ErrorCode, ServiceError}` (existing).
- Produces: `ceremony_error(err: CeremonyError) -> ServiceError` — same signature, but `Internal` now maps to the fixed message `"internal error"`. Later tasks and the native helpers server rely on this mapping unchanged for the non-`Internal` variants.

- [ ] **Step 1: Write the failing tests**

Append to `rust/tonk-account-service/src/handlers.rs`:

```rust
#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::ceremony_error;
    use crate::core::CeremonyError;
    use crate::error::ErrorCode;

    #[dialog_common::test]
    async fn it_hides_internal_detail_from_the_wire() {
        let err = ceremony_error(CeremonyError::Internal(
            "R2 bucket 'tonk-account-chains' unreachable".into(),
        ));
        assert_eq!(err.code, ErrorCode::InternalError);
        assert_eq!(err.message, "internal error");
    }

    #[dialog_common::test]
    async fn it_passes_ceremony_messages_through() {
        let err = ceremony_error(CeremonyError::Forbidden(
            "device is not an active member of this account".into(),
        ));
        assert_eq!(err.code, ErrorCode::Forbidden);
        assert_eq!(
            err.message,
            "device is not an active member of this account"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify the first one fails**

Run: `cargo test -p tonk-account-service --features helpers it_hides_internal_detail -- --nocapture`
Expected: FAIL — message equals the raw R2 detail, not `"internal error"`. (`it_passes_ceremony_messages_through` already passes; that is fine — it pins current behaviour.)

- [ ] **Step 3: Implement the sanitized mapping**

In `rust/tonk-account-service/src/handlers.rs`, replace the `ceremony_error` docstring and body:

```rust
/// Map a [`crate::core::CeremonyError`] onto a
/// [`crate::error::ServiceError`]. Ceremony-level messages (`Invalid`,
/// `Unauthorized`, `Forbidden`, `Conflict`) pass through: they are
/// written for callers. `Internal` detail is library/store error text
/// that must not reach the wire — it is logged here and replaced with a
/// generic message.
///
/// Shared by the wasm handlers and the native helpers server
/// ([`crate::helpers::server`]) so the two backends can't drift apart
/// on error mapping.
#[cfg(any(
    target_arch = "wasm32",
    all(feature = "helpers", not(target_arch = "wasm32"))
))]
pub fn ceremony_error(err: crate::core::CeremonyError) -> crate::error::ServiceError {
    use crate::core::CeremonyError;
    let message = match &err {
        CeremonyError::RateLimited => "rate limited".to_string(),
        CeremonyError::CodeInvalid => "invalid or expired code".to_string(),
        CeremonyError::Conflict(msg)
        | CeremonyError::Invalid(msg)
        | CeremonyError::Unauthorized(msg)
        | CeremonyError::Forbidden(msg) => msg.clone(),
        CeremonyError::Internal(detail) => {
            #[cfg(target_arch = "wasm32")]
            worker::console_error!("internal error: {detail}");
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("internal error: {detail}");
            "internal error".to_string()
        }
    };
    crate::error::ServiceError::new(err.code(), message)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p tonk-account-service --features helpers handlers::tests -- --nocapture`
Expected: 2 passed.

- [ ] **Step 5: Check the wasm target still compiles**

Run: `cargo check -p tonk-account-service --target wasm32-unknown-unknown`
Expected: clean. (`worker::console_error!` is the workers-rs logging macro; this step catches any path/feature surprise with it — if it does not resolve, use `worker::console_log!`'s error-level sibling as exported by the `worker` crate version in `Cargo.lock`, and re-run.)

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-account-service/src/handlers.rs
git commit -m "fix(tonk-account-service): keep internal error detail out of responses"
```

---

### Task 2: Enforce the foreign-key pragma in the sqlite test store

`SqliteStore::in_memory` applies the D1 schema, whose `devices.account_id REFERENCES accounts(id)` is silently unenforced because sqlite defaults `foreign_keys` off. D1 enforces it. Turn the pragma on so the native twin matches production.

**Files:**
- Modify: `rust/tonk-account-service/src/store/sqlite.rs:27-35` (`in_memory`)
- Test: same file, existing `tests` module

**Interfaces:**
- Consumes: existing `Store` trait methods (`insert_device`), `StoreError`.
- Produces: no signature changes. FK violations surface as `StoreError::Internal` (the `map_err` unique/primary-key carve-out intentionally does not cover FK violations — they indicate a bug, not a caller conflict).

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `rust/tonk-account-service/src/store/sqlite.rs`:

```rust
    #[dialog_common::test]
    async fn it_enforces_the_device_account_foreign_key() {
        let store = SqliteStore::in_memory().unwrap();
        let orphan = Device {
            account_id: 999,
            device_did: "did:key:zOrphan".into(),
            delegation_cid: "bafyCid".into(),
            name: "ghost".into(),
            status: DeviceStatus::Active,
            created_at: 1,
        };
        assert!(matches!(
            store.insert_device(&orphan).await,
            Err(StoreError::Internal(_))
        ));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p tonk-account-service --features helpers it_enforces_the_device_account_foreign_key -- --nocapture`
Expected: FAIL — the insert currently succeeds (`Ok(())`), so the `matches!` assertion trips.

- [ ] **Step 3: Turn the pragma on**

In `rust/tonk-account-service/src/store/sqlite.rs`, `in_memory`:

```rust
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0001_init.sql"))
            .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0002_link_requests.sql"))
            .map_err(map_err)?;
        Ok(Self(Mutex::new(conn)))
    }
```

- [ ] **Step 4: Run the full crate suite to verify nothing relied on unenforced FKs**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: all pass, including the new test and the existing HTTP integration test (`tests/service.rs`). If any existing test now trips an FK violation, that test was inserting orphaned rows — fix the fixture to create its parent account first (production D1 would have rejected it too); do not weaken the pragma.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-account-service/src/store/sqlite.rs
git commit -m "fix(tonk-account-service): enforce foreign keys in the sqlite test store"
```

---

### Task 3: Negative test — the authorize account-mismatch filter branch

`authorize` has two rejection paths for a wrong-account device: chain verification failure (already tested by `it_rejects_a_device_of_a_different_account`) and the registry **filter** `device.account_id == account.id` (untested). Hit the filter: the device is registered under account A, but presents a cryptographically valid delegation from account B's root with subject = root B. Verification succeeds; the filter must reject.

**Files:**
- Test: `rust/tonk-account-service/src/auth.rs` (existing `tests` module)

**Interfaces:**
- Consumes: `authorize`, `seed_device`, `derive_root_signer`, `mint_device_delegation`, `InvocationBuilder`, `InvocationChain` — all already imported in the module.
- Produces: nothing; test-only.

- [ ] **Step 1: Write the test**

Append to the `tests` module in `rust/tonk-account-service/src/auth.rs`:

```rust
    #[dialog_common::test]
    async fn it_rejects_a_registered_device_presenting_another_accounts_root() {
        let store = SqliteStore::in_memory().unwrap();

        // The device is registered under account A…
        let root_a = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let root_a_did = root_a.did();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let device_did = device.did();
        seed_device(
            &store,
            root_a_did.as_ref(),
            device_did.as_ref(),
            DeviceStatus::Active,
        )
        .await;

        // …but invokes as a delegate of account B, with a chain that
        // verifies: root B really did delegate to this device key.
        let root_b = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let root_b_did = root_b.did();
        store
            .create_account("b@x.com", root_b_did.as_ref(), "cred-b", 0)
            .await
            .unwrap();

        let chain = tonk_identity::delegation::mint_device_delegation(root_b, &device_did)
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device)
            .audience(&root_b_did)
            .subject(&root_b_did)
            .command(vec!["account".into(), "device".into(), "list".into()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }
```

(`Forbidden` distinguishes the filter from the verification path, which yields `Unauthorized`. The expiration stamp is inert until Task 6 and required after it, so the test is stable across the ordering.)

- [ ] **Step 2: Run it**

Run: `cargo test -p tonk-account-service --features helpers it_rejects_a_registered_device_presenting_another_accounts_root -- --nocapture`
Expected: PASS immediately — this pins existing behaviour of an untested branch. Confirm it is exercising the filter by temporarily changing the asserted variant to `Unauthorized` and watching it fail, then restore.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-account-service/src/auth.rs
git commit -m "test(tonk-account-service): pin the wrong-account device filter in authorize"
```

---

### Task 4: Negative test — cross-account chain get isolation

`get_chain` namespaces by `account.root_did`; prove account B cannot fetch account A's backed-up artifact even with the exact content key.

**Files:**
- Test: `rust/tonk-account-service/src/core/backup.rs` (existing `tests` module)

**Interfaces:**
- Consumes: `put_chain`, `get_chain`, `MemoryChainStore`, the module's `account(id, root_did)` fixture.
- Produces: nothing; test-only.

- [ ] **Step 1: Write the test**

Append to the `tests` module in `rust/tonk-account-service/src/core/backup.rs`:

```rust
    #[dialog_common::test]
    async fn it_refuses_a_chain_get_across_accounts() {
        let chains = MemoryChainStore::default();
        let account_a = account(1, "did:key:root-a");
        let account_b = account(2, "did:key:root-b");

        let key = put_chain(&chains, &account_a, b"a-bytes").await.unwrap();

        assert!(matches!(
            get_chain(&chains, &account_b, &key).await,
            Err(CeremonyError::Invalid(_))
        ));
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p tonk-account-service --features helpers it_refuses_a_chain_get_across_accounts -- --nocapture`
Expected: PASS (pins existing namespacing). Sanity-check it can fail: temporarily pass `&account_a` instead of `&account_b` and confirm the assertion trips, then restore.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-account-service/src/core/backup.rs
git commit -m "test(tonk-account-service): pin cross-account chain get isolation"
```

---

### Task 5: Stamp an expiration on device-signed invocations

`build_device_invocation` (`rust/tonk-identity/src/request.rs`) stamps no expiration — verified: the builder chain goes `.proofs(vec![cid])` straight to `.try_build()`. Root-signed ceremonies already stamp `Timestamp::five_minutes_from_now()` (`rust/tonk-identity/src/ceremony.rs:55`); make device invocations match before the server enforces it (Task 6).

Callers of `build_device_invocation`, enumerated by grep — all inside this repo's deploy trains, none in `tonk-cli`:
- `rust/tonk-worker/src/router/account_backup.rs` (three sites: chain put/list/get — all best-effort)
- `rust/tonk-account-service/tests/service.rs` (integration test)

**Files:**
- Modify: `rust/tonk-identity/src/request.rs`
- Test: same file, existing test

**Interfaces:**
- Consumes: `dialog_ucan_core::time::timestamp::Timestamp` (same import path `ceremony.rs` uses).
- Produces: `build_device_invocation` — signature unchanged; every produced invocation now carries `expiration = Timestamp::five_minutes_from_now()`. Task 6's server enforcement relies on this.

- [ ] **Step 1: Extend the existing test to expect an expiration**

In `rust/tonk-identity/src/request.rs`, inside `it_builds_a_device_signed_invocation_the_service_verifies`, after the `chain.verify(...)` unwrap, add:

```rust
        assert!(
            chain.invocation.expiration().is_some(),
            "device invocations must carry a ceremony expiration"
        );
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p tonk-identity it_builds_a_device_signed_invocation -- --nocapture`
Expected: FAIL on the new assertion (no expiration today).

- [ ] **Step 3: Stamp the expiration**

In `rust/tonk-identity/src/request.rs`, add the import and the builder call:

```rust
use dialog_ucan_core::time::timestamp::Timestamp;
```

and in `build_device_invocation`:

```rust
    let invocation = InvocationBuilder::new()
        .issuer(device)
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![cid])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the device invocation")?;
```

Also update the module docstring's contract sentence (first paragraph) to mention the stamp, e.g. append: "Invocations carry a five-minute expiration; the service refuses stale ones."

- [ ] **Step 4: Run the crate tests**

Run: `cargo test -p tonk-identity`
Expected: PASS.

- [ ] **Step 5: Check the wasm consumer still compiles**

Run: `cargo check -p tonk-worker --target wasm32-unknown-unknown`
Expected: clean (the three `account_backup.rs` call sites take the new behaviour with no signature change).

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-identity/src/request.rs
git commit -m "feat(tonk-identity): stamp a five-minute expiration on device invocations"
```

---

### Task 6: Enforce expiry on device-signed invocations in authorize

The root bootstrap path (`authorize_root`) already requires an expiration inside a five-minute window; the device path (`authorize`) checks nothing. Extract the check into a shared helper and apply it to both. Update the test container helper to stamp expirations, and add the two negative tests.

**Files:**
- Modify: `rust/tonk-account-service/src/auth.rs`
- Test: same file, existing `tests` module

**Interfaces:**
- Consumes: Task 5 (all in-repo senders now stamp expirations), `Timestamp` (already imported).
- Produces: `fn require_ceremony_expiration(chain: &InvocationChain<Ed25519Signature>) -> Result<(), CeremonyError>` (private); `authorize` now returns `CeremonyError::Unauthorized` for missing/expired/over-window expirations.

- [ ] **Step 1: Verify the sender inventory is still complete (STOP-and-report gate)**

Run: `grep -rn "build_device_invocation" rust/ --include="*.rs" | grep -v request.rs`
Expected: exactly `rust/tonk-account-service/tests/service.rs` and three `rust/tonk-worker/src/router/account_backup.rs` sites. Also run `grep -rn "InvocationChain::new" rust/tonk-cli rust/tonk-ui --include="*.rs"` — expected: no hits building device-signed account-service requests outside those files. **If any other sender appears** (especially in `tonk-cli`, whose released binaries update slowly), STOP and report before enforcing: enforcement would break that sender's deployed versions.

- [ ] **Step 2: Update the container helper and write the failing negative tests**

In the `tests` module of `rust/tonk-account-service/src/auth.rs`, replace the `container` helper with an expiration-parameterized pair:

```rust
    async fn container_with_expiration(
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
        expiration: Option<Timestamp>,
    ) -> (String, String, Vec<u8>) {
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let root_did = root.did();
        let chain = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let mut builder = InvocationBuilder::new()
            .issuer(device.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(command)
            .arguments(args)
            .proofs(vec![cid]);
        if let Some(expiration) = expiration {
            builder = builder.expiration(expiration);
        }
        let invocation = builder.try_build().await.unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (root_did.to_string(), device.did().to_string(), bytes)
    }

    async fn container(
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
    ) -> (String, String, Vec<u8>) {
        container_with_expiration(command, args, Some(Timestamp::five_minutes_from_now())).await
    }
```

Then add the negative tests:

```rust
    #[dialog_common::test]
    async fn it_rejects_a_device_invocation_without_expiration() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container_with_expiration(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
            None,
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_an_expired_device_invocation() {
        use std::time::{Duration, UNIX_EPOCH};

        let store = SqliteStore::in_memory().unwrap();
        let expired = Timestamp::new(UNIX_EPOCH + Duration::from_secs(1)).unwrap();
        let (root_did, device_did, bytes) = container_with_expiration(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
            Some(expired),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }
```

- [ ] **Step 3: Run the new tests to verify they fail**

Run: `cargo test -p tonk-account-service --features helpers it_rejects_a_device_invocation_without_expiration it_rejects_an_expired_device_invocation -- --nocapture`
Expected: both FAIL — `authorize` currently accepts them (`Ok(Caller { .. })`).

- [ ] **Step 4: Extract the shared expiration check and enforce it**

In `rust/tonk-account-service/src/auth.rs`, add below `verified_chain`:

```rust
/// Require an expiration on the invocation and bound it to the
/// five-minute ceremony window every account-service request uses.
fn require_ceremony_expiration(
    chain: &InvocationChain<Ed25519Signature>,
) -> Result<(), CeremonyError> {
    let expiration = chain.invocation.expiration().ok_or_else(|| {
        CeremonyError::Unauthorized("invocation must carry an expiration".to_string())
    })?;
    let now = Timestamp::now();
    if expiration < now {
        return Err(CeremonyError::Unauthorized(
            "invocation has expired".to_string(),
        ));
    }
    if expiration > Timestamp::five_minutes_from_now() {
        return Err(CeremonyError::Unauthorized(
            "invocation expiration exceeds the five-minute ceremony window".to_string(),
        ));
    }
    Ok(())
}
```

(The three lines of logic are moved verbatim from `authorize_root`, so the `Timestamp` comparison forms are already known to compile.)

In `authorize`, after the `verified_chain(...)` call:

```rust
    let chain = verified_chain(body, expected_command).await?;
    require_ceremony_expiration(&chain)?;
```

In `authorize_root`, replace the inline expiration block (the `let expiration = ...` through the over-window `if`) with:

```rust
    require_ceremony_expiration(&chain)?;
```

- [ ] **Step 5: Run the full crate suite**

Run: `cargo test -p tonk-account-service --features helpers`
Expected: all pass — the new negatives, every pre-existing `authorize` test (their containers now stamp expirations via the updated helper), `authorize_root` tests (behaviour unchanged, messages slightly generalized), and the HTTP integration test in `tests/service.rs` (its invocations gained expirations in Task 5).

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-account-service/src/auth.rs
git commit -m "fix(tonk-account-service): require the ceremony expiry window on device invocations"
```

---

### Task 7: Abuse-controls runbook and deploy verification

Code carries no per-IP limits by design — that lives at the Cloudflare edge. Document the runbook in the crate README and verify the two outstanding deploy prerequisites from the CLI-handoff work: migration `0002_link_requests.sql` applied everywhere, and the rate rule extended from `POST /codes` to also cover `POST /links`. Turnstile stays deferred until abuse is observed.

**Files:**
- Modify: `rust/tonk-account-service/README.md`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: documentation only.

- [ ] **Step 1: Add the runbook section**

Append to `rust/tonk-account-service/README.md`:

```markdown
## Abuse controls

Application-level throttles live in `src/core/codes.rs`: per-email 60 s
resend cooldown, 10-minute code TTL, five verification attempts per code.
Everything IP-shaped is enforced at the Cloudflare edge, not in code:

- **Rate rule** (zone `tonk.xyz`, and the staging host): one rate-limiting
  rule covering the two unauthenticated write paths —

  ```
  (http.request.method eq "POST" and
   http.request.uri.path in {"/codes" "/links"} and
   http.host in {"accounts.tonk.xyz" "accounts-staging.tonk.xyz"})
  ```

  Suggested threshold to start: 10 requests per 10 minutes per IP, block
  for 1 hour. `/links/resolve|complete|consume` need no rule: they demand
  the 256-bit bearer secret and cheap lookups fail closed.
- **Turnstile**: deliberately not deployed. Revisit only if the rate rule
  proves insufficient in practice.

### Deploy verification

Migrations must be applied to both environments (wrangler reads
`wrangler.account.toml`):

```sh
npx wrangler d1 migrations list tonk-accounts --remote -c wrangler.account.toml
npx wrangler d1 migrations list tonk-accounts-staging --remote -c wrangler.account.toml --env staging
```

Both must show `0001_init.sql` and `0002_link_requests.sql` as applied;
apply any pending ones with the matching `d1 migrations apply` command.
Confirm the rate rule exists in the Cloudflare dashboard (Security →
WAF → Rate limiting rules) and that its path list includes `/links`.
```

- [ ] **Step 2: Run the manual verification and record the outcome**

Run both `wrangler d1 migrations list` commands above (requires Cloudflare auth; if credentials are unavailable in this session, STOP and report — the PR can land, but flag the verification as still owed in the PR body). Check the dashboard rate rule the same way. Expected: 0001 and 0002 applied in both environments; rule present with both paths.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-account-service/README.md
git commit -m "docs(tonk-account-service): abuse-controls runbook and deploy verification"
```

---

### Task 8: Full verification gate

**Files:** none (verification only).

**Interfaces:** consumes all prior tasks.

- [ ] **Step 1: Workspace lint gate**

Run: `cargo clippy --workspace --all-targets --all-features`
Expected: no warnings. (This compiles integration tests too; it is the CI gate.)

- [ ] **Step 2: Format check**

Run: `cargo fmt --check`
Expected: no diffs.

- [ ] **Step 3: Crate suites**

Run: `cargo test -p tonk-account-service --features helpers && cargo test -p tonk-identity`
Expected: all pass.

- [ ] **Step 4: Wasm targets**

Run: `cargo check -p tonk-account-service --target wasm32-unknown-unknown && cargo check -p tonk-worker --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 5: Push and open the PR**

Branch: `fix/account-service-hardening`, base `staging`. PR title: `fix(tonk-account-service): hardening follow-ups from the registry review`. Body lists the six review items this closes and the two accepted decisions (no nonce table; stale-service-worker rollout note), and records the Task 7 verification outcome.

```bash
git push -u origin fix/account-service-hardening
gh pr create --base staging --title "fix(tonk-account-service): hardening follow-ups from the registry review" --body-file -
```
