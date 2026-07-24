# Session Delegations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give revocation something to withhold. `root → device` is unexpiring, so withdrawing it can only ever be a registry lookup. A short-lived `device → session` delegation makes the credential that actually signs presigns lapse on its own, so a registry outage costs at most one session lifetime instead of unbounded access.

**Architecture:** The device keeps its `root → device` grant — that is what survives offline and what the registry records — and uses it to mint a bounded delegation to an ephemeral session key. The session key signs presign invocations; the composed chain still carries the device's hop, so the revocation screen sees the device identity exactly as it does today. Enforcement lives in the access-service's credential screen, which already re-parses the container.

**Tech Stack:** `dialog-ucan-core` (`DelegationBuilder::expiration`, `Timestamp`), `tonk-identity`, `tonk-access-service`, `dialog_common::test`.

## The finding this rests on

`Invocation::check` computes the intersection of every hop's time bounds and returns it as a `TimeRange`. `InvocationChain::verify` then does `.map(|_| ())`, and `UcanAuthorizer::authorize` never looks. **Expiry was therefore unenforced at the presign boundary**: a chain that expired last year verified exactly like a fresh one, and only a chain that could *never* be valid (an empty intersection) was rejected. An expiry nothing checks buys nothing, so enforcement had to land before sessions meant anything.

## Why this shape (decisions)

- **Self-minted, not service-issued.** The device signs the session itself from the grant it holds. No renewal endpoint, nothing to be unreachable, and no new hot-path dependency — which matters because the whole point is to reduce what an outage can cost.
- **12-hour TTL.** Hours, not minutes: a session must survive a stretch offline and a closed laptop, or renewal failure becomes the common path rather than the exceptional one. Short enough that a lost grant stops mattering within a working day. Tunable — `SESSION_TTL_SECONDS` in `tonk-identity/src/session.rs`.
- **Enforce, do not require.** The screen rejects a chain outside its window; it does not demand that a chain *have* a window. Unbounded chains keep working, so this is additive and can ship ahead of the clients that will start bounding themselves. Requiring bounded invocations is the breaking half and wants a soak first (see Task 5).
- **Screen, not authorizer.** Enforcement lives in `tonk-access-service`'s own credential screen rather than upstream in dialog. It reads the window off the parse the revocation screen already does, so it costs nothing extra, and it needs no upstream change to a pinned dependency.
- **Inclusive bounds.** A chain expiring exactly now is valid. Clients stamp `now + ttl`; an exclusive bound would give them a one-second cliff for no benefit.

## Global Constraints

- Lint gate: `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo fmt --check`.
- Tests: `#[dialog_common::test]`, names `it_does_x`.
- No stage/phase/PR references in code or doc comments.
- Conventional commits, scoped to the crate touched.

---

### Task 1: Enforce the presented time window — DONE

**Files:**
- Added: `rust/tonk-access-service/src/expiry.rs`
- Modified: `rust/tonk-access-service/src/revocation.rs`, `src/handlers/ucan.rs`, `src/lib.rs`

- [x] **Step 1:** `PresentedCredentials` carries `not_before` and `expires_at`, computed in `collect_presented` as the latest start and earliest end across the invocation and every delegation — the window every hop agrees on.
- [x] **Step 2:** `check_window` returns `Valid` / `Expired` / `NotYetValid`. Unbounded chains are `Valid`.
- [x] **Step 3:** The handler runs the window screen *before* the revocation screen, so an expired chain is refused without spending a D1 query, returning `401 INVOCATION_EXPIRED`.
- [x] **Step 4:** Tests — unbounded, inside, past expiry, before start, inclusive bounds, plus collection from a real session-shaped container (`it_reads_the_window_from_an_expiring_delegation`).

---

### Task 2: Mint session delegations — DONE

**Files:**
- Added: `rust/tonk-identity/src/session.rs`
- Modified: `rust/tonk-identity/src/lib.rs`

- [x] **Step 1:** `mint_session_delegation(device, session, ttl)` — subject-open like the grant it descends from, bounded by `expiration`.
- [x] **Step 2:** `extend_with_session(grant, device, session, ttl)` composes the grant with a fresh session hop, keeping the grant's CID in the chain so the revocation screen still sees the device.
- [x] **Step 3:** Tests — bounded audience, TTL respected, grant hop retained, and an unexpiring grant becoming bounded once extended.

