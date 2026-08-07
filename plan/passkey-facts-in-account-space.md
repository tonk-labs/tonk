# Portable passkey facts in the account space implementation plan

**Goal:** Make the hidden root-owned account repository the source of truth for passkey creation time and creation device, so those facts travel with the account instead of living only in the account service's D1 row.

**Approach:** Add one account-subject-keyed concept, `AccountPasskeyCreated`, alongside the existing `AccountDisplayName`. Seed it from this device's `tonk-local-root-v1` record whenever the account repository is ready and the fact is absent, which also repairs accounts whose creation invocation carried no metadata. The local worker's `/api/account/summary` prefers the space fact and falls back to the provider row, and renders passkey facts even when the provider is unreachable.

**Constraints:**
- The D1 `accounts.passkey_created_at` / `passkey_created_on` columns and the provider's `POST /account/summary` response stay exactly as they are. This change is additive; nothing is dropped and no row is backfilled by inference.
- **Fresh account creation keeps writing D1.** No task touches `rust/tonk-identity/src/ceremony.rs:247-256` (which binds `passkeyCreatedAt`/`passkeyCreatedOn` into the root-signed creation invocation) or `rust/tonk-account-service/src/core/accounts.rs:94` (which stores them). New accounts therefore land in both places, and the two values are expected to agree. They serve different purposes: the D1 row is the immutable claim fixed at creation inside a root-signed invocation and never updated, while the space fact is the portable one that any linked device can read without the provider. Keeping both means a later divergence is detectable rather than invisible.
- Passkey metadata remains informational. Nothing in root derivation, delegation bytes/CIDs, authorization, revocation, or account-repository authority may read or depend on it.
- `created_on` describes the browser/OS where Tonk ran `navigator.credentials.create()`, never the current password manager or storage provider.
- Account and device `created_at` values must never be substituted for passkey creation time. Absent metadata renders as an explicit unavailable state.
- The account space is only written when `AccountStateStatus::Ready`. An unconfigured or unhydrated account must never block the dashboard or the sweep.
- No new dependencies. No changes to `tonk-cli` (it has no account-summary path and never writes `AccountDisplayName`).

## File map

- `rust/tonk-schema/src/domain.rs`: `account::PasskeyCreatedAt` and `account::PasskeyCreatedOn` attributes.
- `rust/tonk-schema/src/account.rs`: the `AccountPasskeyCreated` concept and its constructor/accessor.
- `rust/tonk-schema/src/lib.rs`: re-export next to `AccountDisplayName`.
- `rust/tonk-worker/src/router/account_state.rs`: read and seed the space fact, wired into the ready sweep and the post-hydration path.
- `rust/tonk-worker/src/router/account_devices.rs`: provider-response type, source-preference merge, and provider-unavailable fallback in `summary`.
- `rust/tonk-worker-api/src/account.rs`: `AccountSummary::email` becomes optional.
- `rust/tonk-ui/src/account.rs`: render an absent email as unavailable without hiding passkey facts.
- `rust/tonk-ui/src/account_flow.rs`: real-browser coverage that the facts still render end to end.

## Task 1: Add the account-space passkey concept

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs:536-545` (`pub mod account`)
- Modify: `rust/tonk-schema/src/account.rs` (new concept after `AccountDisplayName`)
- Modify: `rust/tonk-schema/src/lib.rs:69`
- Test: `rust/tonk-schema/src/account.rs:31` (`mod tests`)

**Interfaces:**

Produces, in `domain.rs` `pub mod account`:

```rust
    /// Browser-reported Unix time in seconds, captured immediately after
    /// `navigator.credentials.create()` returned. `f64` because the value
    /// system stores numbers as `Float`; second-resolution Unix times convert
    /// losslessly at this magnitude. Cardinality-one: an account has one
    /// passkey creation moment, so concurrent linked-device writes converge on
    /// a deterministic winner rather than accumulating.
    #[derive(Attribute, Clone, PartialEq, PartialOrd)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct PasskeyCreatedAt(pub f64);

    /// The browser and operating system where passkey creation ran, e.g.
    /// `Chrome on macOS`. Never the password manager or storage provider —
    /// WebAuthn does not expose those reliably.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.account")]
    #[cardinality(one)]
    pub struct PasskeyCreatedOn(pub String);
