# Account-setup provider recovery implementation plan

**Goal:** Make account creation safe to retry after a lost response and expose a proof-bound setup-status check without revealing whether an arbitrary root or email has an account.
**Approach:** Validate every creation ceremony before the existing atomic account/device insert, then treat an insert conflict as a successful replay only when the persisted account and its earliest active device reproduce every caller-controlled semantic input. The shared `tonk-account` contract derives a domain-separated, length-framed BLAKE3 fingerprint from those inputs, allowing the caller to retain it before sending the request. A device-signed setup-status invocation can then distinguish absent, accepted, and mismatched provider state without adding mutable setup rows or a schema migration.
**Constraints:**
- Base this work only on live `origin/staging` commit `605ab21f548e2404db18d4800afbf67631ea9b94`; do not copy or depend on the dirty browser-account audit worktree.
- Keep the initial `POST /accounts` response at HTTP 201. An exact replay returns HTTP 200 with `reused: true`; both outcomes carry the same stable account ID, canonical descriptor, and creation fingerprint.
- Continue attempting the atomic account/device insert first. Recovery is allowed only after its conflict and only after the descriptor and delegation have passed full cryptographic validation.
- The normalized email is the service's existing lowercase form, and the optional passkey `createdOn` label is trimmed. Credential ID, optional passkey presence/creation time, canonical descriptor bytes, first-device DID/name, and delegation CID plus decoded bytes must otherwise match exactly.
- The shared `tonk-account::creation` input and algorithm are the single source of truth for provider and client fingerprints. Its wire form is exactly 64 lowercase hexadecimal characters; its version-1 test vector is pinned.
- The random attachment ID and server-selected account/device creation time are excluded from equality and the fingerprint.
- Every non-exact conflict retains the current conflict classification and must not expose SQLite/D1 driver text.
- Setup status is signed by the device through one command-open, subject-open, unexpired `root -> device` proof. It may query only the invocation subject root, never a caller-supplied email or root.
- Do not add a database migration or new dependency. If the live schema cannot support the contract, stop before changing it.
- This provider-only PR changes no rendered UI or copy, so it does not add Storybook journeys; API documentation and executable HTTP coverage record the contract.

## File map

- `plan/audit-account-setup-provider.md`: durable scope, invariants, TDD sequence, and verification evidence.
- `rust/tonk-account/src/creation.rs`: caller-visible canonical fingerprint input, versioned algorithm, strict wire parser, and fixed test vector.
- `rust/tonk-account/src/lib.rs`: exports the shared creation-recovery contract.
- `rust/tonk-account-service/src/core/accounts.rs`: canonical stored creation facts, shared fingerprint adapter, exact-conflict recovery, and typed setup-status policy.
- `rust/tonk-account-service/src/auth.rs`: narrow root-to-device setup proof verifier that does not consult account storage.
- `rust/tonk-account-service/src/handlers/accounts.rs`: Cloudflare create replay response and setup-status handler.
- `rust/tonk-account-service/src/helpers/server.rs`: native route parity for status codes, JSON, CORS, and integration tests.
- `rust/tonk-account-service/src/lib.rs`: Worker route and preflight registration.
- `rust/tonk-account-service/tests/service.rs`: response-loss replay, status outcomes, proof rejection, and wire-error regressions.
- `rust/tonk-account-service/README.md`: public request/response, authentication, fingerprint, and deployment-order contract.

### Task 1: Recover only an exact account-creation replay

**Files:**
- Create: `rust/tonk-account/src/creation.rs`
- Modify: `rust/tonk-account/src/lib.rs`
- Modify: `rust/tonk-account-service/src/core/accounts.rs:CreateAccount, create_account, tests`
- Test: `rust/tonk-account-service/src/core/accounts.rs:tests`

