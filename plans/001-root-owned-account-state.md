# Plan 001: Finish the root-owned account state repository

> **Executor instructions**: Follow this plan in order. Treat each numbered PR
> as a review boundary and run its verification gate before starting the next.
> Do not weaken an invariant to get a test green. If a STOP condition occurs,
> stop and report it rather than improvising. When the full plan is complete,
> update `plans/README.md`.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 639cdb1f2..HEAD -- \
>   Cargo.toml Cargo.lock \
>   rust/tonk-account-service rust/tonk-access-service rust/tonk-cli \
>   rust/tonk-fab rust/tonk-host rust/tonk-identity rust/tonk-schema \
>   rust/tonk-ui rust/tonk-worker rust/tonk-worker-api
> git diff --stat -- docs/superpowers/specs/2026-07-27-account-profile-name-design.md
> ```
>
> The plan was written against commit `639cdb1f2` and the uncommitted,
> 2026-07-28 revision of the design document. If source changed, reconcile the
> excerpts below with the live code. If the design diff no longer describes the
> immutable descriptor, trusted-base gate, and atomic create-if-absent lifecycle,
> stop and ask which design is authoritative.

## Status

- **Priority**: P1
- **Effort**: L (five focused PRs)
- **Risk**: HIGH — identity transport, remote CAS, and cross-device merge paths
- **Depends on**: PR #650 / commit `e46799bd5` (already present)
- **Category**: direction / migration
- **Planned at**: commit `639cdb1f2`, 2026-07-28
- **Design of record**: `docs/superpowers/specs/2026-07-27-account-profile-name-design.md`

### Implementation progress

- **Status: DONE.** The five review boundaries were completed in order without
  hitting a STOP condition.
- PR 1: `d698ff068` — typed remote absence and atomic genesis publication.
  The ignored live test passed against the staging HTTPS `/ucan/` endpoint on
  2026-07-28, proving one conditional first-publish winner and winner
  preservation on retry.
- PR 2: `bf9ceb7df` — immutable signed descriptor transport and persistence.
- PR 3: `b2997a786` — trusted hidden `tonk:account` lifecycle and hydration.
- PR 4: `4d81dc35b` — authoritative display names and idempotent projection.
- PR 5: `aaca52408` — explicit legacy-account establishment and recovery.
  Staging revealed an obsolete required `devices.delegation_hex` column and a
  percent-encoded path bypass around hidden-repository routing. Migration 0004
  normalizes that deployed schema, and repository middleware now compares a
  decoded URI segment; both have regression coverage.

### Completion verification

The final worktree passed:

- `cargo fmt --check`;
- `cargo clippy --workspace --all-targets --all-features`;
- all focused account, identity, schema, worker API, CLI, account-service, and
  access-service test commands from the full gate;
- worker, UI, and FAB wasm test compilation;
- `nix develop -c test:native:debug`: 1,196 passed, 3 skipped;
- `nix develop -c test:web:debug`: 1,135 passed, 1 skipped (one passing test was
  reported as leaky by the harness).

Two narrow environment accommodations were retained: Darwin Nix uses the
repository's remarshal pin to avoid the host libffi crash, and the native-only
access-service integration test is excluded from wasm compilation.

### Staging record

The full gate ran against the staging HTTPS `/ucan/` provider using disposable,
isolated browser profiles, a CLI profile, and virtual passkeys. Temporary
Wrangler files selected the live replacement D1 database because the checked-in
staging UUIDs are stale; no infrastructure IDs or credentials were committed.
Migrations 0003 and 0004 and the account/access workers were deployed for the
gate.

Verified outcomes:

- new-account creation, initial authoritative name, and a Ready hidden account
  replica, including canonical and percent-encoded route denial;
- second-browser hydration and online/offline rename convergence;
- stale local roster repair, isolated projection failure, and boot-time retry;
- CLI link persistence and `account status` reaching Ready;
- legacy Unconfigured setup, one descriptor winner, and recovery after lost
  local state without a second initial-name seed;
- provider revocation enforcement after its documented 60-second cache bound,
  with the revoked credential receiving HTTP 403.

CDP cannot export PRF state between virtual authenticators. Device B's root
ceremony was therefore completed through device A's authenticator for B's
independently generated DID; B's own persistence, hydration, and convergence
remained real. This limitation is recorded rather than presented as physical
cross-authenticator passkey portability.

## Why this matters

Linked devices currently have one account root but independent profile-name
facts. A rename updates only the current device's profile branch and the space
rosters that device knows. This plan gives the account one root-owned,
syncable system repository containing a typed `AccountDisplayName`, then makes
the existing profile and roster names retryable projections of that fact.

The account service coordinates one signed repository descriptor but never
becomes the display-name authority. A ready device remains writable offline;
an unhydrated device cannot accidentally fork account history.

## Already complete — do not reimplement

Commit `e46799bd5` (`fix(tonk-worker): preserve member name on renewed join`)
implements delivery item 1 from the design:

- `rust/tonk-worker/src/router/join.rs:338` defines
  `membership_has_name`.
- `record_claim_on_content` queries `MemberName` before building its
  transaction and asserts a name only when the membership is unnamed.
- Tests cover an unnamed join, a renewed sequential join preserving the chosen
  name, and the non-linearizable concurrent-snapshot limitation.

Do not turn this guard into a lock or fold it into account-state work. It is an
independent fix. Keep its tests green as regression coverage.

## Current state and conventions

### Account links

- Browser worker: `rust/tonk-worker/src/router/account.rs` stores exact
  `root → device` delegation bytes at credential site
  `tonk-account-link-v1`, and separately saves the chain to the profile access
  store.
- CLI: `rust/tonk-cli/src/account.rs` duplicates the same credential-site and
  validation contract.
- Wire DTO: `rust/tonk-worker-api/src/account.rs::AccountLinkRequest` currently
  contains only `root_did` and `delegation_hex`.
- Legacy records are raw `DelegationChain` bytes. They must continue to decode
  as **linked but unconfigured** until the user completes the one-time
  descriptor ceremony.
- Empty bytes are the sign-out tombstone. Preserve that behavior.

### Ceremonies and service

- `rust/tonk-identity/src/ceremony.rs` builds five-minute root-signed account
  invocations and returns `AccountCeremony { root_did, device_did,
  delegation_hex, invocation_hex }`.
- `rust/tonk-account-service/migrations/0001_init.sql` stores accounts and
  devices; `0002_link_requests.sql` stores short-lived CLI handoffs.
- `Store::create_account_with_device` is already atomic across account and
  first-device insertion. Extend that transaction instead of adding a second
  write.
- Browser account creation and self-link live in
  `rust/tonk-ui/src/account.rs`; CLI consumes `/links/consume` in
  `rust/tonk-cli/src/account.rs`.

### Repositories and sync

- `Replica::new(profile, subject)` in
  `rust/tonk-schema/src/replica.rs:219` currently classifies every non-profile
  subject as `tonk:repository`. Add an explicit account constructor/kind; do
  not infer account kind from `subject != profile`.
- `record_replica_meta` in
  `rust/tonk-worker/src/router/repository.rs:2467` mounts a verifier-only
  replica, creates remotes/branches, and records it in the profile index, but
  currently always writes a real-space `Replica`.
- Real-space enumeration currently excludes only `subject == profile` in:
  `repository.rs::existing_space_labels`, `profile.rs::get_profile`, and
  `profile_name.rs::profile_space_keys`. These must instead require
  `kind == tonk:repository`.
- `drain_sync` in `rust/tonk-worker/src/router/sync.rs` syncs dirty repos plus
  every repo opened in the reactor. The account replica must be opened at boot
  so it joins this population without a rendered page.
- A successful reactor pull re-polls subscriptions, but no existing hook
  projects one repository into other branches. Add an explicit convergence
  hook.

### Remote semantics to prove before relying on them

The pinned Dialog revision is `2395873d9a0e764ca32853545d35159988b86e77`.
At that revision:

- `dialog-repository::Branch::fetch()` returns `Ok(None)` when the remote
  revision cell resolves to HTTP 404.
- `dialog-remote-s3` maps a first `Publish { when: None }` to
  `If-None-Match: *`.
- A failed conditional PUT maps HTTP 412 to
  `MemoryError::VersionMismatch`.
- Ordinary `Branch::push()` short-circuits when the local and upstream **tree**
  hashes are both the canonical empty tree. It therefore cannot provision an
  empty remote by itself. Provisioning must publish the locally-created empty
  `Revision` through the opened remote branch directly.

These are source observations, not a live guarantee. PR 1 proves them against
the real provider.

### Rust/test conventions

- Rust edition 2024; no `mod.rs`.
- Tests use `#[dialog_common::test]` and BDD names (`it_does_x`).
- Worker wasm test modules use
  `wasm_bindgen_test_configure!(run_in_service_worker)`; UI tests use
  `run_in_browser`.