---

### Task 3: Hold a session key in the client — DONE, but not as written

**Files:**
- Added: `rust/tonk-worker/src/session.rs`
- Modified: `rust/tonk-worker/src/worker.rs`, `src/router.rs`, `src/router/sync.rs`, `src/router/profile_name.rs`, `src/lib.rs`, `Cargo.toml`
- Upstream: dialog-db `feat/storage-clone` (#407)

**The seam this task assumed does not exist.** There is no point where the client "loads its `root → device` grant for sync" and could call `extend_with_session`. The presign invocation is signed by dialog's *operator* key, and its proofs are assembled by a `CertificateStore::prove` walk starting at the operator — the client never hands a grant to a signing call.

The `device → session` hop the plan wanted is already there structurally: it is `profile → operator`, minted unexpiring by `.allow(Subject::any())`. Bounding *that* is what bounds every chain the worker presents, and it keeps the device's identity in the chain exactly as the architecture paragraph intended.

- [x] **Step 1: Where the session key lives.** In memory, re-derived per session from the profile seed and a random context. Not persisted: `derive` is a KDF over profile seed plus caller-supplied context, so a random context is all it takes to get a fresh key, and there is no key-at-rest beyond the profile that already exists.
- [x] **Step 2: Mint on boot.** `session::open` derives the operator and claims `profile → operator` with a `SESSION_TTL_SECONDS` expiration instead of `.allow(Subject::any())`.
- [x] **Step 3: Renew before expiry.** The sync drain rotates when within `RENEWAL_MARGIN_SECONDS`. Rotation replaces the *key*, not just the delegation: certificates are content-addressed with no delete, and `prove` filters on the *requested* range — which the presign path leaves unbounded, and an unbounded requirement is satisfied by every range including a lapsed one. A re-mint under the same audience would sit beside the dead certificate and be picked about half the time.
- [x] **Step 4: Tests.** Seven in `session.rs`, including `it_authorizes_a_presign_chain_bounded_by_the_session` — the one that proves swapping `.allow()` for a bounded claim still resolves a real two-hop chain. The wasm suites *do* run locally: Chrome 150 at the default path plus nixpkgs chromedriver 150, `CHROMEDRIVER=… cargo test -p tonk-worker --target wasm32-unknown-unknown`. All 200 pass.
- [x] **Step 5: Commit** `feat(tonk-worker): sign presigns with a short-lived session delegation`

**Upstream dependency.** Rotation needs the replacement operator built over the *same* storage pool, or the reactor's cached repository and branch handles keep talking to the retired one. `OperatorBuilder::build` consumes its `Storage` and `Storage` was not `Clone`, so the pin moved to a rev carrying that one commit. See the implementation notes.

`tonk_identity::session::extend_with_session` (Task 2) is consequently unused by the client. It stays as the primitive for anything that does hold a grant directly — the CLI, which presents unbounded chains today.

---

### Task 4: Soak

- [ ] **Step 1:** Deploy Tasks 1–3 to staging and watch for `presign rejected: presented chain has expired` in the access-service logs. Any hit that is not an expected lapsed session means a client is stamping windows it cannot honour — find it before Task 5.
- [ ] **Step 2:** Confirm the CLI and any non-browser client still presign successfully. They present unbounded chains, which stay valid; a failure here means something is stamping expirations unintentionally.

---

### Task 5: Require bounded invocations — GATED ON THE SOAK

**Do not start before Task 4 has run clean for a meaningful period.** This is the breaking half: it refuses any chain that does not bound itself, which strands every client that has not adopted sessions.

- [ ] **Step 1:** Add a `WindowVerdict::Unbounded` case and refuse it.
- [ ] **Step 2:** Tests — an unbounded chain is refused, a bounded one is not.
- [ ] **Step 3:** Update the access-service README: presign invocations must carry a window.
- [ ] **Step 4: Commit** `feat(tonk-access-service): require presented chains to bound themselves`