**Interfaces:**
- Produces: `CreateAccountOutcome { account_id: i64, descriptor: Vec<u8>, create_fingerprint: String, reused: bool }`.
- Produces: shared `AccountCreationFingerprintInput` and strict `AccountCreationFingerprint` types so a caller can calculate and retain the fingerprint before `POST /accounts`.
- Produces: lowercase 64-hex `createFingerprint`, BLAKE3 domain `tonk-account-create-v1`, over explicit length-framed semantic fields in the order email, root DID, credential ID, passkey option/value, descriptor bytes, device DID, device name, delegation CID, delegation bytes. The pinned version-1 vector is `35cda4b0895490a01c4307584da2fe045c568b53d9817b3f38c97309a07dbb52`.
- Consumes: the current `Store::create_account_with_device`, `Store::account_by_root`, and `Store::devices` methods; no store or schema extension is required.

- [x] Add `it_reuses_only_an_exact_account_creation` with normalized-equivalent email, passkey label, descriptor hex, delegation hex, and a second random attachment candidate; require the same account ID/descriptor/fingerprint, `reused: true`, and exactly one unchanged device row. Its first focused run failed through the existing `CeremonyError::Conflict` path before implementation.
- [x] Add the shared caller-visible fingerprint contract, fixed vector, and strict parse/format test, then adapt immutable service creation facts to that contract.
- [x] Implement post-conflict comparison against the account plus earliest device row. Require the earliest row to remain active; compare decoded delegation bytes and exclude attachment ID and provider timestamps.
- [x] Add a semantic mismatch regression covering normalized email, root DID through a separately valid ceremony, credential ID, absent/present passkey metadata, each passkey field, a different valid descriptor for the same root, a different delegated device DID, device name, and a newly minted delegation. Every case retains `EMAIL_TAKEN` or `ROOT_TAKEN` and never returns an account outcome.
- [x] Convert the existing database-text regression into a non-exact replay so it still proves raw SQLite/D1 details cannot cross the boundary.
- [x] Run the exact replay and mismatch filters plus adjacent `core::accounts::tests`; all focused filters pass.

### Task 2: Authenticate and answer setup status without arbitrary lookup

**Files:**
- Modify: `rust/tonk-account-service/src/auth.rs:verified_chain, setup-proof helper, tests`
- Modify: `rust/tonk-account-service/src/core/accounts.rs:AccountSetupStatus, account_setup_status, tests`

**Interfaces:**
- Consumes: command `['account', 'setup', 'status']` with required `createFingerprint` and exactly one attached root-to-device delegation.
- Produces: `SetupCaller { root_did, device_did, delegation_cid, arguments }` only after invocation/delegation signatures, five-minute expiry, issuer/audience alignment, root issuer, subject-open shape, and command-open shape verify.
- Produces: `{ "status": "absent" }`, `{ "status": "accepted", "accountId", "descriptorHex", "createFingerprint" }`, or `{ "status": "mismatch" }`.

- [x] Add auth tests proving a valid root-to-device invocation succeeds without a store/account row, while missing proof, a proof from the wrong root, a proof delegated to a different device, subject-specific proof, and command-scoped proof fail. The focused positive test failed to compile before the helper existed.
- [x] Implement the narrow verifier by extending the already verified invocation chain; it accepts no root/email argument and calls no `Store` method.
- [x] Add setup-status core tests: valid unknown root returns `Absent`; exact persisted state and matching first-device proof returns `Accepted`; a wrong 32-byte fingerprint returns `Mismatch`; mismatched device DID/CID and inactive first device are rejected; malformed legacy descriptor/delegation state cannot be reported as accepted.
- [x] Implement status lookup by verified root, require the proof to name the stored earliest active device and delegation CID, reconstruct the same fingerprint, and return only the typed outcome.
- [x] Run focused auth/status tests and adjacent auth/core suites; all focused filters pass.

### Task 3: Keep Cloudflare and native HTTP contracts identical

**Files:**
- Modify: `rust/tonk-account-service/src/handlers/accounts.rs:handle_inner, setup status handler`
- Modify: `rust/tonk-account-service/src/helpers/server.rs:dispatch, accounts_route, setup-status route`
- Modify: `rust/tonk-account-service/src/lib.rs:Worker routes`
- Test: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- `POST /accounts`: initial create returns 201; exact replay returns 200. Both return `{ accountId, descriptorHex, createFingerprint, reused }`.
- `POST /accounts/setup-status`: accepts the CBOR invocation above and returns one typed JSON status with HTTP 200. Authentication/argument failures retain structured 4xx responses; storage failures retain generic 500 responses.
- `OPTIONS /accounts/setup-status`: returns the service's existing CORS headers.

