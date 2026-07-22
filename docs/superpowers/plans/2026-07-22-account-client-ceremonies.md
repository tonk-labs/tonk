# Account Client Ceremonies Implementation Plan

**Goal:** Make the stage-1 account worker usable from a browser and the CLI: create an account, link another browser profile with the synced passkey, and link a native CLI profile through a browser handoff.

**Base:** `origin/staging` after account worker PR #625 (`05fad2ee5`).

## Decisions

- Split stage 2 into two reviewable PRs. PR A ships browser account creation and browser self-link. PR B adds the CLI handoff protocol and command.
- WebAuthn stays in the top document. Portal guests have opaque origins and cannot use the `tonk.spot` RP ID; a message delivered from a guest also cannot be assumed to retain transient user activation. Account ceremony controls therefore render in the top document.
- A ceremony derives the root signer once, mints `root → device`, builds the root-signed service request, then drops the signer. PRF output returned by `credentials.create()` is reused so account creation does not prompt twice on platforms that support it.
- The local profile DID remains the device DID. The resulting delegation is saved both to the dialog UCAN store and as exact serialized bytes in the profile credential store so later account invocations can attach it without deriving the root.
- Account creation and root self-link use root-signed invocation containers. Email verification still authorizes account creation, but every mutable request field is bound by the root signature.
- The public root-signed manifest discussed on 2026-07-22 remains additive. These ceremonies already produce its canonical input (`root → device` delegations); no public device correlation is introduced until manifest privacy and freshness semantics are settled.
- Stage 3 roster migration and re-anchoring are not part of these PRs.

## Protocols

### Account creation

1. The top-document account UI requests `POST /codes` with the email address.
2. The user enters the emailed code and clicks Create account.
3. `navigator.credentials.create()` creates the discoverable passkey and requests PRF.
4. If creation returned PRF output, use it. Otherwise perform one follow-up `get()`.
5. Derive the root signer and mint `root → localProfileDid`.
6. Build a root-signed invocation with command `account/create`; arguments bind email, code, credential id, device DID, device name, and delegation bytes.
7. `POST /accounts` verifies the invocation, consumes the code, and atomically inserts the account and first device.
8. `POST /api/account/link` validates and persists the delegation in the local profile.

### Browser self-link

1. The new browser already has a new local profile DID from the service worker.
2. The user clicks Link this browser and completes a discoverable passkey `get()`.
3. Derive the root signer and mint `root → localProfileDid`.
4. Build a root-signed invocation with command `account/device/link`; arguments bind the new device DID, name, and delegation bytes.
5. `POST /devices/link` resolves the account by the invocation subject/root DID, verifies the root signature and delegation, and inserts the device.
6. Persist the delegation locally through `POST /api/account/link`.

### CLI browser handoff

1. `tonk account link` opens the local profile, generates a 32-byte handoff secret, and creates a five-minute pending request containing the CLI profile DID and device name.
2. The CLI prints and attempts to open `https://tonk.spot/account/link#<secret>`, then polls the account service.
3. The browser resolves the pending request with the secret, asks for the synced passkey, derives the root, and mints `root → cliProfileDid`.
4. A root-signed `account/link/complete` invocation binds the pending request, CLI DID, name, and delegation.
5. The account service atomically registers the device and marks the handoff complete.
6. The CLI consumes the delegation once and persists it to its UCAN and credential stores.

The raw handoff secret is a bearer capability. D1 stores only its BLAKE3 hash; completed delegation bytes expire after first retrieval or five minutes. Pending creation is rate-limited separately from email codes.

## PR A: Browser creation and self-link

### 1. Root-signed bootstrap invocations

Files:

- `rust/tonk-identity/src/ceremony.rs` (new)
- `rust/tonk-identity/src/lib.rs`
- `rust/tonk-identity/src/install.rs`
- `rust/tonk-identity/Cargo.toml`

Work:

- Add serializable `AccountCeremony` output: root DID, device DID, delegation hex, and invocation bytes.
- Add `create_account_invocation(email, code, credential_id, device_did, device_name, root)` and `link_device_invocation(device_did, device_name, root)`.
- Set `issued_at`, a short expiration, and a random nonce on both invocations.
- Change the passkey create path so the compound ceremony can consume `PasskeyCredential.prf_output` before it is dropped.
- Export top-document functions that return byte arrays/hex rather than root key material.
- Test argument binding, issuer/subject equality, commands, expiration, and delegation audience.

### 2. Harden account creation and add root self-link

Files:

- `rust/tonk-account-service/src/auth.rs`
- `rust/tonk-account-service/src/core/accounts.rs`
- `rust/tonk-account-service/src/core/devices.rs`
- `rust/tonk-account-service/src/handlers/accounts.rs`
- `rust/tonk-account-service/src/handlers/devices.rs`
- `rust/tonk-account-service/src/lib.rs`
- `rust/tonk-account-service/src/helpers/server.rs`
- `rust/tonk-account-service/tests/service.rs`
- `rust/tonk-account-service/README.md`

