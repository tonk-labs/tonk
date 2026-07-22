# Sync Traffic Reduction Implementation Plan

> **STATUS (2026-07-22): implemented, with one part of the design deliberately
> abandoned. Read this before following anything below.**
>
> Part A (Tasks 1–2) shipped as dialog-db PR #402. Task 3 (tag + pin bump) is
> still outstanding and must wait for that PR to merge; carry pins the same tag,
> so bump it in step. Part B (Tasks 4–6) shipped on `fix/polling`. Task 7's live
> browser verification has not been run.
>
> **The visible-idle backoff described below was rejected and is NOT in the
> code.** This plan specifies a consecutive-no-op streak that decays a visible
> page's cadence from 2s toward `SYNC_BACKOFF_CAP_MS` (30s). That was built,
> reviewed, and then removed: a visible tab always polls at 2s. Savings come from
> hidden tabs (a flat `SYNC_HIDDEN_INTERVAL_MS`) plus Part A's permit cache, and
> the intended next step is push-based invalidation over WebSockets rather than
> smarter polling — see this plan's own "Deferred" note, which became the
> direction. Consequently `noop_streak`, `record_drain_outcome`, `reset_backoff`,
> `SYNC_BACKOFF_CAP_MS`, the keepalive-exclusion predicate (which existed only to
> avoid resetting that backoff), and Task 5's entire `changed`/`before != after`
> plumbing do not exist. `sync_repository` and `drain_sync` keep their original
> signatures.
>
> Two things the plan did not anticipate were added: pending local work bypasses
> the quiet interval (so a hidden tab's own un-pushed commits don't wait out the
> interval), and the dirty set is kept separate from the retry set (a repo that
> only ever fails must not latch that bypass open).
>
> The traffic arithmetic below is roughly 2x optimistic. `publish_settled_status`
> calls `handle.fetch()` on every successful sync of every branch — a second
> Resolve beyond the pull's — so an idle drain costs ~4 requests per branch per
> repo, not 2. The reduction *ratio* is unaffected. Threading the pull's resolved
> upstream revision into the settled-status classification would halve remaining
> idle traffic again, and is the best follow-up lever in this repo.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut idle Cloudflare request volume from ~70k/day per open tab to a few thousand, without hurting active-collaboration latency.

**Architecture:** Three independent reductions. (A) In dialog-db, cache redeemed access-service GET permits so a no-op pull costs one request instead of two. (B/C) In tonk-worker, make the single drain gate (`SyncScheduler::may_drain`) enforce a dynamic quiet interval derived from page visibility and a consecutive-no-op streak — every drain path (self-scheduled loop, per-fetch debounce, keepalive, Background Sync) already flows through that gate, so both the loop *and* the 10s keepalive-ridden drains obey it.

**Tech Stack:** Rust (wasm32 service worker + native), dialog-db effect providers, web-sys Clients API.

## Why (investigation summary, 2026-07-22)

- The SW self-schedules a sync drain every `SYNC_LOOP_MS = 2_000` while any page holds a live subscription (`rust/tonk-worker/src/worker.rs:1321`). A backgrounded tab keeps SSE subscriptions alive, so it polls all day.
- Each drain pulls every open repo's upstream branch. Idle push already short-circuits before network (`revision.tree == base` guard in dialog-repository `push.rs`); idle pull does not — it resolves the remote revision cell every time.
- Every remote Resolve = **2 Cloudflare requests**: a POST to the access service redeeming the UCAN for a presigned URL (`dialog-remote-ucan-s3/src/site.rs` `redeem`, fresh `reqwest::Client`, no caching) + the presigned R2 GET. Presigned URLs are valid 1 hour (`DEFAULT_EXPIRES = 3600`) but discarded after one use.
- The page also POSTs `/api/sync?why=keepalive` every 10s (`rust/tonk-host/src/host.rs` `spawn_keepalive`) to keep the SW alive for SSE. Every fetch schedules a debounced drain, so gating only the loop would still leave one drain per 10s. Hence: gate in the scheduler, not in the loop.
- Arithmetic: ~2.5s/cycle × 2 req ≈ 70k req/day per always-open tab. Observed: 72k/day across 4 users. Cloudflare Workers free tier is 100k/day.

Expected after all three: hidden idle tab ≈ 1 drain/60s × 1 req = ~1.4k/day; visible idle tab ≈ 1 drain/30s × 1 req = ~2.9k/day. Active use unchanged (2s cadence, resets on any real traffic).

**Deferred (noted, not planned):** push-based invalidation — a Durable Object holding a WebSocket/SSE per client that pings when the remote revision cell moves, so clients pull only on change. Revisit if the quiet-interval numbers are still too high.

## Global Constraints