- Local wasm execution may hang in this environment. Compile wasm tests
  locally; CI's `web` matrix executes them.
- Conventional commits, lowercase imperative subjects, no emojis.
- Do not change the Dialog pin unless PR 1 proves the required primitive is
  missing and a maintainer explicitly approves an upstream patch.

## Resolved implementation decisions

These decisions implement the design; do not reopen them during coding unless
one hits a STOP condition.

### Shared `tonk-account` crate

Create `rust/tonk-account`. It owns the shared account-link/descriptor contract,
pure lifecycle types, and low-level remote probe/create-if-absent helpers used
by worker and CLI. It must not become a generic settings/projector framework.
It exposes only:

- `AccountRepositoryDescriptorV1` encode/sign/validate/hash;
- versioned local `AccountLinkRecord` encode/decode with legacy fallback;
- `AccountStateStatus::{Unconfigured, Unhydrated, Ready}` and typed lifecycle
  outcomes/errors;
- account repository constants (`main`, `origin`, `tonk:account`, credential
  site names);
- the proven remote probe and atomic genesis publication operation.

`tonk-identity` consumes this crate to sign descriptors during passkey
ceremonies. The service, worker, UI DTO layer, and CLI consume the same parser;
none may implement a second descriptor validator.

### Descriptor V1 binary contract

Use a direct Ed25519 signature envelope, not the expiring account invocation:

1. Canonical payload bytes are DAG-CBOR encoding of the fixed tuple
   `(1_u64, account_subject_string, canonical_remote_string)`.
2. Sign
   `b"tonk/account-repository-descriptor/v1\0" || payload_bytes` with the
   passkey-derived root signer.