Work:

- Add `authorize_root`: parse and verify an invocation, require issuer = subject = root DID, require the expected command, and enforce issued-at/expiration bounds before store access.
- Replace the unsigned JSON body of `POST /accounts` with the `account/create` container.
- Add `POST /devices/link` for root-authorized self-link; it must resolve an existing account by root and validate the attached delegation before insertion.
- Keep `/devices/register` for day-to-day device-authorized administration.
- Add negative tests for modified arguments, foreign roots, expired invocations, unknown roots, replay/duplicate device conflicts, and verify-before-store ordering.

### 3. Persist the account link in the local profile

Files:

- `rust/tonk-worker/src/router/account.rs` (new)
- `rust/tonk-worker/src/router.rs`
- `rust/tonk-worker-api/src/account.rs` (new)
- `rust/tonk-worker-api/src/lib.rs`

Work:

- Add `POST /api/account/link` accepting root DID and delegation hex.
- Verify the chain cryptographically and require its audience to equal the current profile DID before writes.
- Save the chain through `profile.access().save(UcanDelegation(...))`.
- Save the exact serialized chain bytes under a fixed profile credential-site key for later account invocations.
- Add `GET /api/account` returning unlinked or `{ rootDid, deviceDid }` from the persisted link.
- Test malformed/foreign-audience rejection, round-trip persistence, and idempotency.

### 4. Top-document account surface

Files:

- `rust/tonk-ui/src/account.rs` (new)
- `rust/tonk-ui/src/bin/ui.rs`
- `rust/tonk-ui/src/api.rs`
- `rust/tonk-ui/src/identity.rs`
- `rust/tonk-ui/Cargo.toml`
- `rust/tonk-core/assets/library/profile.yaml`

Work:

- Register a top-document `<tonk-account>` custom element and route `/account` to it without placing WebAuthn inside a sealed guest.
- States: unlinked choice, email/code account creation, passkey self-link, working, success, and actionable failure.
- Fetch the local device DID from `/api/identify`; never accept it from editable DOM state.
- Account service base defaults to `https://accounts.tonk.spot` and is injectable in tests/off-apex staging.
- Persist locally only after the remote mutation succeeds; if local persistence fails, retry it without repeating the root ceremony.
- Add a Hub/FAB account entry that navigates to `/account`.
- Browser test with a PRF-enabled CDP authenticator and native account-service helper: request captured code, create account, verify local persistence, start a fresh profile/browser context, self-link, and list both devices.

## PR B: CLI handoff

### 5. Pending-link storage and endpoints

Files:

- `rust/tonk-account-service/migrations/0002_link_requests.sql`
- store trait plus D1/sqlite implementations
- `core/links.rs`, `handlers/links.rs`
- worker and native-helper routes

Schema:

```sql
CREATE TABLE link_requests (
    token_hash TEXT PRIMARY KEY,
    device_did TEXT NOT NULL,
    device_name TEXT NOT NULL,
    delegation_hex TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER
);
```

Endpoints:

- `POST /links`: create pending request from token hash + CLI device metadata.
- `POST /links/resolve`: bearer-secret lookup for browser display.
- `POST /links/complete`: root-signed completion, atomically register device + attach delegation.
- `POST /links/consume`: bearer-secret one-time retrieval by CLI.

Tests cover expiry, DID substitution, double completion, one-time consumption, and transaction rollback.

### 6. CLI command and browser completion route

Files:

- `rust/tonk-cli/src/account.rs` (new)
- `rust/tonk-cli/src/bin/tonk.rs`
- `rust/tonk-cli/src/lib.rs`
- `rust/tonk-ui/src/account.rs`

Work:

- Add `tonk account status` and `tonk account link`.
- Generate the secret locally, create the pending request, print/open the URL, poll with bounded exponential backoff, and handle Ctrl-C without deleting local identity.
- Persist the returned delegation through the same shared helper used by the worker.
- The top-document `/account/link` route resolves and completes the handoff after an explicit user click.
- Native integration test drives CLI request → browser completion core → CLI consume without a real browser; CDP test covers the actual passkey ceremony.

## Gates

For each PR:

```fish
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test -p tonk-identity
cargo test -p tonk-account-service --features helpers
cargo test -p tonk-worker
cargo test -p tonk-cli
cargo check --target wasm32-unknown-unknown -p tonk-identity -p tonk-account-service -p tonk-worker -p tonk-ui
```

Run the PRF-enabled CDP ceremony scenarios through the existing `tonk-ui` integration harness. Before deploying, smoke-test D1 migration state, `/codes`, account creation, self-link, and CORS against the real account worker.