- dialog-db code must never mention tonk/slide/consumers (repo rule).
- All tests use `#[dialog_common::test]`, named `it_does_x`, grouped by behaviour.
- No `mod.rs` — `foo.rs` + `foo/` form.
- Wasm-runnable test mods MUST have `wasm_bindgen_test_configure!` (`run_in_service_worker` for tonk-worker).
- Conventional commits, no emojis.
- tonk PRs target `staging` (repo default), not `main`. dialog-db PRs target `main`.
- Lint gate is workspace `cargo clippy --all-targets --all-features` (native) + `cargo fmt --check` — wasm-gated code must not leave native-dead helpers.
- Only GET permits may be cached (PUT/DELETE presigns can bind payload-specific material).
- Constants: `PERMIT_TTL_SECONDS = 300` (dialog-db), `SYNC_HIDDEN_INTERVAL_MS = 60_000`, `SYNC_BACKOFF_CAP_MS = 30_000` (tonk).

## Repos and branches

- **Part A (Tasks 1–3):** `~/tonk/dialog-db`, branch `feat/permit-cache` off `main`.
- **Part B (Tasks 4–6):** `~/tonk/tonk-pulls` (this repo), branch `feat/sync-quiet-intervals` off `origin/staging`.
- Parts A and B are independent; land as two PRs. Task 3 (pin bump) happens only after the dialog-db PR merges and is tagged.

---

## Part A — dialog-db: permit cache

### Task 1: `PermitCache` module with TTL and GET-only storage

**Files:**
- Create: `rust/dialog-remote-ucan-s3/src/permit_cache.rs`
- Modify: `rust/dialog-remote-ucan-s3/src/lib.rs` (add `mod permit_cache; pub use permit_cache::{PermitCache, PermitKey, redeem_cached};`)
- Modify: `rust/dialog-remote-ucan-s3/Cargo.toml` (ensure `chrono = { workspace = true }` under `[dependencies]`; ensure dev-deps per testing conventions: `dialog-common = { workspace = true, features = ["helpers"] }`, `tokio = { workspace = true, features = ["macros", "rt"] }`, `wasm-bindgen-test = { workspace = true }`, and the `[lints.rust] unexpected_cfgs` block if the crate lacks it)

**Interfaces:**
- Produces: `PermitKey = (String, Vec<u8>)`; `PermitCache::shared() -> &'static PermitCache`; `PermitCache::lookup(&self, key: &PermitKey, now: DateTime<Utc>) -> Option<Permit>`; `PermitCache::store(&self, key: PermitKey, permit: &Permit, now: DateTime<Utc>)`; `PermitCache::invalidate(&self, key: &PermitKey)`; `redeem_cached(...)` (added in Step 6). Task 2 consumes all of these.

- [ ] **Step 1: Write the failing tests**