```

Produces, in `account.rs`:

```rust
/// Facts Tonk recorded when it created this account's passkey, keyed by the
/// immutable account subject.
///
/// Informational only: no derivation, delegation, authorization, or revocation
/// path reads these. Both attributes are asserted in one transaction, so a
/// query requiring both never observes a half-written pair.
///
/// Derives `PartialOrd` but not `Ord`, because [`PasskeyCreatedAt`] wraps an
/// `f64` — the same shape `command::Invite` uses for its `TimeStamp`.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct AccountPasskeyCreated {
    /// The immutable account subject.
    pub this: Entity,
    /// Unix seconds at credential creation.
    pub created_at: PasskeyCreatedAt,
    /// Browser and operating-system label where creation ran.
    pub created_on: PasskeyCreatedOn,
}

impl AccountPasskeyCreated {
    /// Record creation facts on the account subject.
    pub fn new(account: Entity, created_at: u64, created_on: String) -> Self {
        Self {
            this: account,
            created_at: PasskeyCreatedAt(created_at as f64),
            created_on: PasskeyCreatedOn(created_on),
        }
    }

    /// Unix seconds, back in the integer form the wire DTO carries.
    pub fn seconds(&self) -> u64 {
        self.created_at.0 as u64
    }
}
```

The `use` line at `account.rs:6` becomes
`use crate::domain::account::{DisplayName, PasskeyCreatedAt, PasskeyCreatedOn};`.
`lib.rs:69` becomes `pub use account::{AccountDisplayName, AccountPasskeyCreated};`.

- [x] Add `wasm_bindgen_test_configure!(run_in_browser);` to `account.rs`'s `mod tests`, mirroring `rust/tonk-schema/src/membership.rs:174-176`. The mod currently has no configure line, so any test added to it would try to run under Node and fail `test:web:debug` with `failed to find or execute Node.js`.
- [x] Add `it_round_trips_passkey_creation_facts_on_the_account_subject`: on a `helpers::test_repo` branch, assert `AccountPasskeyCreated::new(account.this(), 1_754_380_800, "Chrome on macOS".into())`, then `select` with `this: Term::from(account.this())`, `created_at: Term::var("created_at")`, `created_on: Term::var("created_on")`; expect exactly one row, `seconds() == 1_754_380_800`, and `created_on.0 == "Chrome on macOS"`.
- [x] Add `it_keeps_one_passkey_creation_fact_per_account`: reuse the `converge(a_first)` helper at `account.rs:41` to write two different pairs on divergent branches, merge both orders, and expect one surviving row whose `seconds()` and `created_on` come from the *same* write in both orders. This is what proves the two attributes cannot be torn apart by cardinality-one merge.
- [x] Run `nix develop -c cargo test -p tonk-schema account::tests`; expect both to fail to compile with `cannot find type AccountPasskeyCreated`.
- [x] Add the two attributes, the concept, the constructor/accessor, and the re-export.
- [x] Run `nix develop -c cargo test -p tonk-schema account::tests`; expect success.
- [x] Run `nix develop -c cargo test -p tonk-schema`; expect success.

## Task 2: Seed the fact from this device's local root

**Files:**
- Modify: `rust/tonk-worker/src/router/account_state.rs` (new `passkey_facts` and `seed_passkey_facts`, plus call sites at `sync_ready:497` and `ensure_account_state_swept:555`)
- Test: `rust/tonk-worker/src/router/account_state.rs:1100` (`mod tests`, native)

**Interfaces:**

- Consumes: `crate::router::identity::local_root(tonk) -> Result<LocalRoot, _>` whose `passkey: Option<tonk_worker_api::PasskeyMetadata>` field (`identity.rs:41`) holds `{ created_at: u64, created_on: String }`; and `require_ready_account_state(tonk) -> Result<ReadyAccountBranch, _>`.
- Produces:

```rust
/// This account's passkey facts as recorded in the account space, absent when
/// the account is not ready or carries none.
///
/// Best-effort by design — the dashboard has an explicit unavailable state and
/// must not fail because a hidden system repository is mid-hydration. Every
/// `None` that is not simply "no fact" is logged, so an unreadable branch is
/// visible rather than silent.
pub(crate) async fn passkey_facts(tonk: &TonkState) -> Option<tonk_worker_api::PasskeyMetadata>;