3. Canonical envelope bytes are DAG-CBOR encoding of the fixed tuple
   `(payload_bytes, signature_64_bytes)`.
4. The descriptor content hash is BLAKE3 of the exact canonical envelope bytes.

Validation must:

- reject an envelope over 4096 bytes and a remote over 2048 bytes;
- decode then re-encode both tuples and require byte equality (reject alternate
  encodings);
- require version exactly 1;
- parse `account_subject` as an Ed25519 `did:key`;
- verify the signature with that DID and the domain-separated payload;
- parse the remote with `url::Url`, allowing HTTPS and HTTP only for loopback
  test/development hosts;
- reject username/password, query, fragment, and non-canonical spelling;
- require the canonical URL string to end in `/` (the current endpoint is
  `<origin>/ucan/`).

V1 has one subject and one remote. Do not add generation, endpoint arrays,
provider migration, or rotation fields.

### Local account-link record

Keep credential site `tonk-account-link-v1` so existing installations are
found. New bytes are the canonical DAG-CBOR tuple
`(2_u64, delegation_bytes, descriptor_bytes_or_null)`. Decode rules:

- empty bytes => unlinked tombstone;
- V2 tuple => validate the delegation and, when present, the descriptor;
- otherwise try the legacy raw `DelegationChain` format and return a linked
  record with no descriptor.

The delegation and descriptor are persisted in this **one** credential record.
Saving the delegation to the access store remains a separate prerequisite:
save access first, then save the link record. If the final record save fails,
account discovery remains unconfigured even though harmless authority material
was retained; never write a record containing only one of delegation or
required descriptor for a new link.

### Service persistence and winner semantics

Add nullable `repository_descriptor BLOB` to `accounts` and nullable
`descriptor_hex TEXT` to `link_requests` in migration
`0003_account_repository_descriptor.sql`.

- New account creation validates and stores descriptor bytes in the existing
  account+first-device transaction.
- Existing-account establishment uses one SQL statement equivalent to:

  ```sql
  UPDATE accounts
  SET repository_descriptor = COALESCE(repository_descriptor, ?2)
  WHERE id = ?1
  RETURNING repository_descriptor;
  ```

  Return both the exact stored winner and `created: bool`. Only `created: true`
  may seed an initial display name.
- Browser self-link returns the established descriptor with the registered
  device. If an old account has none, return a typed conflict telling the user
  to complete account-state setup; never substitute the current origin.
- CLI handoff completion copies the account's established descriptor into the
  pending handoff atomically with device registration/delegation completion;
  `/links/consume` returns delegation and descriptor together.

The service stores and relays exact signed bytes. It never parses out a remote
for discovery and never stores the display name.

### Trusted-base marker

Use fixed credential site `tonk-account-trusted-base-v1`. Its value is exactly
the 32-byte descriptor content hash. It may be written only after:

1. a remote probe returns a concrete established revision and pull/reset
   adopts it successfully; or
2. this device wins atomic publication of the canonical empty genesis
   revision.

A mounted replica, configured remote, local genesis commit, timeout, 401/403,
5xx, malformed response, or unknown error never writes the marker. A marker for
a different descriptor hash does not grant readiness. Once matching, temporary
remote failure does not clear it.

### Canonical empty genesis and create-if-absent

For an unhydrated replica:

1. Fetch the remote `origin/main` revision.
2. `Ok(Some(revision))`: pull; if identical-tree merge is a no-op, reset the
   local branch to the fetched revision; then mark trusted.
3. `Ok(None)`: create a local empty transaction solely to obtain an
   authenticated `Revision`, then publish that revision directly through the
   opened remote branch. This is protocol genesis, not an account-state write.
4. Publish success: winner; retain the local revision and mark trusted.
5. `VersionMismatch`: loser; fetch the winner, reset/pull to it, then mark
   trusted.
6. Any other error: remain unhydrated. The local genesis may remain but is not
   writable account state and may be reset on retry.

Do not use normal push for step 3 and do not treat an arbitrary pull failure as
absence.

### Existing-account initial-name rule