In `rust/dialog-remote-ucan-s3/src/permit_cache.rs` (module skeleton + tests; implementation methods stubbed with `todo!()` initially or written directly — TDD at file granularity is fine here since the crate won't compile with `todo!()` removed later):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn get_permit() -> Permit {
        Permit {
            url: "https://bucket.example/key?X-Amz-Signature=abc"
                .parse()
                .unwrap(),
            method: "GET".to_string(),
            headers: vec![],
        }
    }

    fn put_permit() -> Permit {
        Permit {
            method: "PUT".to_string(),
            ..get_permit()
        }
    }

    fn key(endpoint: &str, capability: &[u8]) -> PermitKey {
        (endpoint.to_string(), capability.to_vec())
    }

    #[dialog_common::test]
    fn it_returns_a_cached_permit_before_expiry() {
        let cache = PermitCache::new();
        let now = chrono::Utc::now();
        let k = key("https://access.example/ucan/", b"cap-a");
        cache.store(k.clone(), &get_permit(), now);
        let hit = cache.lookup(&k, now + TimeDelta::seconds(PERMIT_TTL_SECONDS - 1));
        assert_eq!(hit.map(|p| p.method), Some("GET".to_string()));
    }

    #[dialog_common::test]
    fn it_expires_a_permit_after_its_ttl() {
        let cache = PermitCache::new();
        let now = chrono::Utc::now();
        let k = key("https://access.example/ucan/", b"cap-a");
        cache.store(k.clone(), &get_permit(), now);
        assert!(
            cache
                .lookup(&k, now + TimeDelta::seconds(PERMIT_TTL_SECONDS))
                .is_none()
        );
    }

    #[dialog_common::test]
    fn it_keys_permits_by_endpoint_and_capability() {
        let cache = PermitCache::new();
        let now = chrono::Utc::now();
        cache.store(key("https://a.example/", b"cap-a"), &get_permit(), now);
        assert!(cache.lookup(&key("https://a.example/", b"cap-b"), now).is_none());
        assert!(cache.lookup(&key("https://b.example/", b"cap-a"), now).is_none());
    }

    #[dialog_common::test]
    fn it_never_stores_a_mutating_permit() {
        let cache = PermitCache::new();
        let now = chrono::Utc::now();
        let k = key("https://access.example/ucan/", b"cap-a");
        cache.store(k.clone(), &put_permit(), now);
        assert!(cache.lookup(&k, now).is_none());
    }

    #[dialog_common::test]
    fn it_invalidates_a_permit_on_demand() {
        let cache = PermitCache::new();
        let now = chrono::Utc::now();
        let k = key("https://access.example/ucan/", b"cap-a");
        cache.store(k.clone(), &get_permit(), now);
        cache.invalidate(&k);
        assert!(cache.lookup(&k, now).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p dialog-remote-ucan-s3 permit_cache` (from `~/tonk/dialog-db`)
Expected: compile FAILURE (types not defined yet) — that's the failing state.

- [ ] **Step 3: Write the implementation**

Top of `rust/dialog-remote-ucan-s3/src/permit_cache.rs`:

```rust
//! Cache of redeemed access-service permits.
//!
//! Every remote effect used to POST its UCAN invocation to the access
//! service and receive a fresh presigned URL, even though presigned URLs
//! stay valid for an hour — on a periodically syncing replica the redeem
//! round-trip doubled the cost of every idle poll. A GET permit addresses
//! a stable (endpoint, capability) pair, so it is cached here and reused
//! for [`PERMIT_TTL_SECONDS`]. Mutating permits (PUT/DELETE) can bind
//! payload-specific signing material, so they are never cached.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, TimeDelta, Utc};
use dialog_remote_s3::Permit;

/// How long a redeemed GET permit is reused before redeeming afresh.
/// Well under the service's hour-long presign validity, so a cached
/// permit is never presented close to its expiry.
pub const PERMIT_TTL_SECONDS: i64 = 300;

/// Cache key: access-service endpoint + the dag-cbor bytes of the
/// capability the permit was redeemed for.
pub type PermitKey = (String, Vec<u8>);

struct Entry {
    permit: Permit,
    expires_at: DateTime<Utc>,
}

/// TTL cache of redeemed GET permits, keyed by [`PermitKey`].
#[derive(Default)]
pub struct PermitCache {
    entries: Mutex<HashMap<PermitKey, Entry>>,
}

impl PermitCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-wide cache the providers share.
    pub fn shared() -> &'static PermitCache {
        static CACHE: OnceLock<PermitCache> = OnceLock::new();
        CACHE.get_or_init(PermitCache::new)
    }

    /// The cached permit for `key`, unless it has passed its TTL.
    pub fn lookup(&self, key: &PermitKey, now: DateTime<Utc>) -> Option<Permit> {
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(key)?;
        (now < entry.expires_at).then(|| entry.permit.clone())
    }

    /// Cache `permit` under `key`. Non-GET permits are dropped: a
    /// mutating presign can be payload-specific, so reuse is unsound.
    pub fn store(&self, key: PermitKey, permit: &Permit, now: DateTime<Utc>) {
        if permit.method != "GET" {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        // Opportunistic sweep keeps the map bounded by the working set.
        entries.retain(|_, entry| now < entry.expires_at);
        entries.insert(
            key,
            Entry {
                permit: permit.clone(),
                expires_at: now + TimeDelta::seconds(PERMIT_TTL_SECONDS),
            },
        );
    }

    /// Drop the entry for `key`, so the next redeem goes to the service.
    pub fn invalidate(&self, key: &PermitKey) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p dialog-remote-ucan-s3 permit_cache`
Expected: 5 tests PASS.

- [ ] **Step 5: fmt + clippy the crate**

Run: `cargo fmt -p dialog-remote-ucan-s3 && cargo clippy -p dialog-remote-ucan-s3 --all-targets`
Expected: clean.

- [ ] **Step 6: Add `redeem_cached` (the seam Task 2 wires in)**

Append to `permit_cache.rs` (above the tests mod):

```rust
use dialog_capability::{Capability, Constraint, Effect};
use dialog_remote_s3::S3Error;

use crate::site::{UcanAddress, UcanAuthorization};

/// Redeem `authorization` for a permit, reusing a cached GET permit for
/// the same (endpoint, capability) when one is still fresh. Returns the
/// permit together with its cache key so the caller can
/// [`invalidate`](PermitCache::invalidate) on a downstream failure.
pub async fn redeem_cached<Fx>(
    authorization: &UcanAuthorization,
    address: &UcanAddress,
    capability: &Capability<Fx>,
) -> Result<(Permit, PermitKey), S3Error>
where
    Fx: Effect,
    Fx::Of: Constraint,
    Capability<Fx>: serde::Serialize,
{
    let capability_bytes = serde_ipld_dagcbor::to_vec(capability)
        .map_err(|e| S3Error::Authorization(e.to_string()))?;
    let key: PermitKey = (address.endpoint().to_string(), capability_bytes);
    let now = DateTime::<Utc>::from(dialog_common::time::now());
    if let Some(permit) = PermitCache::shared().lookup(&key, now) {
        return Ok((permit, key));
    }
    let permit = authorization.redeem(address).await?;
    PermitCache::shared().store(key.clone(), &permit, now);
    Ok((permit, key))
}
```

Note: if `Capability<Fx>: serde::Serialize` needs different bounds in practice, mirror the bounds the provider impls in `src/provider/memory.rs` already carry — the capability is serialized into the UCAN invocation on that path, so serializability is already established; copy whatever bound set makes it compile.

- [ ] **Step 7: Compile + commit**

Run: `cargo clippy -p dialog-remote-ucan-s3 --all-targets && cargo fmt --check`
Expected: clean.

```bash
git add rust/dialog-remote-ucan-s3
git commit -m "feat(dialog-remote-ucan-s3): add TTL cache for redeemed GET permits"
```

### Task 2: wire the seven providers through the cache

**Files:**
- Modify: `rust/dialog-remote-ucan-s3/src/provider/memory.rs` (Resolve, Publish, Retract impls)
- Modify: `rust/dialog-remote-ucan-s3/src/provider/archive.rs` (Get, Put impls)
- Modify: `rust/dialog-remote-ucan-s3/src/provider/blob.rs` (Read, Import impls)

**Interfaces:**
- Consumes: `redeem_cached`, `PermitCache::shared().invalidate` from Task 1.
- Produces: no new interfaces — behavior change only (GET-bearing effects reuse permits for up to 300s; any effect that fails after a cached redeem invalidates its entry so the next attempt redeems afresh).

- [ ] **Step 1: Rewrite each provider impl to the cached pattern**

Current shape (all seven follow it):

```rust
invocation
    .authorization
    .redeem(&invocation.address)
    .await?
    .invoke(invocation.capability)
    .perform(&S3)
    .await
```

New shape (example: `Resolve` in `provider/memory.rs`; apply identically to all seven impls — `Resolve`/`Publish`/`Retract` in memory.rs, `Get`/`Put` in archive.rs, `Read`/`Import` in blob.rs, keeping each impl's existing return type):

```rust
let (permit, key) = crate::permit_cache::redeem_cached(
    &invocation.authorization,
    &invocation.address,
    &invocation.capability,
)
.await?;
let result = permit.invoke(invocation.capability).perform(&S3).await;
if result.is_err() {
    // A permit that failed downstream may be stale (revoked or
    // expired server-side); drop it so the next attempt redeems.
    crate::permit_cache::PermitCache::shared().invalidate(&key);
}
result
```

Notes:
- Non-GET permits are never stored (Task 1), so Publish/Retract/Put/Import redeem every time — the uniform call keeps the providers identical rather than special-casing methods.
- `blob.rs` impls may pass the capability differently (e.g. a nested address); keep the same `redeem_cached(authorization, address, capability)` triple — every `ForkInvocation` carries all three fields (`dialog-capability/src/fork.rs:36`).
- The `?` on `redeem_cached` relies on the same `From<S3Error>` conversions the current `redeem(...).await?` already uses per impl — no new error plumbing.

- [ ] **Step 2: Compile and run the crate's tests**

Run: `cargo clippy -p dialog-remote-ucan-s3 --all-targets && cargo test -p dialog-remote-ucan-s3`
Expected: clean, all tests pass.

- [ ] **Step 3: Run the UCAN end-to-end integration tests**

These exercise the full redeem path through real (local) S3 + access servers and prove the cache doesn't break pull/push/collaboration:

Run: `cargo test -p dialog-repository it_pushes_and_pulls_via_ucan it_collaborates_via_ucan_delegation`
Expected: PASS. (If the integration harness requires a feature flag or `nix develop`, follow how CI invokes `dialog-repository` tests — check `.github/workflows` in dialog-db.)

Cache-hit behavior is additionally proven live in Task 7 (redeem POST count visibly drops in the network tab).

- [ ] **Step 4: Full-workspace gate + commit**

Run: `cargo clippy --workspace --all-targets && cargo fmt --check` (from `~/tonk/dialog-db`)
Expected: clean.

```bash
git add rust/dialog-remote-ucan-s3
git commit -m "feat(dialog-remote-ucan-s3): reuse cached permits in effect providers"
```

- [ ] **Step 5: Open the dialog-db PR**

```bash
git push -u origin feat/permit-cache
gh pr create --repo dialog-db/dialog-db --base main \
  --title "feat(dialog-remote-ucan-s3): cache redeemed GET permits" \
  --body "Redeeming at the access service on every effect doubled remote round-trips; GET permits are valid for an hour, so cache them for 300s keyed by (endpoint, capability). Non-GET permits are never cached (payload-bound presigns). Failed downstream requests invalidate their entry."
```

### Task 3: tag dialog-db and bump the tonk pin

Only after the Task 2 PR merges.

**Files:**
- Modify: `~/tonk/tonk-pulls/Cargo.toml` (every `dialog-*` dependency pinned with `tag = "tonk-2026-07-17"` → the new tag)
- Modify: `~/tonk/tonk-pulls/Cargo.lock` (via cargo)

- [ ] **Step 1: Tag the merged commit** (maintainer action, from `~/tonk/dialog-db` on updated `main`)

```bash
git fetch origin && git tag tonk-2026-07-22 origin/main && git push origin tonk-2026-07-22
```

- [ ] **Step 2: Bump every dialog-* pin in tonk**

In `~/tonk/tonk-pulls/Cargo.toml`, replace all occurrences of `tag = "tonk-2026-07-17"` with `tag = "tonk-2026-07-22"`, then:

Run: `cargo update --workspace` (or `cargo update -p dialog-repository` and siblings) and `cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean build. **Reminder:** carry pins the same dialog-db tag — coordinate its bump too or invite claims break on version skew.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: bump dialog-db pin to tonk-2026-07-22 for permit caching"
```

---

## Part B — tonk-worker: quiet-interval gate (visibility + backoff)

### Task 4: scheduler backoff + visibility state, enforced in `may_drain`

**Files:**
- Modify: `rust/tonk-worker/src/worker.rs` — `SyncScheduler` struct (~line 651), its impl (~line 690), `may_drain` (~line 750), constants block (~line 842), and the wasm test mod `route_for_tests` (~line 352, scheduler tests start ~line 428)

**Interfaces:**
- Produces (all on `SyncScheduler`, wasm-gated like the rest): `fn quiet_interval(&self) -> f64`; `fn record_drain_outcome(&self, changed: bool)`; `fn reset_backoff(&self)`; `fn set_visible(&self, visible: bool)`. Constants `SYNC_HIDDEN_INTERVAL_MS: i32 = 60_000`, `SYNC_BACKOFF_CAP_MS: i32 = 30_000`. Tasks 5 and 6 consume these.

- [ ] **Step 1: Write the failing tests**

Add to the scheduler section of `route_for_tests` in `worker.rs` (same style as `it_never_overlaps_two_drains`; the scheduler's clock is injected, so these are deterministic):

```rust
#[dialog_common::test]
fn it_backs_off_after_consecutive_noop_drains() {
    let s = SyncScheduler::default();
    let t = s.next(0.0);
    assert!(s.should_drain(t, SYNC_DEBOUNCE_MS as f64));
    s.begin_drain();
    s.end_drain(1_000.0);
    s.record_drain_outcome(false);
    // One no-op: quiet interval is 4s (2s * 2^1), so 2s after the
    // drain end is refused, 4s is allowed.
    let t = s.next(2_000.0);
    assert!(!s.should_drain(t, 3_000.0), "backed-off gate must refuse early drains");
    assert!(s.should_drain(t, 5_000.0), "gate must allow once the interval passes");
}

#[dialog_common::test]
fn it_caps_the_backoff_interval() {
    let s = SyncScheduler::default();
    for _ in 0..10 {
        s.record_drain_outcome(false);
    }
    assert_eq!(s.quiet_interval(), SYNC_BACKOFF_CAP_MS as f64);
}

#[dialog_common::test]
fn it_resets_backoff_when_a_drain_finds_changes() {
    let s = SyncScheduler::default();
    s.record_drain_outcome(false);
    s.record_drain_outcome(false);
    s.record_drain_outcome(true);
    assert_eq!(s.quiet_interval(), 0.0, "a changed drain restores the active cadence");
}

#[dialog_common::test]
fn it_holds_drains_to_the_hidden_interval_while_hidden() {
    let s = SyncScheduler::default();
    s.set_visible(false);
    s.begin_drain();
    s.end_drain(0.0);
    let t = s.next(10_000.0);
    assert!(
        !s.should_drain(t, SYNC_HIDDEN_INTERVAL_MS as f64 - 1.0),
        "hidden pages must not drain at the active cadence"
    );
    assert!(s.should_drain(t, SYNC_HIDDEN_INTERVAL_MS as f64 + 1.0));
}

#[dialog_common::test]
fn it_resumes_the_active_cadence_on_becoming_visible() {
    let s = SyncScheduler::default();
    s.set_visible(false);
    s.record_drain_outcome(false);
    s.record_drain_outcome(false);
    s.set_visible(true);
    assert_eq!(
        s.quiet_interval(),
        0.0,
        "regaining visibility must clear both the hidden hold and the streak"
    );
}
```

- [ ] **Step 2: Run the wasm tests to verify they fail**

Run: `nix develop -c test:web:debug` (filter output for `worker`), or compile-check first with `cargo check -p tonk-worker --target wasm32-unknown-unknown`.
Expected: compile FAILURE (methods don't exist yet).

- [ ] **Step 3: Implement**

(a) Add two fields to `SyncScheduler` (keep `#[derive(Clone)]`, drop `Default` from the derive — see (b)):

```rust
    /// Consecutive drains that pulled no upstream change. Grows the
    /// quiet interval so an idle replica stops paying the active
    /// cadence; any real page traffic or a drain that lands changes
    /// resets it.
    noop_streak: std::rc::Rc<std::cell::Cell<u32>>,
    /// Whether any window client was visible at the last check. Hidden
    /// pages hold drains to [`SYNC_HIDDEN_INTERVAL_MS`] — a
    /// backgrounded tab keeps its SSE subscriptions (and the keepalive)
    /// alive, so subscription liveness alone can't tell "watching"
    /// from "abandoned overnight".
    visible: std::rc::Rc<std::cell::Cell<bool>>,
```

(b) Replace the derived `Default` with a manual impl (the derive would default `visible` to `false`, holding the first post-drain cycle to the hidden interval and breaking the existing cap-clock tests):

```rust
impl Default for SyncScheduler {
    fn default() -> Self {
        Self {
            generation: Default::default(),
            in_flight: Default::default(),
            loading: Default::default(),
            last_request_at: Default::default(),
            pending_since: Default::default(),
            cause: Default::default(),
            last_drain_end: Default::default(),
            stopped: Default::default(),
            noop_streak: Default::default(),
            visible: std::rc::Rc::new(std::cell::Cell::new(true)),
        }
    }
}
```

(c) New methods on the wasm-gated `impl SyncScheduler`:

```rust
    /// The enforced gap between drain completions, from visibility and
    /// the no-op streak. Zero while a page is visible and recent drains
    /// found changes — [`SYNC_COOLDOWN_MS`] stays the floor, so the
    /// active cadence is unchanged.
    fn quiet_interval(&self) -> f64 {
        if !self.visible.get() {
            return SYNC_HIDDEN_INTERVAL_MS as f64;
        }
        let streak = self.noop_streak.get();
        if streak == 0 {
            return 0.0;
        }
        let scaled = (SYNC_LOOP_MS as f64) * f64::from(1u32 << streak.min(4));
        scaled.min(SYNC_BACKOFF_CAP_MS as f64)
    }

    /// Record whether the drain that just finished moved any branch.
    fn record_drain_outcome(&self, changed: bool) {
        if changed {
            self.noop_streak.set(0);
        } else {
            self.noop_streak.set(self.noop_streak.get().saturating_add(1));
        }
    }

    /// Real page activity: restore the active cadence.
    fn reset_backoff(&self) {
        self.noop_streak.set(0);
    }

    /// Update the visibility reading. Regaining visibility also clears
    /// the streak so the first foreground drain runs promptly.
    fn set_visible(&self, visible: bool) {
        if visible && !self.visible.get() {
            self.reset_backoff();
        }
        self.visible.set(visible);
    }
```

(d) In `may_drain` (~line 766), replace the cooldown comparison:

```rust
        // Quiet period measured from the LAST DRAIN'S COMPLETION. The
        // floor is SYNC_COOLDOWN_MS; visibility and the no-op streak can
        // raise it (see quiet_interval) so idle and hidden replicas stop
        // paying the active cadence. Every drain entrypoint passes
        // through here — including the drains the page's keepalive
        // fetches schedule — so the quiet interval binds them all.
        let quiet = (SYNC_COOLDOWN_MS as f64).max(self.quiet_interval());
        self.last_drain_end.get().is_none_or(|end| now - end >= quiet)
```

(e) Constants, next to `SYNC_COOLDOWN_MS` (~line 872), same cfg gate:

```rust
/// Enforced drain gap while no window client is visible. A hidden tab
/// still keepalives and holds subscriptions, so without this it pays
/// the active cadence all night for changes nobody is watching.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_HIDDEN_INTERVAL_MS: i32 = 60_000;

/// Ceiling on the no-op backoff for visible pages: idle viewing decays
/// from the 2s cadence toward this, and any activity or landed change
/// snaps it back. Bounds the worst-case latency for seeing another
/// device's change on an idle-but-visible page.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_BACKOFF_CAP_MS: i32 = 30_000;
```

Note `SYNC_LOOP_MS` is `u64` and declared later in the file (~line 1321) — the cast in `quiet_interval` is fine; if ordering is an issue move the constant up beside the others.

- [ ] **Step 4: Run the wasm tests**

Run: `nix develop -c test:web:debug`
Expected: new tests PASS, existing scheduler tests still PASS (they exercise streak 0 + visible=true, where `quiet_interval() == 0` and behavior is unchanged).

- [ ] **Step 5: Native gate + commit**

Run: `cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean (all additions are inside existing wasm-gated blocks, so no native-dead code).

```bash
git add rust/tonk-worker/src/worker.rs
git commit -m "feat(tonk-worker): quiet-interval drain gate with visibility and no-op backoff"
```

### Task 5: thread drain outcomes into the scheduler

**Files:**
- Modify: `rust/tonk-worker/src/router/sync.rs` — `sync_repository` (~line 347), `drain_sync` (~line 952)
- Modify: `rust/tonk-worker/src/worker.rs` — every `drain_sync` call site (~lines 1203, 1207, 1237, 1298, 1420)
- Modify: `rust/tonk-worker/src/router.rs` — the two `sync_repository(...)` test call sites (~lines 1479, 1494)

**Interfaces:**
- Consumes: `SyncScheduler::record_drain_outcome` from Task 4.
- Produces: `pub async fn sync_repository(state: &AppState, repo: &str) -> Result<bool, String>` (true = some branch's local revision moved); `pub async fn drain_sync(state: &AppState) -> bool`.

- [ ] **Step 1: Change `sync_repository` to report movement**

Return type becomes `Result<bool, String>`; doc comment gains: "`Ok(true)` means at least one branch's local revision moved (a pull landed changes)." In the branch loop, accumulate:

```rust
    let mut failed = false;
    let mut changed = false;
    for branch in branches_to_sync(&info.branch) {
        // ... existing match on sync(...) ...
            Ok(Json(response)) if !response.success => { /* unchanged */ }
            Ok(Json(response)) => {
                changed |= response.before != response.after;
            }
            Err(e) => { /* unchanged */ }
    }
```

(`Revision` derives `PartialEq`, and a push leaves the local head where it was, so `before != after` isolates pulls that landed changes.) Early returns (`paused`, unknown repo) return `Ok(false)`. Final: `if failed { Err(...) } else { Ok(changed) }`.

- [ ] **Step 2: Change `drain_sync` to return the union**

```rust
pub async fn drain_sync(state: &AppState) -> bool {
    // ... existing dirty/open/order computation unchanged ...
    let mut changed = false;
    for repo in order {
        match sync_repository(state, &repo).await {
            Ok(moved) => changed |= moved,
            Err(e) => {
                log!("drain_sync: {repo} did not fully reconcile: {e}");
                let tonk = state.read().await;
                tonk.sync_queue.requeue(&repo, now);
            }
        }
    }
    changed
}
```

Update its doc comment to note the return value feeds the scheduler's backoff.

- [ ] **Step 3: Record outcomes at the call sites in `worker.rs`**

Wherever the scheduler is in scope, replace:

```rust
scheduler.begin_drain();
crate::router::drain_sync(&state).await;
scheduler.end_drain(js_sys::Date::now());
```

with:

```rust
scheduler.begin_drain();
let changed = crate::router::drain_sync(&state).await;
scheduler.end_drain(js_sys::Date::now());
scheduler.record_drain_outcome(changed);
```

That's the loop tick (~1298), `on_connectivity` (~1237), and the trailing-edge debounce future (~1420). The Background Sync `onsync` sites (~1203/1207): `self.sync_scheduler` is on the same struct — apply the same pattern if the scheduler is reachable there; if that path drains without the scheduler, discard with `let _ = crate::router::drain_sync(&state).await;` (Background Sync fires rarely; it must not grow the streak wrongly, and ignoring is safe).

- [ ] **Step 4: Fix the two router.rs test call sites**

`sync_repository(...).expect(...)` still compiles (Result changed only in `Ok` payload) — verify no other pattern-matching on the old `Ok(())` exists: `rg -n 'sync_repository' rust/`.

- [ ] **Step 5: Test + gate + commit**

Run: `nix develop -c test:web:debug` and `cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean, tests pass.

```bash
git add rust/tonk-worker/src
git commit -m "feat(tonk-worker): feed drain outcomes into the sync backoff"
```

### Task 6: visibility plumbing + keepalive exclusion

**Files:**
- Modify: `rust/tonk-worker/Cargo.toml` — add `"WindowClient"`, `"VisibilityState"` to the `web-sys` feature list (~line 69)
- Modify: `rust/tonk-worker/src/worker.rs` — `on_fetch` (~line 1093), the loop tick (~line 1294), the debounce future (~line 1395), new `any_client_visible` helper, new `onvisibility` export (mirror `on_connectivity` at ~line 1224)
- Modify: `rust/tonk-ui/assets/service_worker.js` — message handler (~line 334)
- Modify: `rust/tonk-ui/index.html` — beside the `sendConnectivity` wiring (~line 321)

**Interfaces:**
- Consumes: `set_visible`, `reset_backoff`, `quiet_interval` gate from Task 4.
- Produces: SW-exported `onvisibility()` (wasm_bindgen), page message `{type: "visibility"}`.

- [ ] **Step 1: `any_client_visible` helper in worker.rs** (wasm-gated, near `has_live_subscribers`):

```rust
/// Whether any window client of this SW is currently visible.
/// `clients.matchAll()` defaults to window clients. Errors read as
/// visible so a Clients API hiccup can never silently stall sync.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn any_client_visible() -> bool {
    use wasm_bindgen::JsCast;
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global().unchecked_into();
    let Ok(clients) = wasm_bindgen_futures::JsFuture::from(global.clients().match_all()).await
    else {
        return true;
    };
    let clients: js_sys::Array = clients.unchecked_into();
    clients.iter().any(|c| {
        c.dyn_into::<web_sys::WindowClient>()
            .map(|w| w.visibility_state() == web_sys::VisibilityState::Visible)
            .unwrap_or(false)
    })
}
```

(Adjust to `match_all`'s exact web-sys signature — it may return `Result<Promise, JsValue>`; unwrap accordingly with the same visible-on-error fallback.)

- [ ] **Step 2: Refresh visibility before both gates**

In the loop tick, before `if !scheduler.may_drain(...)` (~line 1294):

```rust
                scheduler.set_visible(any_client_visible().await);
```

In the debounce future, after the `SYNC_DEBOUNCE_MS` sleep and before `should_drain` (~line 1397):

```rust
        scheduler.set_visible(any_client_visible().await);
```

- [ ] **Step 3: Keepalive exclusion in `on_fetch`**

Beside the existing `schedule_sync_drain` call (~line 1093), where `path` is already computed:

```rust
        // Real page traffic restores the active sync cadence. The
        // keepalive/poke path is excluded: POST /api/sync exists to keep
        // the SW alive (and to ride the drain scheduling), not as
        // evidence anyone is doing anything — counting it would defeat
        // the idle backoff, since it fires every 10s forever.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if path != "/api/sync" {
            self.sync_scheduler.reset_backoff();
        }
```

- [ ] **Step 4: `onvisibility` export** (mirror `on_connectivity`, ~line 1224):

```rust
    /// A page became visible: restore the active cadence and reconcile
    /// immediately, instead of waiting out a hidden/backoff interval.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[wasm_bindgen(js_name = "onvisibility")]
    pub fn on_visibility(&self) -> Promise {
        self.sync_scheduler.set_visible(true);
        self.ensure_sync_loop();
        let state = self.state.clone();
        let scheduler = self.sync_scheduler.clone();
        future_to_promise(async move {
            if !offline() && scheduler.may_drain(js_sys::Date::now()) {
                scheduler.begin_drain();
                let changed = crate::router::drain_sync(&state).await;
                scheduler.end_drain(js_sys::Date::now());
                scheduler.record_drain_outcome(changed);
            }
            Ok(JsValue::UNDEFINED)
        })
    }
```

- [ ] **Step 5: Shim + page wiring**

`rust/tonk-ui/assets/service_worker.js`, in `self.onmessage` beside the `"connectivity"` branch (~line 334):

```js
    // A page became visible again — wake the worker so sync resumes the
    // active cadence immediately instead of waiting out a hidden interval.
    if (event.data && event.data.type === "visibility") {
        event.waitUntil?.(
            (async () => {
                try {
                    const worker = await activateWorker();
                    await worker.onvisibility?.();
                } catch (err) {
                    log("visibility dispatch failed:", err);
                }
            })(),
        );
        return;
    }
```

`rust/tonk-ui/index.html`, next to `sendConnectivity` (~line 321):

```js
            // Tell the worker when this page becomes visible so sync
            // drops its hidden/backoff interval without waiting for the
            // next keepalive to age past it.
            document.addEventListener("visibilitychange", async () => {
                if (document.visibilityState !== "visible") return;
                const reg = await navigator.serviceWorker.ready;
                reg.active?.postMessage({ type: "visibility" });
            });
```

- [ ] **Step 6: Update stale doc comments**

`SYNC_LOOP_MS`'s comment ("ticking this often is cheap") and `drain_sync`'s header still describe the flat 2s world — amend both to mention the quiet-interval gate.

- [ ] **Step 7: Test + gate + commit**

Run: `nix develop -c test:web:debug` and `cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean.

```bash
git add rust/tonk-worker rust/tonk-ui/assets/service_worker.js rust/tonk-ui/index.html
git commit -m "feat(tonk-worker): visibility-aware sync cadence with keepalive exclusion"
```

### Task 7: live verification + PR

- [ ] **Step 1: Serve the built UI locally** (use the repo's `run` skill / usual dev serving path), open a space, and in devtools' network tab filter for the access-service origin.
- [ ] **Step 2: Verify, over a 2-minute window:**
  - Foreground idle: request rate decays from ~2/2.5s to ~1 per 30s (backoff cap), single request per drain once the dialog-db pin (Task 3) is in.
  - Background the tab: rate drops to ~1 drain per 60s.
  - Refocus the tab: a drain fires within ~1s (visibility message), then 2s cadence briefly, decaying again.
  - Type/edit in a space: cadence snaps to 2s (fetch traffic resets backoff), the edit pushes promptly, and a second browser profile viewing the same space sees it within a few seconds.
- [ ] **Step 3: PR to staging**

```bash
git push -u origin feat/sync-quiet-intervals
gh pr create --base staging \
  --title "feat(tonk-worker): quiet-interval sync gate (visibility + idle backoff)" \
  --body "$(cat <<'EOF'
Four users were generating ~72k Cloudflare requests/day, dominated by the SW's flat 2s sync loop running for backgrounded tabs (2 requests per no-op pull: access-service redeem + R2 GET).

- may_drain now enforces max(cooldown, quiet_interval): hidden pages hold drains to 60s; consecutive no-op drains decay visible-idle cadence 2s -> 30s.
- Every drain path (loop, per-fetch debounce, keepalive-ridden drains, Background Sync) flows through the same gate.
- POST /api/sync keepalives no longer count as activity; any real fetch, a landed pull, or regaining visibility restores the 2s cadence.
- Page posts {type:"visibility"} on refocus for an immediate reconcile.

Companion: dialog-db permit cache (redeemed GET permits reused for 300s) halves the remaining per-drain cost; pin bumped separately.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review notes

- Active-latency invariant: `streak == 0 && visible` ⇒ `quiet_interval() == 0` ⇒ gate reduces to today's `SYNC_COOLDOWN_MS` — existing scheduler tests must pass unmodified; if any fails, the gate changed observable behavior for the active case and the implementation (not the test) is wrong.
- The keepalive still *schedules* drains (it must — it's what executes a due interval while the page is otherwise silent); it just doesn't *reset* the backoff.
- `set_visible(false)` does not clear the streak; `set_visible(true)` does — hidden→visible is the only transition that should snap the cadence.
- Part A alone halves traffic; Part B alone cuts cadence ~25×; together ~50×. Either can ship first.