/// Write this device's recorded passkey facts into the account space when it
/// has them and the space does not. Returns whether it wrote.
///
/// Idempotent: a device that only ever *evaluated* an existing root has
/// nothing to contribute and returns `false` without touching the branch.
pub(crate) async fn seed_passkey_facts(tonk: &TonkState) -> bool;
```

Both are plain `async fn` on all targets, not `#[cfg(wasm32)]`-gated like `converge_account_state`, so the native test below can drive them. Only the `sync_queue.mark_dirty` call inside `seed_passkey_facts` is cfg-gated, matching `adopt_account_display_name:813-814`.

`seed_passkey_facts` order of operations, which matters:
1. `require_ready_account_state`; on `Err`, return `false` without logging (unconfigured and unhydrated are ordinary states, hit on every sweep of a signed-out profile).
2. Query `AccountPasskeyCreated` for `ready.subject.this()`. On a query error, `log!` and return `false`. On a non-empty result, return `false` — an existing fact is never overwritten, so a device that later derives a different label cannot rewrite history.
3. `local_root(tonk).await` — on `Err`, return `false`. `.passkey` `None` → return `false`.
4. Assert `AccountPasskeyCreated::new(ready.subject.this(), metadata.created_at, metadata.created_on)` and commit on `MAIN_BRANCH`. On commit error, `log!` and return `false`.
5. `mark_dirty` (wasm only), return `true`.

Call sites:

- In `sync_ready`, between the `pull` at `:491-496` and `converge_account_state` at `:497`, so the seed sees remote facts before deciding and is included in the `push` at `:500`:
  ```rust
      if seed_passkey_facts(tonk).await {
          log!("recorded this device's passkey creation facts in the account space");
      }
  ```
- In `ensure_account_state_swept`, immediately before `converge_account_state` at `:555` (the post-hydration branch), with the same two lines. This is the path a freshly created account takes, where `sync_ready` has not run yet.

- [x] Add `it_seeds_passkey_creation_facts_from_the_local_root` to the native tests mod, cloning the setup of `it_mounts_hydrates_and_keeps_readiness_offline` at `account_state.rs:1101-1182` (access service, profile, `bootstrap_profile`, generated root, signed descriptor, minted device delegation, `persist_root`, `persist_link`, trusted marker). Change one thing: pass `passkey: Some(tonk_worker_api::PasskeyMetadata { created_at: 1_754_380_800, created_on: "Chrome on macOS".to_string() })` to `persist_root` instead of `None`. Then `ensure_account_state(&state).await == AccountStateStatus::Ready`, and query `AccountPasskeyCreated` on the ready branch expecting exactly one row with `seconds() == 1_754_380_800` and `created_on.0 == "Chrome on macOS"`.
- [x] Extend the same test to call `ensure_account_state` a second time and re-query, expecting still exactly one row with identical values — the seed must be idempotent across every sweep.
- [x] Add `it_seeds_nothing_when_the_local_root_has_no_passkey_metadata` with the identical setup but `passkey: None`, expecting `ensure_account_state` to reach `Ready` and the `AccountPasskeyCreated` query to return zero rows. This is the evaluated-root case, and it must degrade to the provider fallback rather than to a fabricated value.
- [x] Run `nix develop -c cargo test -p tonk-worker account_state`; expect both new tests to fail — the first with zero rows returned, the second failing to compile until `seed_passkey_facts` exists.
- [x] Implement `passkey_facts` and `seed_passkey_facts` with the five-step ordering above, and add the two call sites.
- [x] Run `nix develop -c cargo test -p tonk-worker account_state`; expect success.
- [x] Run `nix develop -c test:native:debug`; expect success.

## Task 3: Prefer the space fact in the account summary

**Files:**
- Modify: `rust/tonk-worker/src/router/account_devices.rs:93-119` (`summary`), plus a new provider-response type and merge function
- Test: `rust/tonk-worker/src/router/account_devices.rs:210` (`mod tests`)

**Interfaces:**

- Consumes: `account_state::passkey_facts` from Task 2.
- Produces:

```rust
/// The account service's `POST /account/summary` response.
///
/// Deliberately its own type rather than [`AccountSummary`]: the provider hop
/// and the local hop no longer carry the same shape, and decoding the provider
/// straight into the local DTO is what would silently re-couple them.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSummary {
    email: String,
    passkey: Option<PasskeyMetadata>,
}

/// Prefer the portable account-space fact; fall back to what the provider
/// recorded at account creation.
///
/// The provider row is not a legacy-only path. Every account still writes it at
/// creation, and it answers three live cases: an account created before the
/// space fact existed, a device that never held the passkey and so cannot seed
/// it, and a fresh account read in the window between account creation and the
/// first sweep that seeds the space.
fn merge_summary(
    email: String,
    space: Option<PasskeyMetadata>,
    provider: Option<PasskeyMetadata>,
) -> AccountSummary {
    AccountSummary {
        email,
        passkey: space.or(provider),
    }
}
```

`summary` reads `account_state::passkey_facts(&state)` before building the invocation, decodes the provider body into `ProviderSummary` instead of `AccountSummary`, and returns `merge_summary(provider.email, space, provider.passkey)`. `AccountSummary::email` stays `String` in this task; Task 4 changes it.

- [x] Add `it_prefers_the_account_space_passkey_fact_over_the_provider_row` covering all three combinations against `merge_summary` directly: space `Some(a)` + provider `Some(b)` → `a`; space `None` + provider `Some(b)` → `b`; both `None` → `None`. A pure function keeps this out of the service-worker harness, which has no provider to stub.
- [x] Run `nix develop -c test:web:debug -E 'test(account_devices)'`; expect a compile failure on the missing `merge_summary`. (`test:web:debug` builds the wasm test archive and runs `cargo nextest run --archive-file …`, appending whatever arguments follow — so nextest filter expressions work, but a bare `--` separator does not.)
- [x] Add `ProviderSummary`, `merge_summary`, and rewire `summary` to read the space first and decode the provider into `ProviderSummary`.
- [x] Run `nix develop -c test:web:debug -E 'test(account_devices)'`; expect success.
- [x] Run `nix develop -c cargo test -p tonk-worker`; expect success.

## Task 4: Render passkey facts when the provider is unreachable

This is the task that cashes in the portability. Until it lands, the space is the preferred source but the dashboard still fails whole when the provider is down, because `email` has no other home and `load_summary` treats any error as total.

**Files:**
- Modify: `rust/tonk-worker-api/src/account.rs:10-15` (`AccountSummary`)
- Modify: `rust/tonk-worker/src/router/account_devices.rs` (`summary`, `merge_summary`)
- Modify: `rust/tonk-ui/src/account.rs:197-216` (`render_summary`)
- Test: `rust/tonk-ui/src/account.rs:1248` (`mod tests`)

**Interfaces:**

- Produces: `AccountSummary { email: Option<String>, passkey: Option<PasskeyMetadata> }`. `email` is `None` only when the provider could not be reached and the space had facts to show; the email itself stays service-owned and is never mirrored into the space (it is the uniqueness key and the enumeration boundary).
- `merge_summary`'s first parameter becomes `Option<String>`.

`summary` control flow after the change:

```rust
    let space = super::account_state::passkey_facts(&state).await;
    match super::http::post_cbor(&endpoint, &body).await {
        Ok(response) => {
            let provider: ProviderSummary = serde_json::from_slice(&response.body)
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("parse account summary: {error}"))
                })?;
            Ok(Json(merge_summary(Some(provider.email), space, provider.passkey)))
        }
        // The account repository already answered the passkey question, so an
        // unreachable provider costs the email and nothing else. With no space
        // fact there is nothing to serve, and the caller keeps the real error.
        Err(error) if space.is_some() => {
            log!("account summary falling back to account-space facts: {error}");
            Ok(Json(merge_summary(None, space, None)))
        }
        Err(error) => Err(error),
    }
```

`render_summary` replaces `set_text(host, "#account-email-value", &summary.email)` at `:198` with a match that writes the address when present and `"Unavailable"` when absent, leaving the existing passkey arms at `:199-215` untouched. No markup or stylesheet change: `#account-email-value` already exists and already renders `Unavailable` on the total-failure path at `:228`.

