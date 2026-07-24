# Revocation Enforcement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make device revocation real: a device marked `revoked` in the account registry loses presigned storage access at the access-service `/ucan/` boundary.

**Architecture:** The access-service gains a read-only D1 binding to the accounts database. After the existing cryptographic authorization succeeds, the handler independently parses the presented UCAN container, collects every delegation CID and issuer DID plus the invocation issuer, and asks a `RevocationRegistry` whether any of them belong to a revoked device. The decision core is pure and natively tested; D1 and a per-isolate TTL cache are thin wasm-only glue. The check fails open on registry errors, per the approved design's availability posture.

**Tech Stack:** workers-rs (`worker` crate, D1), `dialog-ucan-core` (container parsing), Cloudflare D1, `dialog_common::test` for native tests.

## Why this shape (decisions)

- **Match on both delegation CIDs and issuer DIDs.** The CID match kills the
  exact `root → device` grant recorded at registration (`devices.delegation_cid`
  is `Cid::to_string()` — `tonk-account-service/src/core/delegation.rs:54`).
  The issuer-DID match is the security-bearing one: it also severs anything a
  revoked device *itself* signed — re-anchored chains
  (`space → eph → device → root → other`) flow through a delegation *issued by*
  the revoked device, whose CID the registry has never seen, and a revoked
  device could mint fresh delegations at will. Both checks come from the same
  one D1 query.
- **Check runs after `authorize` succeeds.** Garbage requests keep failing on
  the existing path; the revocation parse only ever sees containers the
  verifier already accepted, so a `collect_presented` failure is logged and
  fails open rather than rejecting.
- **Fail-open** on any registry/D1 error, with a per-isolate verdict cache
  (60 s TTL) so the hot path does at most one D1 query per unseen credential
  set per minute. Accepted by the master design: a revoked device gets a brief
  window during a D1 outage, never an availability loss for everyone else.

  > **Superseded.** This shipped as written in #641 and was then inverted:
  > the screen fails *closed*, with the cache extended by a 10-minute grace
  > window that covers an unreachable registry for credentials it recently
  > cleared. Fail-open made the security property conditional on config
  > correctness — a renamed binding disabled enforcement with only a log
  > line — while the grace window buys back the availability fail-open was
  > protecting. See the completion spec's stage R for the reasoning.
- **Entitlement seam:** billing later adds an `entitlements` lookup as a new
  method on `RevocationRegistry` (or a sibling trait) plus one more arm in the
  handler; `collect_presented`, `assess`, and the cache do not change. Keep
  the trait in `revocation.rs` documented to that effect.
- **The D1 binding is read-only by convention only** — D1 has no grant
  scoping. The access-service must never issue anything but `SELECT` against
  `ACCOUNTS_DB`; migrations stay owned by `tonk-account-service`
  (`migrations_dir` is deliberately absent from the new binding stanzas).

## Global Constraints

- Lint gate: `cargo clippy --workspace --all-targets --all-features` and `cargo fmt --check` must pass (the `--all-features` build compiles the helpers-gated tests).
- Tests: `#[dialog_common::test]`, names `it_does_x`, native tests gated `#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]` (mirrors `tonk-account-service`).
- No `mod.rs`: submodules use `foo.rs` + `foo/` form (`revocation.rs` + `revocation/d1.rs`).
- No stage/phase/PR references in code or doc comments.
- Conventional commits, scope `tonk-access-service` (config commit scope `account`).
- wasm-only code is gated exactly like `tonk-account-service/src/store.rs` gates its `d1` module: `#[cfg(target_arch = "wasm32")]`.
- Dialog pin: `dialog-db` tag `tonk-2026-07-17` (checkout `~/.cargo/git/checkouts/dialog-db-1cb9c87f3902090f/2e18c18/`). If the pin moves before execution, re-run Task 1.

---

### Task 1: Verify the parsing and registry invariants this plan stands on

Everything below was verified against the current pin during planning and is
re-checked here because the plan may execute after a rebase or dialog bump.

**Files:** none modified — read-only verification.

- [ ] **Step 1: The `/ucan/` body parses as `InvocationChain` (same type the account service parses)**

Run:
```bash
sed -n '289,296p' ~/.cargo/git/checkouts/dialog-db-*/2e18c18/rust/dialog-remote-ucan-s3/src/authorizer.rs
```
Expected: `UcanAuthorizer::authorize` begins `let chain = InvocationChain::try_from(container)`. The body handed to `authorize` in `rust/tonk-access-service/src/handlers/ucan.rs:54` is therefore a standard UCAN container (`{ "ctn-v1": [invocation, delegation...] }`) that this plan may parse independently with public APIs.

- [ ] **Step 2: `Container`, `Delegation`, `Invocation` expose what `collect_presented` needs**