The descriptor establishment response includes `created`. The browser asks the
worker to seed its current `ProfileName` only when `created == true`. New account
creation also requests that one seed. Self-links, CLI links, retries that return
an already-established winner, and ordinary boot never seed from local
petnames. If the winning device disappears before the write, the account stays
valid but unnamed until an explicit rename.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features` | exit 0, no warnings |
| Focused native tests | `cargo test -p tonk-account -p tonk-identity -p tonk-schema -p tonk-worker-api -p tonk-cli` | all pass |
| Account service tests | `cargo test -p tonk-account-service --features helpers` | all pass |
| Access live/helper tests | `cargo test -p tonk-access-service --features helpers` | all pass |
| Worker wasm compile | `cargo check -p tonk-worker --target wasm32-unknown-unknown --tests` | exit 0 |
| UI/FAB wasm compile | `cargo check -p tonk-ui -p tonk-fab --target wasm32-unknown-unknown --tests` | exit 0 |
| CI-equivalent native | `nix develop -c test:native:debug` | all pass |
| CI-equivalent web | `nix develop -c test:web:debug` | all pass; if local browser harness hangs, record compile success and rely on CI |

## Scope

### In scope

- Root workspace: `Cargo.toml`, `Cargo.lock`.
- New shared crate: `rust/tonk-account/**`.
- Descriptor ceremonies: `rust/tonk-identity/src/{ceremony,install,lib}.rs`,
  `rust/tonk-identity/Cargo.toml`.
- Account service descriptor persistence/transport:
  `rust/tonk-account-service/migrations/0003_account_repository_descriptor.sql`,
  `src/store.rs`, `src/store/{d1,sqlite}.rs`, `src/core/{accounts,links}.rs`, a
  focused descriptor core module, relevant handlers, `src/lib.rs`,
  `src/helpers/server.rs`, tests, and README.
- Browser/worker wire and lifecycle:
  `rust/tonk-worker-api/src/account.rs`,
  `rust/tonk-worker/src/router/{account,account_state,profile,profile_name,repository,sync}.rs`,
  `rust/tonk-worker/src/{router,worker,error}.rs`.
- Schema: `rust/tonk-schema/src/{account,domain,lib,replica}.rs`.
- Browser transport/migration UI: `rust/tonk-ui/src/{account,api,identity}.rs`
  and account markup/style only where needed.
- Actionable rename result: a narrow host helper in `rust/tonk-host` and the
  profile-name commit path in `rust/tonk-fab`.
- CLI descriptor/link/lifecycle: `rust/tonk-cli/src/{account,account_state}.rs`,
  `rust/tonk-cli/src/bin/tonk.rs`, and `rust/tonk-cli/Cargo.toml`.
- Provider compatibility documentation under `docs/`.

### Out of scope

- `rust/tonk-worker/src/router/join.rs` except adjustments strictly required by
  a changed shared type; do not redesign the landed name guard.
- Account root rotation/succession, descriptor updates, provider movement,
  multiple remotes, direct device-to-device handoff, or exported descriptors.
- A generic account settings map, projector registry, pending rename queue, or
  any account fact besides `AccountDisplayName`.
- Secrets, email, passkey material, billing, entitlements, provider-metered
  usage, blobs, or event logs in the account repository.
- A CLI display-name editing command or native background daemon.
- Replacing the account service for new-device enrollment or portable
  revocation.

## Git workflow

- Branch from the current feature branch, targeting `staging` when the stack is
  ready.
- Produce five reviewable PRs/commit groups in the order below. Do not squash
  descriptor transport, lifecycle, and display-name projection into one
  opaque commit.
- Match repository commit style, e.g.
  `feat(account): session delegations and signed revocation artifacts`.
- Do not push or open PRs unless instructed.

---

## PR 1 — Prove typed absence and atomic genesis publication

**Goal:** Turn the remote assumptions into tested APIs before any durable
account descriptor depends on them.

### Step 1.1: Create the shared crate and pure lifecycle result types

Add `rust/tonk-account` to the workspace. Define:

```rust
pub enum RemotePresence { Absent, Present(Revision) }
pub enum CreateGenesis { Winner(Revision), Loser(Revision) }
pub enum AccountStateStatus { Unconfigured, Unhydrated, Ready }
```

Add typed errors that preserve at least `Unauthorized`, `Unavailable`,
`Malformed`, and `Other` for diagnostics while treating all of them identically
for readiness (none authorizes initialization). Do not classify by substring in
the final API; preserve/match concrete Dialog/HTTP error variants where
available. If the pinned dependency collapses status before Tonk can inspect
it, retain an opaque `Remote` error and distinguish only `Ok(None)` from all
`Err` for the safety decision.

**Verify:** `cargo test -p tonk-account` → crate builds and pure result tests pass.

### Step 1.2: Prove local root-subject authorization

Write a native integration test using generated root, device, and operator
credentials:

- mint/store the existing subject-open `root → device` chain;
- mount a verifier-only repository whose subject is the root DID and whose
  routing key comes from `repo_key`;
- create/open `main` and commit an empty transaction;
- assert the resulting revision's subject is the immutable root DID;
- assert no root key is present after setup.

This proves root→device→operator authority can write a root-subject repository.

**Verify:** focused test passes on native and wasm test compilation remains clean.

### Step 1.3: Add helper-backed absence/CAS integration tests

Use the native `tonk-access-service` helper (real UCAN authorization and an
S3-compatible backing server), not a mocked `Branch::fetch`:

1. A never-published `origin/main` resolves as `RemotePresence::Absent`.
2. 401/403, a 5xx helper response, malformed access response, and an
   unreachable endpoint all return error, never `Absent`.
3. Two independent authorized devices start from absent, each create an empty
   local revision, and race direct remote-branch publication.
4. Exactly one succeeds; the other receives CAS/version mismatch, fetches the
   exact winning revision, resets/pulls to it, and reports `Loser(winner)`.
5. Repeating create is idempotent and never replaces the winner.
6. A later normal write from either hydrated device pulls/merges/pushes without
   non-fast-forward oscillation.

Add an ignored/live variant parameterized by
`TONK_ACCOUNT_REMOTE_URL`. Run it against the deployed staging R2 path before
calling the primitive complete. Record only status/result classes, never
credentials.

**Verify:** helper tests pass; live test demonstrates 404→absent and
conditional PUT→single winner.

### Step 1.4: Expose the smallest production helper

Implement the tested operations in `tonk-account`:

- `probe_remote_main(...) -> Result<RemotePresence, ...>`;
- `publish_genesis_if_absent(...) -> Result<CreateGenesis, ...>`.

The helper may use the public remote-branch API directly. Do not fork all of
Dialog into Tonk. If a small upstream Dialog change is required to preserve a
typed outcome, stop, present that patch separately, and wait for approval
before changing the workspace pin.

**PR 1 gate:**

```bash
cargo fmt --check
cargo clippy -p tonk-account -p tonk-access-service --all-targets --all-features
cargo test -p tonk-account
cargo test -p tonk-access-service --features helpers
```

Expected: all green, plus a recorded successful staging run.

---

## PR 2 — Add the durable descriptor and carry it through every link

**Goal:** New links cannot retain a delegation without the one authenticated
repository locator; existing raw links remain linked but unconfigured.

### Step 2.1: Implement descriptor and account-link codecs

In `tonk-account`, implement the exact descriptor and V2 local-record contracts
from “Resolved implementation decisions.” Include tests for:

- deterministic bytes/hash from the same inputs;
- wrong signer/subject/signature;
- unsupported version;
- alternate/non-canonical CBOR;
- oversized envelope/remote;
- URL credentials, query, fragment, unsupported scheme, production HTTP;
- descriptor subject mismatching the link delegation issuer;
- V2 round trip, legacy raw chain fallback, and empty tombstone;
- no expiry/timestamp field in descriptor bytes.

Expose validation as a parsed value plus canonical bytes/hash, so consumers do
not validate and then accidentally persist the caller's non-canonical input.

**Verify:** `cargo test -p tonk-account` → all codec and validation tests pass.

### Step 2.2: Extend root ceremonies

Update `tonk-identity`:

- account creation accepts a canonical default remote, signs
  `AccountRepositoryDescriptorV1`, and binds its exact descriptor hex into the
  existing five-minute `account/create` invocation;
- add `establish_account_repository(root, remote)` producing a descriptor plus
  a root invocation with command `account/repository/establish`;
- `link_device` and `complete_link` do **not** propose a descriptor;
- JS outputs include descriptor hex only for creation/establishment.

Update CDP ceremony tests to prove the descriptor is valid, non-expiring, and
bound to the same root as the delegation/invocation.

**Verify:** `cargo test -p tonk-identity` and UI wasm test compile pass.

### Step 2.3: Store one descriptor winner in the account service

Add migration 0003 and extend both D1 and SQLite stores. Implement one shared
core validator and set-if-absent operation. Update:

- account creation: require descriptor argument, validate subject == caller
  root, and store it in the atomic account/device transaction;
- `POST /devices/link`: require an established descriptor before inserting and
  return `{ descriptorHex }`;
- CLI completion: require established descriptor and atomically copy it into
  the handoff row; consume returns both exact values;
- `POST /account/repository/establish`: root-authorized, validates candidate,
  stores only if absent, and returns `{ descriptorHex, created }` where bytes
  are always the stored winner.

Concurrency tests must run two different valid candidates with `join!` and
assert both responses contain exactly one stored winner. Add rollback tests:
failed descriptor validation or device insertion leaves no partial account/link
state.

Update the native helper routes and full HTTP ceremony test; do not test only
core functions.

**Verify:** `cargo test -p tonk-account-service --features helpers` → all store,
core, and HTTP tests pass.

### Step 2.4: Transport and persist descriptor in browser and CLI

Update shared DTOs and clients:

- `AccountLinkRequest` carries delegation + descriptor together for new links;
  keep root DID derived/checked rather than trusting duplicate editable state.
- `AccountStatus::Linked` includes computed account-state status.
- Browser creation reads the current top origin and proposes canonical
  `<origin>/ucan/`; service responses, not ceremony candidates, provide bytes
  to local persistence.
- Browser self-link persists the descriptor returned by `/devices/link`.
- CLI `/links/consume` parses both fields and performs one V2 credential-record
  save.
- Worker and CLI reject a descriptor whose account subject differs from the
  delegation issuer or whose delegation audience differs from the current
  profile.
- Legacy raw records continue to report Linked/Unconfigured.

At this PR boundary, descriptor persistence is complete but account-repository
hydration may still report `Unhydrated`; do not fake readiness.

**PR 2 gate:**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features
cargo test -p tonk-account -p tonk-identity -p tonk-worker-api -p tonk-cli
cargo test -p tonk-account-service --features helpers
cargo check -p tonk-worker -p tonk-ui --target wasm32-unknown-unknown --tests
```

Expected: all green; HTTP tests prove browser/self-link/CLI receive stored bytes,
not a page default.

---

## PR 3 — Mount the `tonk:account` replica and enforce trusted hydration

**Goal:** Every configured device has the same hidden system replica; only a
trusted base permits account-state writes.

### Step 3.1: Add explicit account replica kind and exact space filters

In `tonk-schema::Replica` add:

- `ACCOUNT = "tonk:account"`;
- `account_kind()`;
- an explicit constructor/factory that preserves the same `(profile, subject)`
  entity but accepts only the three known kinds.

Keep `Replica::new` behavior for profile/real-space call sites. Account mount
must call the explicit account constructor.

Change all space enumerators to pin the query to
`Replica::repository_kind()` rather than filtering only the self subject:

- `repository.rs::existing_space_labels`;
- `profile.rs::get_profile`;
- `profile_name.rs::profile_space_keys` (rename to `real_space_keys`);
- any Hub/FAB switcher input derived from `ProfileInfo`.

Harden direct controls:

- remove and pause require the indexed kind to be `tonk:repository`;
- invite/template/restore/roster migration paths cannot accept account kind;
- the old `repo-vs-profile` migration must not overwrite an already-stamped
  account kind;
- normal user-space listing, pause, and removal tests include an account row
  and prove it is absent/ineligible.

Do not add the account replica to the account-service chain-backup/restore
artifact list; it is discovered from the descriptor.

**Verify:** schema tests plus worker wasm compile; tests prove only real spaces
are enumerated.

### Step 3.2: Implement worker `ensure_account_state`

Create focused `router/account_state.rs` rather than growing
`repository.rs`. The worker adapter must:

1. Load the versioned account-link record. Invalid/missing descriptor =>
   `Unconfigured` without touching repository state.
2. Derive subject/routing key solely from descriptor account subject.
3. Mount/load a verifier-only repository with explicit `tonk:account` kind,
   `origin` from descriptor, and `main -> origin/main`.
4. Record account replica/meta/profile-index facts without roster, standard
   library, template, invite, backup, or real-space status behavior.
5. Acquire/open `main` in the reactor even when no page renders it.
6. Compare trusted-base marker to descriptor hash.
7. Matching marker => `Ready`; attempt normal sync but never clear readiness on
   failure.
8. Missing/mismatched marker => run PR 1 probe/create lifecycle exactly. Mark
   ready only on the two allowed outcomes.

Factor pure decisions so timeout/401/403/5xx/malformed tests run natively. Wasm
integration tests use a controllable helper remote and assert no account fact
commit can acquire a writable handle while unhydrated.

Add a narrow `ready_account_branch`/`require_ready_account_state` API. Every
account fact write in later PRs must go through it; mounting APIs return no
unguarded mutation handle to callers.

**Verify:** lifecycle tests cover unconfigured, every unhydrated error, winner,
loser, present pull, matching marker offline, and marker mismatch.

### Step 3.3: Integrate worker boot and background sync

- After `bootstrap_profile` and state wrapping in `worker.rs`, dispatch
  `ensure_account_state` before space restore. Do not hold a write lock across
  network I/O.
- A ready/just-hydrated account branch remains open in the reactor, so
  `drain_sync` includes it.
- Account replicas bypass user pause checks and are never stamped with a user
  pause preference.
- After a successful account-repository pull, call the convergence hook added
  in PR 4. Add the hook point now as a no-op or focused callback; do not rely on
  subscription frames.
- Queue/retry account sync through the existing `SyncQueue`; an unavailable
  remote remains retryable and never changes descriptor/marker.

Tests prove boot opens account `main`, normal background sync sees it, and
offline readiness survives worker replacement.

### Step 3.4: Implement CLI ensure-on-link/on-demand

Create `tonk-cli/src/account_state.rs`:

- store account-system repository bytes under
  `SpotStore::open()`'s state root in a dedicated `account/` directory, never
  under `spots/` and never in `spots.json`;
- build a stable account operator context (new constant, never reuse/change the
  historical spot operator context) over that directory;
- mount the same root-subject verifier repository and run the shared
  probe/create lifecycle;
- store trusted marker through the shared profile credential site;
- run ensure immediately after CLI link persistence and before any future
  account-state operation;
- expose readiness in `tonk account status`.

The CLI has no daemon and no display-name command. Do not add either.

### Step 3.5: Document provider compatibility

Add a short provider contract under `docs/` stating that a compatible V1 remote
must:

- implement typed missing-cell reads and conditional first publish;
- preserve CAS/non-fast-forward semantics;
- authorize root-subject repositories through root→device→operator chains;
- enforce device revocation for every presented device hop (or explicitly be
  documented as weaker); Tonk's provider uses registry screening and fails
  closed when no valid cached verdict is available;
- accept that account facts are provider-visible unless the repository layer
  later adds E2E encryption.

**PR 3 gate:**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features
cargo test -p tonk-account -p tonk-cli -p tonk-schema
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
nix develop -c test:native:debug
```

Expected: all green; account replica is absent from every user-space list and
ready/offline behavior is covered.

---

## PR 4 — Make `AccountDisplayName` authoritative and project it everywhere

**Goal:** A linked ready rename writes one account fact; every device cache and
known real-space roster converges independently and idempotently.

### Step 4.1: Add the typed fact and verify merge semantics first

Create `tonk-schema/src/account.rs`:

```rust
AccountDisplayName {
    this: Entity, // immutable account subject
    name: account::DisplayName,
}
```

Define `xyz.tonk.account/display-name` explicitly with
`#[cardinality(one)]`. Export only this account fact; add no registry/settings
map.

Before wiring production rename, write a two-replica integration test:

1. both replicas start from the same base;
2. each writes a different name without seeing the other;
3. merge/pull/push in A→B and B→A order produces the same value;
4. repeated sync does not oscillate;
5. a subsequent rename after convergence supersedes the winner.

Document the observed deterministic winner in the test assertion without
claiming wall-clock latest-write-wins.

**Verify:** `cargo test -p tonk-schema <test-name>` passes in both orderings.

### Step 4.2: Implement explicit, idempotent convergence

Add `converge_account_state` and direct V1
`adopt_account_display_name` in worker account-state code:

1. Require Ready and read the one account fact from account `main`.
2. If absent, return successfully without guessing a name.
3. Query local `ProfileName`; commit only when stale.
4. Enumerate only `kind == tonk:repository` targets.
5. For each target independently, query the root-keyed `MemberName` and any
   obsolete device-keyed row.
6. Commit only if root name is absent/stale or device row exists. Retract the
   obsolete row in that same target transaction.
7. Continue after one target failure and report/log it; never skip later
   targets because the local `ProfileName` already matches.
8. Refresh `state:self` overlays for targets affected by a local-cache or
   roster change. Overlay writes are transient; durable no-op targets receive
   no commit.
9. Mark every changed real space dirty for sync.

Return a report (`profile_changed`, changed keys, failed keys) so tests and the
rename endpoint can assert exact behavior.

Run convergence:

- immediately after first readiness;
- during linked-device boot when already ready;
- after a local account-name commit;
- after every successful background/manual pull of the account repo.

Tests must include a failed middle space followed by a successful later space,
then a retry healing only the failed target with no account fact change.

### Step 4.3: Replace linked rename flow with a ready-only account write

Extract the existing local rename/projection body from
`repository.rs::run_profile_rename` into shared operations.

- Unlinked profile: preserve today's local `ProfileName` + roster + overlay
  behavior exactly.
- Linked Ready: assert `AccountDisplayName(account_subject, name)` on account
  `main`; queue account repo; run convergence; queue changed real spaces. Do not
  wait for push.
- Linked Unconfigured/Unhydrated: perform **no** account commit, local fallback,
  roster write, or pending intent. Return a typed 503
  `account_state_unavailable` with copy directing the user to `/account`.
- Whitespace-only remains a no-op.

Add `POST /api/account/display-name` with a shared DTO and return the committed
name plus convergence report. The existing command handler calls the same core
function for compatibility, but the shipped FAB must use a result-bearing host
call:

- add a narrow `tonk-host` helper that posts through the sealed guest's existing
  fetch relay;
- on success, let the profile subscription paint the committed name;
- on failure, restore the last subscribed value in `<ui-profile-name>` and show
  an actionable message (not a phantom successful edit).

Do not add a generic host RPC framework.

### Step 4.4: Seed only explicit creation winners

After browser account creation has persisted its returned descriptor, ask the
worker to initialize from the current `ProfileName`. The worker:

- ensures Ready;
- writes only if `AccountDisplayName` is absent;
- uses `resolve_display_name`, never service `device_name`;
- then runs ordinary convergence/sync.

Normal self-link and CLI link pass no initialization flag.

**PR 4 gate:**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features
cargo test -p tonk-schema -p tonk-account -p tonk-worker-api
cargo check -p tonk-worker -p tonk-fab -p tonk-ui --target wasm32-unknown-unknown --tests
nix develop -c test:web:debug
```

Expected: divergent merge test is deterministic; wasm tests cover ready rename,
all unready rejections, independent projection, no-op targets, and retry.

---

## PR 5 — Establish descriptors for existing linked accounts

**Goal:** Existing raw account links gain exactly one descriptor through an
explicit root/passkey ceremony, with no boot-time name election.

### Step 5.1: Add the migration state to the account page

When `AccountStatus::Linked` reports `Unconfigured`, show a dedicated “Finish
account setup” panel. On explicit confirmation:

1. Read the current top-page origin and canonicalize `<origin>/ucan/`.
2. Prompt for the passkey and call
   `establishAccountRepository({ remote })`.
3. Submit the root-signed candidate to
   `/account/repository/establish`.
4. Persist **the response's stored winner bytes**, never the candidate held by
   the page.
5. Ask worker ensure to hydrate/create.
6. Seed current profile name only when response `created == true` and readiness
   was acquired in this attempt.
7. Report Unhydrated with retry guidance; never choose another remote.

If local persistence fails after service acceptance, a later ceremony returns
the same winner with `created == false`. It persists that winner but does not
guess/replay an initial name.

### Step 5.2: Pin migration and concurrency behavior in tests

Add tests for:

- legacy raw link => Linked/Unconfigured, no account repo initialization;
- two valid descriptor candidates => one stored winner on every response;
- losing browser persists winner bytes and does not seed its name;
- ordinary boot with descriptor but no `AccountDisplayName` leaves account
  unnamed;
- accepted descriptor followed by local failure can be recovered by a later
  login/device without ephemeral provisioning authority;
- another authorized device can win remote creation after establishing browser
  disappears;
- new account uses initiating device's current profile name; account-service
  `device_name` never appears in the account fact;
- browser and CLI linking to an unestablished old account fail actionably rather
  than deriving a remote from their own origin.

### Step 5.3: End-to-end two-device staging gate

Using two clean browser profiles and one CLI profile:

1. Create a new account on device A; verify account replica is hidden and Ready.
2. Rename on A while online; link B; B displays/adopts the same name.
3. Take remote offline after both are Ready; rename on A; local UI/rosters
   update and account branch is ahead.
4. Restore remote; verify B adopts the new name and every stale local-only
   space B knows is repaired.
5. Force one projection target failure, retry without another account rename,
   and verify only that target receives a durable write.
6. Exercise an old legacy link: establish descriptor, lose local page state,
   repeat ceremony, and verify the exact winner is recovered without a second
   initial-name seed.
7. Link CLI; verify descriptor+delegation persist together and account status
   reports Ready; verify no CLI display-name command/daemon was introduced.
8. Revoke a device and verify the configured provider rejects its account-repo
   presigns under the documented revocation contract.

Record remote URL class/environment and outcomes in the PR description, not
credentials or account secrets.

**PR 5/full gate:**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features
cargo test -p tonk-account -p tonk-identity -p tonk-schema -p tonk-worker-api -p tonk-cli
cargo test -p tonk-account-service --features helpers
cargo test -p tonk-access-service --features helpers
cargo check -p tonk-worker -p tonk-ui -p tonk-fab --target wasm32-unknown-unknown --tests
nix develop -c test:native:debug
nix develop -c test:web:debug
```

Expected: all automated gates green (or wasm execution green in CI if the local
harness hangs), plus the staging checklist recorded.

## Test plan summary

Required regression matrix across the five PRs:

- Descriptor: canonical bytes/hash, signer/subject/version/URL/size failures,
  durable non-expiring artifact.
- Service: atomic account creation, descriptor set-if-absent winner, exact
  browser/CLI relay, no partial link.
- Remote: exact absent vs every error class, single CAS winner, loser adoption,
  root-subject authorization.
- Lifecycle: mount != ready, trusted marker events, ready offline writes,
  descriptor mismatch, boot-open sync population.
- Kind: account hidden from profile/Hub/FAB and refused by roster migration,
  pause, remove, templates, invite, and restore enumeration.
- Rename/projection: unlinked unchanged; ready linked authoritative write;
  unready rejection without fallback; every target independently checked;
  unchanged targets no durable writes; failed target retry.
- Merge: two divergent names converge in both orders and later write
  supersedes.
- Migration: only set-if-absent winner seeds; boot never guesses; lost local
  persistence recovers descriptor but does not replay initial name.
- Portability: routing key remains based on immutable account subject; provider
  revocation requirements documented.

## Done criteria

All must hold:

- [x] PR 1 live gate proves typed absence and atomic first publish against the
      deployed remote; no fallback treats arbitrary errors as absence.
- [x] New account links persist one validated V2 record containing delegation
      and exact service-returned descriptor.
- [x] Existing legacy links remain usable as Linked/Unconfigured until explicit
      establishment.
- [x] Exactly one descriptor winner is stored per account and only root-signed
      valid candidates are accepted.
- [x] Trusted-base marker is set only by successful pull/adoption or CAS winner.
- [x] `tonk:account` is never returned as a user space and cannot be paused or
      removed through space controls.
- [x] Worker boot opens account `main`; CLI ensures it without adding a daemon.
- [x] Linked Ready rename writes `AccountDisplayName`; unready linked rename
      returns actionable failure and writes nowhere.
- [x] Convergence checks every real space independently and performs no durable
      write to already-correct targets.
- [x] Divergent name writes converge deterministically in both orders.
- [x] Initial name is seeded only by new-account creation or the winning
      existing-account establishment attempt, from current `ProfileName`.
- [x] No files implement another account fact, generic settings map, projector
      registry, multi-remote descriptor, or provider migration.
- [x] Full format/lint/native/wasm compile and execution gates pass; the staging
      checklist and its virtual-authenticator limitation are recorded.
- [x] `git status --short` shows only files in Scope plus plan-status updates.
- [x] `plans/README.md` marks this plan DONE.

## STOP conditions

Stop and report without weakening the design if any occurs:

- The live provider cannot distinguish a missing revision cell from
  unauthorized/unavailable/malformed responses.
- The live provider ignores/rejects `If-None-Match: *`, or two different first
  revisions can both succeed.
- Root→device→operator cannot authorize a root-subject repository without
  persisting root key material.
- Losing atomic creation cannot fetch and adopt/reset to the winning revision.
- Canonical descriptor encoding cannot round-trip byte-for-byte on native and
  wasm.
- D1 cannot implement atomic set-if-absent-and-return-winner in one statement or
  transaction.
- The implementation would persist a ceremony candidate before the service
  returns the established winner.
- A mounted/unhydrated branch becomes reachable by an account mutation API.
- Adding `tonk:account` requires exposing it in a user-space list or spot
  registry.
- The real cardinality-one divergence test is order-dependent, oscillates, or a
  post-convergence rename cannot supersede the winner.
- Completing the task requires account-service display-name storage, root key
  storage, a pending rename intent, provider migration, or a generic settings
  framework.
- A required Dialog dependency change is larger than a focused typed-error/CAS
  fix; present it separately and wait for approval.

## Maintenance notes

- `account_subject == genesis root DID` is a V1 initialization fact, not a
  promise that current signing authority can never rotate. Future succession
  must preserve authorization to this immutable subject and routing key.
- Descriptor hash keys local trust. A future descriptor version/migration must
  define data transfer and marker invalidation explicitly; do not silently
  accept a different descriptor.
- Every future account fact requires its own boundedness, writer-set, merge,
  projection, migration, and confidentiality review.
- Reviewers should scrutinize the exact two marker-write sites, all error→absence
  mappings, service winner bytes, kind-filtered enumeration, and no-op
  projection tests before UI details.
- Remote operators can inspect V1 account display names. This is deliberate for
  non-secret metadata, not a precedent for credentials or private account data.