- [x] Add an HTTP response-loss test that discards the first 201 body, replays the exact bytes, and requires 200 plus `reused: true`, the original ID/descriptor/fingerprint, and no duplicate device.
- [x] Add HTTP tests for accepted, mismatch, and valid-proof absent status; unknown/missing proof, wrong device/delegation, wrong command, and malformed fingerprint produce structured 4xx errors without raw DB text or account details.
- [x] Run the status HTTP test before routing; it failed with the expected 404. Exact replay had already failed at the core conflict boundary in the first RED run.
- [x] Implement the Worker handler/routes, native helper route, response status selection, JSON serialization, and preflight registration from the same core/auth seams.
- [x] Run the focused response-loss, privacy, and CORS HTTP filters; each passes. Native HTTP filters required loopback permission after the sandbox rejected binding before behavior ran.

### Task 4: Document and verify the provider slice

**Files:**
- Modify: `rust/tonk-account-service/README.md:API and deployment order`
- Modify: `plan/audit-account-setup-provider.md:verification evidence`

**Interfaces:**
- Documents: retry semantics, exact fingerprint inputs/exclusions, setup-status proof requirements, response shapes, mismatch privacy, and deployment order before a browser client begins recovery calls.

- [x] Update the README with exact request commands, response status codes/fields, proof requirements, and the no-account-existence-leak boundary.
- [x] Run `cargo fmt --all -- --check` and `git diff --check`; both pass after rustfmt's mechanical layout changes.
- [x] Run `cargo test -p tonk-account` (35 unit plus 1 integration test) and `cargo test -p tonk-account-service --features helpers` (49 unit plus 9 native HTTP integration tests); all pass. The service package ran with loopback permission because the restricted sandbox had already denied native listener binding before behavior.
- [x] Run all-target warning-denied clippy for both `tonk-account` and `tonk-account-service --features helpers`; both pass.
- [x] Run the repository-defined account-service-only Cloudflare build, `nix build path:.#tonk-account-service --no-link`; the final source passes at `/nix/store/f89778xgprap6jl7qgmbqkm2rmh0r8hv-tonk-account-service-0.6.9` without building `tonk-ui`.
- [x] Re-read the final diff against every mismatch and privacy requirement, update this plan with fresh green evidence, and confirm no migration, lock-file change, UI source, or unrelated work entered the branch. That review found and fixed one malformed stored-delegation acceptance gap before commit.

## Focused TDD evidence

- Exact replay RED: `it_reuses_only_an_exact_account_creation` ran 0 passed / 1 failed / 42 filtered; SQLite's root uniqueness conflict was safely classified as `an account already exists for this passkey`.
- Exact replay GREEN, including every normalization-equivalent form: 1 passed / 0 failed / 47 filtered.
- Shared fingerprint contract RED: the fixed-vector test did not compile before the shared types existed. GREEN fixed-vector and strict parse/format filters each passed 1 / 1 with 34 filtered.
- Auth helper RED: the positive setup-proof test did not compile before `authorize_setup_device` existed. GREEN proof-shape matrix passed 1 / 1 with 46 filtered.
- Status core RED: the absent-status test did not compile before the status contract existed, and the accepted-status test initially returned `Mismatch`. A final mutation proved that merely decoding malformed stored delegation bytes could incorrectly return `Accepted`; that focused run failed 0 / 1 with 48 filtered, then passed unchanged after the stored bytes were revalidated to the authenticated root, device, and CID. The full negative matrix is green.
- HTTP RED: the response-loss/status test reached the native route and received 404 before registration. GREEN response-loss, privacy/error, and CORS filters each passed 1 / 1. The final response-loss form computes the shared fingerprint before `POST /accounts`, discards the `201` body, and obtains `Accepted` before receiving any replay response.
- The first native HTTP attempt in the restricted sandbox failed to bind loopback with `PermissionDenied` before behavior; every unchanged HTTP filter passed with loopback permission.