Run:
```bash
grep -n "pub use container\|pub fn from_bytes\|pub fn into_tokens" ~/.cargo/git/checkouts/dialog-db-*/2e18c18/rust/dialog-ucan-core/src/lib.rs ~/.cargo/git/checkouts/dialog-db-*/2e18c18/rust/dialog-ucan-core/src/container.rs
grep -n "pub const fn issuer\|pub fn to_cid" ~/.cargo/git/checkouts/dialog-db-*/2e18c18/rust/dialog-ucan-core/src/delegation.rs ~/.cargo/git/checkouts/dialog-db-*/2e18c18/rust/dialog-ucan-core/src/invocation.rs
```
Expected: `pub use container::{Container, ContainerError}` re-export; `Container::from_bytes(&[u8])`, `into_tokens() -> Vec<Vec<u8>>`; `Delegation::issuer() -> &Did`, `Delegation::to_cid() -> Cid`; `Invocation::issuer()`, `Invocation::to_cid()`. Tokens decode with `serde_ipld_dagcbor::from_slice` (see `container/invocation.rs` `TryFrom<Container>`); token 0 is the invocation, the rest are delegations.

- [ ] **Step 3: CID string format parity with the registry**

Run:
```bash
sed -n '44,55p' rust/tonk-account-service/src/core/delegation.rs
```
Expected: the stored `devices.delegation_cid` is `chain.proof_cids()[0].to_string()` — the same `dialog_ucan_core` `Cid` `Display` this plan uses, so string equality is the correct join.

- [ ] **Step 4: Registry schema and status strings**

Run:
```bash
grep -n "status" rust/tonk-account-service/migrations/0001_init.sql
grep -n '"revoked"' rust/tonk-account-service/src/store.rs
```
Expected: `devices.status TEXT NOT NULL DEFAULT 'active'`; status strings are exactly `active` / `revoked`.

- [ ] **Step 5: D1 access from a route context**

Run:
```bash
grep -rn "pub fn d1" ~/.cargo/registry/src/*/worker-0.8*/src/router.rs
```
Expected: `RouteContext::d1(&self, binding: &str) -> Result<D1Database>`.

**STOP conditions** — halt and report instead of improvising:
- `authorize` no longer parses `InvocationChain::try_from` (container format changed) → the independent parse may disagree with what the verifier checks; the fix is an upstream dialog-db hook exposing verified proofs, not a workaround here.
- `Container`/`Delegation`/`Invocation` or their accessors are no longer public → same upstream conversation.
- `devices.delegation_cid` is no longer `Cid::to_string()` format → the CID join breaks silently; reconcile formats first.

- [ ] **Step 6: Nothing to commit** — verification only.

---

### Task 2: Bind the accounts database in `wrangler.toml`

**Files:**
- Modify: `wrangler.toml` (repo root — this is the access-service deploy config)

**Interfaces:**
- Produces: D1 binding name `"ACCOUNTS_DB"`, consumed by Task 8's `ctx.d1("ACCOUNTS_DB")`.

- [ ] **Step 1: Add the production binding**

After the existing `[[r2_buckets]]` block (before `[vars]`) insert:

```toml
# Read-only view of the account registry (owned and migrated by
# tonk-account-service): the presign path checks presented credentials
# against revoked devices. Never write through this binding.
[[d1_databases]]
binding = "ACCOUNTS_DB"
database_name = "tonk-accounts"
database_id = "a0c6698d-25e6-414b-8986-4fbeb1b5f992"
```

- [ ] **Step 2: Add the staging binding**

After the existing `[[env.staging.r2_buckets]]` block insert:

```toml
[[env.staging.d1_databases]]
binding = "ACCOUNTS_DB"
database_name = "tonk-accounts-staging"
database_id = "d82a5715-d210-4bca-bd9d-4ab64e909566"
```

- [ ] **Step 3: Sanity-check the TOML**

Run: `python3 -c "import tomllib; tomllib.load(open('wrangler.toml','rb')); print('ok')"`
Expected: `ok`

- [ ] **Step 4: Commit**

```bash
git add wrangler.toml
git commit -m "feat(account): bind the accounts database in the access-service worker"
```

---

### Task 3: `DeviceRevoked` error code

**Files:**
- Modify: `rust/tonk-access-service/src/error.rs`

**Interfaces:**
- Produces: `ErrorCode::DeviceRevoked` (HTTP 403), consumed by Task 8.

- [ ] **Step 1: Add the variant**

In the `ErrorCode` enum, extend the 403 group:

```rust
    // 403 Forbidden - Authorization errors
    ChainInvalid,
    CommandMismatch,
    SubjectNotAllowed,
    DeviceRevoked,
```

And in `status_code()` extend the 403 arm:

```rust
            // 403 Forbidden
            ErrorCode::ChainInvalid
            | ErrorCode::CommandMismatch
            | ErrorCode::SubjectNotAllowed
            | ErrorCode::DeviceRevoked => 403,
```