- [x] Add `it_renders_passkey_facts_without_a_verified_email` to `tonk-ui`'s browser tests: build a host from the existing `host()` helper at `account.rs:1254`, call `render_summary` with `AccountSummary { email: None, passkey: Some(PasskeyMetadata { created_at: 1_754_380_800, created_on: "Chrome on macOS".into() }) }`, and expect `#account-email-value` to read `Unavailable`, `#account-passkey-device-value` to read `Chrome on macOS`, and `#account-passkey-created-value` to be non-empty and not equal to `Unavailable`.
- [x] Run `nix develop -c test:web:debug -E 'package(tonk-ui)'`; expect a type error on `email: None`.
- [x] Change the DTO, `merge_summary`'s signature, `summary`'s fallback arm, and `render_summary`'s email arm.
- [x] Run `nix develop -c test:web:debug -E 'package(tonk-ui)'`; expect success.
- [x] Run `nix develop -c cargo test -p tonk-worker-api -p tonk-worker`; expect success.

## Task 5: Verify the complete slice

- [x] Run `nix develop -c cargo fmt --all -- --check` and `git diff --check`.
- [x] Run `nix flake check` — the lint gate is workspace `clippy --all-targets --all-features` plus fmt, and `--all-features` compiles integration tests that per-crate runs skip.
- [x] Run `nix develop -c test:native:debug` and `nix develop -c test:web:debug`; expect success.
- [x] Run `nix develop -c build:web`; expect success. Confirm every new file is tracked first — an untracked file breaks the flake source snapshot.
- [x] Run `nix develop -c test:e2e`. `it_signs_up_through_the_account_panels` in `rust/tonk-ui/src/account_flow.rs` already asserts the verified email, a non-empty localized creation date, and a `Chrome on …` label; it must stay green, now served from the account space rather than D1.
- [x] Extend `account_flow.rs` with a second assertion after signup: read `/api/account/summary` twice and confirm the passkey values are byte-identical, proving the seed does not rewrite itself on the next sweep.
- [x] Re-read the diff and confirm no account `created_at` or device `created_at` is presented as passkey creation time, and that no code path outside the summary and the dashboard reads either new attribute.

## Explicitly deferred

- Dropping the D1 columns, the creation-time write, or the provider's `passkey` response field. Existing accounts have no other source, devices that never held the passkey cannot seed the space, and the creation-time row is the only copy fixed by a root-signed invocation. Removing any of it is a separate decision, not a follow-up implied by this plan.
- Per-credential modelling. `accounts.credential_id` is a single column and this concept is keyed on the account subject, so both still assume one passkey per account. Multiple passkeys need a credential-keyed entity and a rule for which one the dashboard shows; that belongs with the fan-out work in `plan/passkey-fanout.md`, not here.
- Mirroring the verified email into the account space. It is the uniqueness key and the enumeration boundary, and moving it would change the service's security model rather than just where a fact is stored.
- Backfilling `passkey_created_on` for accounts whose creating device is gone. Any value Tonk could synthesize would be a guess presented as a record.
- Recording authenticator attachment, backup eligibility, or AAGUID. Still deferred for the reasons in `plan/passkey-account-summary.md:143-147`.

## Findings during implementation

- **Cardinality-one merge is per attribute, not per concept.** Task 1's
  `it_keeps_one_passkey_creation_fact_per_account` was specified to prove that
  asserting both attributes in one transaction keeps them from being torn
  apart. It disproves it: two replicas that record *different* pairs converge
  to one value per attribute independently, and the surviving row paired one
  replica's `created_at` with the other's `created_on`. Convergence is
  order-independent, but the pair is not atomic.

  The write rules make this unreachable rather than merely unlikely:
  `evaluate_root` (`rust/tonk-identity/src/ceremony.rs:151`) records no
  metadata, so only the browser that ran `navigator.credentials.create()` ever
  holds a pair, and one account has one such pair to seed from any device. The
  test now pins what holds — identical concurrent seeds survive intact, and
  divergent ones still converge in either order — and the concept doc says
  plainly that per-attribute merge is what a second recorded pair would run
  into. Making the pair atomic means keying it on the credential rather than
  the account, which is the deferred per-credential modelling.