- [ ] **Step 2: Build**

Run: `cargo check -p tonk-access-service`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add rust/tonk-access-service/src/error.rs
git commit -m "feat(tonk-access-service): add a device-revoked error code"
```

---

### Task 4: Collect presented credentials from a UCAN container

**Files:**
- Create: `rust/tonk-access-service/src/revocation.rs`
- Modify: `rust/tonk-access-service/src/lib.rs` (module declaration)

**Interfaces:**
- Produces:
  - `pub struct PresentedCredentials { pub invocation_issuer: String, pub delegation_cids: Vec<String>, pub delegation_issuers: Vec<String> }`
  - `impl PresentedCredentials { pub fn keys(&self) -> Vec<String> }` — deduplicated union of all three, the registry lookup key set
  - `pub fn collect_presented(container_bytes: &[u8]) -> Result<PresentedCredentials, ContainerError>`
- Consumed by Tasks 6 and 8.

- [ ] **Step 1: Declare the module**

In `rust/tonk-access-service/src/lib.rs` after `mod handlers;`:

```rust
mod revocation;
```

- [ ] **Step 2: Write the failing tests**

Create `rust/tonk-access-service/src/revocation.rs`:

```rust
//! Revocation screening for presented UCAN containers.
//!
//! After cryptographic authorization succeeds, the presign path checks
//! whether any credential in the presented chain belongs to a revoked
//! device: the CID of every delegation, the issuer DID of every
//! delegation, and the invocation's issuer DID are matched against the
//! account registry. The decision logic here is pure and natively
//! tested; D1 glue lives in [`d1`] and is wasm-only.
//!
//! An entitlement lookup for billing later extends the registry trait
//! (or adds a sibling) — the collection and decision shapes here stay
//! as they are.

use std::collections::BTreeSet;

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::algorithm::eddsa::Ed25519Signature;

/// Every credential identity found in a presented UCAN container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCredentials {
    /// The DID that signed the invocation (the requesting operator).
    pub invocation_issuer: String,
    /// CIDs of every delegation: those referenced by the invocation's
    /// proof list and those carried as container tokens.
    pub delegation_cids: Vec<String>,
    /// Issuer DIDs of every delegation carried in the container.
    pub delegation_issuers: Vec<String>,
}

impl PresentedCredentials {
    /// The deduplicated set of registry lookup keys: delegation CIDs,
    /// delegation issuer DIDs, and the invocation issuer DID. CIDs and
    /// DIDs cannot collide (different prefixes), so one key space is
    /// safe.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: BTreeSet<String> = BTreeSet::new();
        keys.extend(self.delegation_cids.iter().cloned());
        keys.extend(self.delegation_issuers.iter().cloned());
        keys.insert(self.invocation_issuer.clone());
        keys.into_iter().collect()
    }
}

/// Parse a UCAN container and collect every presented credential
/// identity. Token 0 is the invocation; the remaining tokens are
/// delegations, exactly as `InvocationChain::try_from` consumes them.
pub fn collect_presented(
    container_bytes: &[u8],
) -> Result<PresentedCredentials, ContainerError> {
    let tokens = Container::from_bytes(container_bytes)?.into_tokens();
    let Some(invocation_bytes) = tokens.first() else {
        return Err(ContainerError::Invocation(
            "container must contain at least an invocation".to_string(),
        ));
    };

    let invocation: Invocation<Ed25519Signature> =
        serde_ipld_dagcbor::from_slice(invocation_bytes).map_err(|err| {
            ContainerError::Invocation(format!("failed to decode invocation: {err}"))
        })?;

    let mut delegation_cids = BTreeSet::new();
    for cid in invocation.proofs() {
        delegation_cids.insert(cid.to_string());
    }

    let mut delegation_issuers = BTreeSet::new();
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(bytes)
            .map_err(|err| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {err}"))
            })?;
        delegation_cids.insert(delegation.to_cid().to_string());
        delegation_issuers.insert(delegation.issuer().to_string());
    }

    Ok(PresentedCredentials {
        invocation_issuer: invocation.issuer().to_string(),
        delegation_cids: delegation_cids.into_iter().collect(),
        delegation_issuers: delegation_issuers.into_iter().collect(),
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, InvocationBuilder, InvocationChain};
    use dialog_varsig::Principal;

    const ROOT_SEED: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    /// A container shaped like a linked device's presign request: one
    /// subject-open `root → device` delegation, invocation issued by the
    /// device. Returns (delegation cid, root did, device did, bytes).
    async fn device_container() -> (String, String, String, Vec<u8>) {
        let root = Ed25519Signer::import(&ROOT_SEED).await.unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let root_did = root.did();

        let delegation = DelegationBuilder::new()
            .issuer(root.clone())
            .audience(&device.did())
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let cid = delegation.to_cid();

        let invocation = InvocationBuilder::new()
            .issuer(device.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(vec!["memory".to_string(), "resolve".to_string()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .try_build()
            .await
            .unwrap();

        let mut proofs = HashMap::new();
        proofs.insert(cid, Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (
            cid.to_string(),
            root_did.to_string(),
            device.did().to_string(),
            bytes,
        )
    }

    #[dialog_common::test]
    async fn it_collects_cids_and_issuers_from_a_container() {
        let (cid, root_did, device_did, bytes) = device_container().await;

        let presented = collect_presented(&bytes).unwrap();

        assert_eq!(presented.invocation_issuer, device_did);
        assert!(presented.delegation_cids.contains(&cid));
        assert!(presented.delegation_issuers.contains(&root_did));
    }

    #[dialog_common::test]
    async fn it_unions_all_identities_into_the_key_set() {
        let (cid, root_did, device_did, bytes) = device_container().await;

        let keys = collect_presented(&bytes).unwrap().keys();

        assert!(keys.contains(&cid));
        assert!(keys.contains(&root_did));
        assert!(keys.contains(&device_did));
        let mut deduped = keys.clone();
        deduped.dedup();
        assert_eq!(deduped, keys, "keys must be deduplicated and sorted");
    }

    #[dialog_common::test]
    async fn it_rejects_an_empty_or_garbage_container() {
        assert!(collect_presented(&[]).is_err());
        assert!(collect_presented(&[0xde, 0xad, 0xbe, 0xef]).is_err());
    }
}
```

- [ ] **Step 3: Run the tests, expect failure to compile only if APIs drifted**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: 3 tests PASS (the code and tests land together; a compile error here means Task 1 drift — STOP per Task 1).

- [ ] **Step 4: Lint**

Run: `cargo clippy -p tonk-access-service --all-targets --all-features && cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-access-service/src/lib.rs rust/tonk-access-service/src/revocation.rs
git commit -m "feat(tonk-access-service): collect presented credentials from ucan containers"
```

---

### Task 5: Verdict cache primitives

**Files:**
- Modify: `rust/tonk-access-service/src/revocation.rs`

**Interfaces:**
- Produces (all in `revocation.rs`):
  - `pub const REVOCATION_TTL_MS: u64 = 60_000;`
  - `pub struct CachedVerdict { pub revoked: bool, pub expires_at_ms: u64 }`
  - `pub struct CacheProbe { pub cached_revoked: bool, pub misses: Vec<String> }`
  - `pub fn split_with(map: &mut HashMap<String, CachedVerdict>, keys: &[String], now_ms: u64) -> CacheProbe` — pure
  - `pub fn store_with(map: &mut HashMap<String, CachedVerdict>, queried: &[String], revoked: &[String], now_ms: u64)` — pure
  - `pub fn cache_probe(keys: &[String], now_ms: u64) -> CacheProbe` and `pub fn cache_record(queried: &[String], revoked: &[String], now_ms: u64)` — thread-local wrappers over the pure pair
- Consumed by Task 6's `assess`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `revocation.rs`:

```rust
    #[dialog_common::test]
    async fn it_splits_unseen_keys_into_misses() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string(), "b".to_string()];

        let probe = split_with(&mut map, &keys, 1_000);

        assert!(!probe.cached_revoked);
        assert_eq!(probe.misses, keys);
    }

    #[dialog_common::test]
    async fn it_serves_cached_verdicts_within_ttl() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string(), "b".to_string()];
        store_with(&mut map, &keys, &["b".to_string()], 1_000);

        let probe = split_with(&mut map, &keys, 1_000 + REVOCATION_TTL_MS - 1);

        assert!(probe.cached_revoked, "b is cached revoked");
        assert!(probe.misses.is_empty());
    }

    #[dialog_common::test]
    async fn it_expires_cached_verdicts_after_ttl() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string()];
        store_with(&mut map, &keys, &[], 1_000);

        let probe = split_with(&mut map, &keys, 1_000 + REVOCATION_TTL_MS + 1);

        assert!(!probe.cached_revoked);
        assert_eq!(probe.misses, keys);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: FAIL — `split_with`/`store_with` not found.

- [ ] **Step 3: Implement the cache**

Add above the tests module:

```rust
use std::cell::RefCell;
use std::collections::HashMap;

/// How long a revocation verdict may be served from the per-isolate
/// cache. Short on purpose: this bounds how stale enforcement can be,
/// and the design accepts up to a minute of lag after a revoke.
pub const REVOCATION_TTL_MS: u64 = 60_000;

/// A cached per-key verdict.
#[derive(Debug, Clone, Copy)]
pub struct CachedVerdict {
    /// Whether the key matched a revoked device when last queried.
    pub revoked: bool,
    /// Absolute expiry, in the caller's millisecond clock.
    pub expires_at_ms: u64,
}

/// The result of probing the cache for a key set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheProbe {
    /// True if any key has a live cached `revoked = true` verdict.
    pub cached_revoked: bool,
    /// Keys with no live cached verdict, needing a registry query.
    pub misses: Vec<String>,
}

/// Probe `map` for `keys` at `now_ms`, evicting expired entries.
pub fn split_with(
    map: &mut HashMap<String, CachedVerdict>,
    keys: &[String],
    now_ms: u64,
) -> CacheProbe {
    map.retain(|_, verdict| verdict.expires_at_ms > now_ms);
    let mut cached_revoked = false;
    let mut misses = Vec::new();
    for key in keys {
        match map.get(key) {
            Some(verdict) => cached_revoked |= verdict.revoked,
            None => misses.push(key.clone()),
        }
    }
    CacheProbe {
        cached_revoked,
        misses,
    }
}

/// Record fresh verdicts: every `queried` key gets an entry, `true` for
/// those in `revoked`.
pub fn store_with(
    map: &mut HashMap<String, CachedVerdict>,
    queried: &[String],
    revoked: &[String],
    now_ms: u64,
) {
    let expires_at_ms = now_ms + REVOCATION_TTL_MS;
    for key in queried {
        map.insert(
            key.clone(),
            CachedVerdict {
                revoked: revoked.contains(key),
                expires_at_ms,
            },
        );
    }
}

thread_local! {
    static VERDICTS: RefCell<HashMap<String, CachedVerdict>> = RefCell::new(HashMap::new());
}

/// Probe the per-isolate verdict cache.
pub fn cache_probe(keys: &[String], now_ms: u64) -> CacheProbe {
    VERDICTS.with(|cell| split_with(&mut cell.borrow_mut(), keys, now_ms))
}

/// Record fresh verdicts in the per-isolate cache.
pub fn cache_record(queried: &[String], revoked: &[String], now_ms: u64) {
    VERDICTS.with(|cell| store_with(&mut cell.borrow_mut(), queried, revoked, now_ms));
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-access-service/src/revocation.rs
git commit -m "feat(tonk-access-service): ttl cache primitives for revocation verdicts"
```

---

### Task 6: Registry trait and the `assess` decision

**Files:**
- Modify: `rust/tonk-access-service/src/revocation.rs`

**Interfaces:**
- Consumes: `PresentedCredentials::keys()` (Task 4), `cache_probe`/`cache_record` (Task 5).
- Produces:
  - `pub struct RegistryError(pub String);` (implements `Display`)
  - `pub trait RevocationRegistry { async fn revoked_of(&self, keys: &[String]) -> Result<Vec<String>, RegistryError>; }` — returns the subset of `keys` that match a revoked device
  - `pub enum RevocationVerdict { Allowed, AllowedFailOpen(String), Revoked }`
  - `pub async fn assess<R: RevocationRegistry>(registry: &R, presented: &PresentedCredentials, now_ms: u64) -> RevocationVerdict`
- Consumed by Tasks 7 and 8.

- [ ] **Step 1: Write the failing tests**

Append to the tests module (the stub counts queries so the cache-hit test can assert no second round trip; each test uses unique keys so the thread-local cache cannot leak across tests):

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubRegistry {
        revoked: Vec<String>,
        fail: bool,
        queries: AtomicUsize,
    }

    impl StubRegistry {
        fn revoking(revoked: &[&str]) -> Self {
            Self {
                revoked: revoked.iter().map(|s| s.to_string()).collect(),
                fail: false,
                queries: AtomicUsize::new(0),
            }
        }

        fn failing() -> Self {
            Self {
                revoked: Vec::new(),
                fail: true,
                queries: AtomicUsize::new(0),
            }
        }
    }

    impl RevocationRegistry for StubRegistry {
        async fn revoked_of(&self, keys: &[String]) -> Result<Vec<String>, RegistryError> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(RegistryError("d1 unavailable".to_string()));
            }
            Ok(keys
                .iter()
                .filter(|key| self.revoked.contains(key))
                .cloned()
                .collect())
        }
    }

    fn presented(prefix: &str) -> PresentedCredentials {
        PresentedCredentials {
            invocation_issuer: format!("did:key:{prefix}-device"),
            delegation_cids: vec![format!("bafy{prefix}cid")],
            delegation_issuers: vec![format!("did:key:{prefix}-root")],
        }
    }

    #[dialog_common::test]
    async fn it_allows_a_chain_with_no_revoked_credentials() {
        let registry = StubRegistry::revoking(&[]);

        let verdict = assess(&registry, &presented("clean"), 1_000).await;

        assert!(matches!(verdict, RevocationVerdict::Allowed));
    }

    #[dialog_common::test]
    async fn it_revokes_when_a_delegation_cid_is_revoked() {
        let registry = StubRegistry::revoking(&["bafycidhitcid"]);

        let verdict = assess(&registry, &presented("cidhit"), 1_000).await;

        assert!(matches!(verdict, RevocationVerdict::Revoked));
    }

    #[dialog_common::test]
    async fn it_revokes_when_a_delegation_issuer_is_a_revoked_device() {
        let registry = StubRegistry::revoking(&["did:key:didhit-root"]);

        let verdict = assess(&registry, &presented("didhit"), 1_000).await;

        assert!(matches!(verdict, RevocationVerdict::Revoked));
    }

    #[dialog_common::test]
    async fn it_fails_open_when_the_registry_errors() {
        let registry = StubRegistry::failing();

        let verdict = assess(&registry, &presented("outage"), 1_000).await;

        assert!(matches!(verdict, RevocationVerdict::AllowedFailOpen(_)));
    }

    #[dialog_common::test]
    async fn it_serves_the_second_assessment_from_cache() {
        let registry = StubRegistry::revoking(&[]);
        let credentials = presented("cached");

        let first = assess(&registry, &credentials, 1_000).await;
        let second = assess(&registry, &credentials, 2_000).await;

        assert!(matches!(first, RevocationVerdict::Allowed));
        assert!(matches!(second, RevocationVerdict::Allowed));
        assert_eq!(
            registry.queries.load(Ordering::SeqCst),
            1,
            "second assessment within the ttl must not query the registry"
        );
    }

    #[dialog_common::test]
    async fn it_does_not_cache_a_fail_open_allowance() {
        let registry = StubRegistry::failing();
        let credentials = presented("nocache");

        let _ = assess(&registry, &credentials, 1_000).await;
        let _ = assess(&registry, &credentials, 2_000).await;

        assert_eq!(
            registry.queries.load(Ordering::SeqCst),
            2,
            "a fail-open result must not be served from cache"
        );
    }
```

Note: `assess` reaches the thread-local cache, so these tests assume the test
body stays on one thread. `dialog_common::test` runs on a current-thread
runtime; if that ever changes and the two cache tests flake, port them onto
`split_with`/`store_with` with an explicit map and drop the query-count
assertions.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: FAIL — `RevocationRegistry`, `assess` not found.

- [ ] **Step 3: Implement**

Add above the tests module:

```rust
use std::fmt;

/// A registry lookup failure. The presign path treats any of these as
/// fail-open: log and allow, per the design's availability posture.
#[derive(Debug)]
pub struct RegistryError(pub String);

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Answers which presented credential keys belong to revoked devices.
///
/// The one production implementation reads the account registry's
/// `devices` table over D1 ([`d1::D1RevocationRegistry`]). An
/// entitlement lookup for plan limits later extends this trait — the
/// decision flow in [`assess`] stays as it is.
pub trait RevocationRegistry {
    /// Return the subset of `keys` (delegation CIDs and device DIDs)
    /// that match a revoked device.
    async fn revoked_of(&self, keys: &[String]) -> Result<Vec<String>, RegistryError>;
}

/// The outcome of screening presented credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationVerdict {
    /// No presented credential is revoked.
    Allowed,
    /// The registry could not answer; allowed by the fail-open posture.
    /// Carries the reason for logging.
    AllowedFailOpen(String),
    /// A presented credential belongs to a revoked device.
    Revoked,
}

/// Screen presented credentials against the registry, through the
/// per-isolate verdict cache. Fail-open results are never cached.
pub async fn assess<R: RevocationRegistry>(
    registry: &R,
    presented: &PresentedCredentials,
    now_ms: u64,
) -> RevocationVerdict {
    let keys = presented.keys();
    let probe = cache_probe(&keys, now_ms);
    if probe.cached_revoked {
        return RevocationVerdict::Revoked;
    }
    if probe.misses.is_empty() {
        return RevocationVerdict::Allowed;
    }
    match registry.revoked_of(&probe.misses).await {
        Ok(revoked) => {
            cache_record(&probe.misses, &revoked, now_ms);
            if revoked.is_empty() {
                RevocationVerdict::Allowed
            } else {
                RevocationVerdict::Revoked
            }
        }
        Err(err) => RevocationVerdict::AllowedFailOpen(err.to_string()),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: all PASS.

- [ ] **Step 5: Lint**

Run: `cargo clippy -p tonk-access-service --all-targets --all-features && cargo fmt --check`
Expected: clean (the `async fn` in a public trait is fine for static dispatch; if clippy raises `async_fn_in_trait` under the workspace lint set, silence it at the trait with a doc-comment-justified `#[allow(async_fn_in_trait)]` — dyn dispatch is never used here).

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-access-service/src/revocation.rs
git commit -m "feat(tonk-access-service): revocation registry trait and fail-open assessment"
```

---

### Task 7: D1-backed registry

**Files:**
- Create: `rust/tonk-access-service/src/revocation/d1.rs`
- Modify: `rust/tonk-access-service/src/revocation.rs` (module declaration + query builder)

**Interfaces:**
- Consumes: `RevocationRegistry`, `RegistryError` (Task 6).
- Produces:
  - `pub fn revoked_query(key_count: usize) -> String` (in `revocation.rs` — pure, natively tested)
  - `pub struct D1RevocationRegistry` with `pub fn new(db: worker::d1::D1Database) -> Self`, implementing `RevocationRegistry` (wasm-only)
- Consumed by Task 8.

- [ ] **Step 1: Write the failing query-builder test**

Append to the tests module in `revocation.rs`:

```rust
    #[dialog_common::test]
    async fn it_builds_the_revoked_query_with_numbered_placeholders() {
        let sql = revoked_query(2);

        assert_eq!(
            sql,
            "SELECT delegation_cid, device_did FROM devices \
             WHERE status = 'revoked' \
             AND (delegation_cid IN (?1, ?2) OR device_did IN (?1, ?2))"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: FAIL — `revoked_query` not found.

- [ ] **Step 3: Implement the query builder and module hook**

In `revocation.rs`, add:

```rust
#[cfg(target_arch = "wasm32")]
pub mod d1;

/// The one query this service issues against the account registry:
/// which of the presented keys match a revoked device, by delegation
/// CID or by device DID. Numbered placeholders are reused across both
/// `IN` lists so the key set binds once.
pub fn revoked_query(key_count: usize) -> String {
    let placeholders = (1..=key_count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT delegation_cid, device_did FROM devices \
         WHERE status = 'revoked' \
         AND (delegation_cid IN ({placeholders}) OR device_did IN ({placeholders}))"
    )
}
```

- [ ] **Step 4: Run the query-builder test**

Run: `cargo test -p tonk-access-service --features helpers revocation`
Expected: all PASS.

- [ ] **Step 5: Implement the D1 glue**

Create `rust/tonk-access-service/src/revocation/d1.rs`:

```rust
//! D1-backed [`RevocationRegistry`](super::RevocationRegistry).
//!
//! Reads the account registry's `devices` table through the
//! `ACCOUNTS_DB` binding. Read-only by convention: the registry is
//! owned and migrated by the account service; this module must never
//! issue anything but `SELECT`.

use serde::Deserialize;
use worker::d1::D1Database;
use worker::wasm_bindgen::JsValue;

use super::{RegistryError, RevocationRegistry, revoked_query};

/// A revoked-device row, only the two join columns.
#[derive(Deserialize)]
struct RevokedRow {
    delegation_cid: String,
    device_did: String,
}

/// D1-backed registry over the accounts database.
pub struct D1RevocationRegistry(D1Database);

impl D1RevocationRegistry {
    /// Wrap the `ACCOUNTS_DB` binding.
    pub fn new(db: D1Database) -> Self {
        Self(db)
    }
}

impl RevocationRegistry for D1RevocationRegistry {
    async fn revoked_of(&self, keys: &[String]) -> Result<Vec<String>, RegistryError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let binds: Vec<JsValue> = keys.iter().map(|key| JsValue::from_str(key)).collect();
        let rows = self
            .0
            .prepare(&revoked_query(keys.len()))
            .bind(&binds)
            .map_err(|err| RegistryError(err.to_string()))?
            .all()
            .await
            .map_err(|err| RegistryError(err.to_string()))?
            .results::<RevokedRow>()
            .map_err(|err| RegistryError(err.to_string()))?;

        Ok(keys
            .iter()
            .filter(|key| {
                rows.iter()
                    .any(|row| row.delegation_cid == **key || row.device_did == **key)
            })
            .cloned()
            .collect())
    }
}
```

- [ ] **Step 6: Compile for the real target**

Run: `cargo check -p tonk-access-service --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-access-service/src/revocation.rs rust/tonk-access-service/src/revocation/d1.rs
git commit -m "feat(tonk-access-service): d1-backed revocation registry"
```

---

### Task 8: Enforce at the presign handler

**Files:**
- Modify: `rust/tonk-access-service/src/handlers/ucan.rs`
- Modify: `rust/tonk-access-service/src/lib.rs` (module visibility if needed: `mod revocation;` stays crate-private; handlers reach it via `crate::revocation`)

**Interfaces:**
- Consumes: `collect_presented`, `assess`, `D1RevocationRegistry`, `RevocationVerdict` (Tasks 4–7), `ErrorCode::DeviceRevoked` (Task 3), `ctx.d1("ACCOUNTS_DB")` (Task 2).

- [ ] **Step 1: Wire the check after authorization succeeds**

In `handlers/ucan.rs`, `handle_inner`, between step 3 (authorize) and step 4 (serialize) insert the screen; the whole block is wasm-gated because `D1RevocationRegistry` is:

```rust
    // 3b. Screen the presented credentials against revoked devices.
    // Runs only after cryptographic authorization succeeded, and fails
    // open: registry trouble must never take sync down.
    #[cfg(target_arch = "wasm32")]
    screen_revoked(&body_bytes, ctx).await?;
```

And add at module level:

```rust
#[cfg(target_arch = "wasm32")]
async fn screen_revoked(
    body_bytes: &[u8],
    ctx: &RouteContext<()>,
) -> std::result::Result<(), ServiceError> {
    use crate::revocation::{self, RevocationVerdict, d1::D1RevocationRegistry};

    let presented = match revocation::collect_presented(body_bytes) {
        Ok(presented) => presented,
        Err(err) => {
            // The authorizer already accepted this container; a parse
            // failure here is a shape drift to surface, not a reason to
            // block the request.
            console_error!("revocation screen skipped, container unparseable: {err}");
            return Ok(());
        }
    };
    let registry = match ctx.d1("ACCOUNTS_DB") {
        Ok(db) => D1RevocationRegistry::new(db),
        Err(err) => {
            console_error!("revocation screen skipped, no ACCOUNTS_DB binding: {err}");
            return Ok(());
        }
    };
    let now_ms = Date::now().as_millis();
    match revocation::assess(&registry, &presented, now_ms).await {
        RevocationVerdict::Allowed => Ok(()),
        RevocationVerdict::AllowedFailOpen(reason) => {
            console_error!("revocation screen failed open: {reason}");
            Ok(())
        }
        RevocationVerdict::Revoked => Err(ServiceError::new(
            ErrorCode::DeviceRevoked,
            "a credential in the presented chain has been revoked",
        )),
    }
}
```

(`worker::*` is already glob-imported at the top of the file, which provides `Date`, `RouteContext`, and `console_error!`.)

- [ ] **Step 2: Compile both targets**

Run: `cargo check -p tonk-access-service && cargo check -p tonk-access-service --target wasm32-unknown-unknown`
Expected: clean. Native builds skip the screen entirely (the native helper server has no D1), so existing native integration tests are unaffected.

- [ ] **Step 3: Full workspace gate**

Run: `cargo clippy --workspace --all-targets --all-features && cargo fmt --check && cargo test -p tonk-access-service --features helpers`
Expected: clean, all tests PASS.

- [ ] **Step 4: Commit**

```bash
git add rust/tonk-access-service/src/handlers/ucan.rs rust/tonk-access-service/src/lib.rs
git commit -m "feat(tonk-access-service): reject presign requests from revoked devices"
```

---

### Task 9: Staging smoke checklist (manual, after deploy)

**Files:** none — operational verification. Record outcomes in the PR body.

- [ ] **Step 1: Baseline** — on staging (`staging.tonk.xyz` + `accounts-staging.tonk.xyz`), create an account in browser A, self-link browser B (two devices on one account), create a synced space from A, confirm B restores and can pull/push.
- [ ] **Step 2: Revoke** — from A, revoke B's device (until the device-management UI ships, hand-craft the `/devices/revoke` invocation or use a maintenance script). Within ~60 s (cache TTL), B's sync must start failing with HTTP 403 `DEVICE_REVOKED` on `/ucan/`; A must keep working.
- [ ] **Step 3: Device-only user unaffected** — a browser with no account (device DID only) claims an invite and syncs; confirm no regression (its chain matches nothing in the registry).
- [ ] **Step 4: Fail-open** — temporarily rename the binding in a staging-only deploy (`ACCOUNTS_DB` → `ACCOUNTS_DB_OFF`), redeploy, confirm sync still works for everyone and the worker log shows "revocation screen skipped"; restore the binding.
- [ ] **Step 5: Latency sanity** — compare `/ucan/` p50 before/after on staging traffic; the added cost must be one cached map probe on repeat requests and one indexed D1 query per cold credential set.

---

## Self-review notes

- Spec coverage: read-only D1 binding (Task 2), parse + collect (Task 4), CID+DID dual match (Tasks 6–7), 403 rejection (Tasks 3, 8), fail-open + cache TTL (Tasks 5–6, 8), entitlement seam (trait docs, Task 6), staging smoke incl. fail-open drill (Task 9). Sequencing note: this plan is independent of the account-service hardening plan (disjoint crates) and must land only after the roster-migration PR merges (it consumes nothing from it, but staging smoke assumes restore/migration are deployed).
- Type consistency: `PresentedCredentials::keys()` feeds `assess` → `cache_probe`/`revoked_of` all on `&[String]`; `revoked_query` binds the same key list once via numbered placeholders; `RevocationVerdict` names match between Tasks 6 and 8.
- Known execution risks called out in-plan: dialog pin drift (Task 1 STOP conditions), thread-local cache tests under a non-current-thread runtime (note in Task 6), clippy `async_fn_in_trait` (Task 6 Step 5).
