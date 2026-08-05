# Portable passkey account summary implementation plan

**Goal:** Show the verified account email plus truthful passkey creation time and creation device on every linked account device, without inferring passkey facts from device-attachment history.
**Approach:** Capture optional passkey metadata only when Tonk successfully creates a passkey, persist it with the provider-neutral local root, and bind it into the later root-signed account-creation invocation. Store the optional pair on the provider account row and expose it through a device-authorized summary endpoint proxied by the local worker. The dashboard renders the verified email on every account and renders either both passkey fields or an explicit legacy/unavailable state.
**Constraints:**
- A passkey may predate account attachment; account or device `created_at` values must never be relabelled as passkey creation time.
- `created_on` describes the browser/OS where Tonk ran `navigator.credentials.create()`, not the current password-manager or storage provider.
- Existing local-root records and account rows remain readable with absent metadata.
- Passkey metadata is informational only and must not alter root derivation, delegation bytes/CIDs, authorization, revocation, or account-repository authority.
- The account summary must require an active device invocation and must not expose whether an arbitrary email or root exists.
- No new dependencies.

## File map

- `rust/tonk-worker-api/src/identity.rs`: optional local passkey metadata on root save/status DTOs.
- `rust/tonk-worker-api/src/account.rs`: authenticated account-summary response DTO.
- `rust/tonk-identity/src/ceremony.rs`: capture creation time and bind optional metadata into account creation.
- `rust/tonk-identity/src/install.rs`: carry metadata across the top-document JavaScript ceremony boundary.
- `rust/tonk-ui/src/identity_bridge.rs`: typed ceremony input/output metadata.
- `rust/tonk-ui/src/account.rs`: persist creation metadata, request the account summary, and render honest fallback copy.
- `rust/tonk-ui/src/api.rs`: local account-summary and metadata-aware root persistence clients.
- `rust/tonk-ui/src/account.html`: account-email and passkey-fact markup.
- `rust/tonk-ui/src/account.css`: aligned summary facts and responsive unavailable state.
- `rust/tonk-account-service/migrations/0007_passkey_metadata.sql`: nullable passkey creation columns for legacy compatibility.
- `rust/tonk-account-service/src/store.rs`: account model and atomic creation interface.
- `rust/tonk-account-service/src/store/sqlite.rs`: native migration/query/binding support.
- `rust/tonk-account-service/src/store/d1.rs`: Cloudflare D1 query/binding support.
- `rust/tonk-account-service/src/core/accounts.rs`: validated optional metadata on account creation.
- `rust/tonk-account-service/src/handlers/accounts.rs`: Worker account-create parsing and authenticated summary handler.
- `rust/tonk-account-service/src/helpers/server.rs`: native helper equivalents for browser and HTTP tests.
- `rust/tonk-account-service/src/lib.rs`: Worker route registration.
- `rust/tonk-worker/src/router/account_devices.rs`: attached-provider lookup and device-signed summary proxy.
- `rust/tonk-worker/src/router.rs`: local `GET /api/account/summary` route.
- `rust/tonk-ui/src/account_flow.rs`: real-browser summary regression coverage.

### Task 1: Preserve passkey metadata with the local root

**Files:**
- Modify: `rust/tonk-worker-api/src/identity.rs:RootStatus, SaveRootRequest`
- Modify: `rust/tonk-worker/src/router/identity.rs:LocalRootRecord, status, persist_root`
- Modify: `rust/tonk-identity/src/ceremony.rs:RootCeremony, create_root`
- Modify: `rust/tonk-identity/src/install.rs:create_root, root_result`
- Modify: `rust/tonk-ui/src/identity_bridge.rs:CreateRootInput, RootOutput`
- Modify: `rust/tonk-ui/src/api.rs:save_root`
- Test: `rust/tonk-worker/src/router/identity.rs:tests`
- Test: `rust/tonk-ui/src/identity_bridge.rs:tests`

**Interfaces:**
- Produces: `PasskeyMetadata { created_at: u64, created_on: String }` and optional `passkey` fields on `RootStatus::Ready` and `SaveRootRequest`.
- Compatibility: missing `passkey` deserializes as `None`; existing record version 1 remains valid.

- [x] Add a local-root regression test that deserializes a save request containing passkey metadata, persists it, reloads it, and expects the same camelCase metadata in `RootStatus`; run it and observe failure because the metadata is currently discarded.
- [x] Add optional metadata to the wire DTO and local record, rejecting blank creation-device labels and zero timestamps at the persistence boundary.
- [x] Capture Unix seconds immediately after successful passkey creation when `createdOn` is provided, return it through the ceremony bridge, and pass the current browser/OS label from the account flow.
- [x] Run the focused local-root and identity-bridge tests; expect success.

### Task 2: Store and return portable account metadata

**Files:**
- Create: `rust/tonk-account-service/migrations/0007_passkey_metadata.sql`
- Modify: `rust/tonk-account-service/src/store.rs:Account, Store::create_account_with_device, account queries`
- Modify: `rust/tonk-account-service/src/store/sqlite.rs:in_memory, account mapping, account creation`
- Modify: `rust/tonk-account-service/src/store/d1.rs:AccountRowD1, account creation`
- Modify: `rust/tonk-account-service/src/core/accounts.rs:CreateAccount, create_account`
- Modify: `rust/tonk-account-service/src/handlers/accounts.rs`
- Modify: `rust/tonk-account-service/src/helpers/server.rs`
- Modify: `rust/tonk-account-service/src/lib.rs`
- Test: `rust/tonk-account-service/src/store/sqlite.rs:tests`
- Test: `rust/tonk-account-service/src/core/accounts.rs:tests`
- Test: `rust/tonk-account-service/tests/service.rs`

**Interfaces:**
- Consumes: optional root-signed `passkeyCreatedAt` integer and `passkeyCreatedOn` string arguments; either both must be present or both absent.
- Produces: device-authorized `POST /account/summary` for command `account/summary`, returning `{ email, passkey: { createdAt, createdOn } | null }`.
- Validation: `createdAt > 0`, no more than five minutes in the future relative to account creation; trimmed `createdOn` is 1–120 characters and contains no control characters.

- [x] Add a migration/store test requiring nullable metadata columns and round-tripping both populated and legacy-null account rows; run it and observe the expected missing-column/value failure.
- [x] Add an HTTP service test that creates an account with signed metadata, fetches the summary with its active device invocation, and asserts the verified email and exact metadata; run it and observe the expected 404/missing-route failure.
- [x] Add migration, atomic store bindings, validation, Worker/native routes, and device authorization. Do not add a root- or email-lookup endpoint.
- [x] Run focused account-service store, core, and HTTP tests; expect success.

### Task 3: Carry metadata through account creation and the local worker

**Files:**
- Modify: `rust/tonk-worker-api/src/account.rs:AccountSummary`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Modify: `rust/tonk-identity/src/ceremony.rs:create_account`
- Modify: `rust/tonk-identity/src/install.rs:create_account`
- Modify: `rust/tonk-ui/src/identity_bridge.rs:CreateAccountInput`
- Modify: `rust/tonk-worker/src/router/account_devices.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-ui/src/api.rs:account_summary`
- Test: adjacent serialization, ceremony, and worker-router tests.

**Interfaces:**
- Consumes: optional local `PasskeyMetadata` from `RootStatus` or a newly created `RootOutput`.
- Produces: root-signed optional metadata on `POST /accounts`; local `GET /api/account/summary` returning `AccountSummary`.

- [x] Extend the ceremony regression test to require exact optional metadata arguments and absence of both arguments for legacy creation; run it and observe failure because the current signature cannot carry metadata.
- [x] Thread the optional value through account input, invocation signing, provider parsing, and storage without changing delegation construction.
- [x] Add the local worker proxy using the profile's attached provider and existing device invocation chain; deserialize the complete provider response.
- [x] Run focused worker-API, identity-ceremony, worker-router, and UI API compile/tests; expect success.

### Task 4: Render the account summary clearly

**Files:**
- Modify: `rust/tonk-ui/src/account.html:account-passkey`
- Modify: `rust/tonk-ui/src/account.css:account passkey facts`
- Modify: `rust/tonk-ui/src/account.rs:show_success, summary rendering`
- Modify: `rust/tonk-ui/src/account_flow.rs:it_signs_up_through_the_account_panels`
- Test: `rust/tonk-ui/src/account.rs:tests`

**Interfaces:**
- Consumes: `AccountSummary` from the local worker.
- Produces: `Account email`, `Created`, and `Created on` facts. Legacy or failed summary loads show `Unavailable` and never substitute account/device dates.

- [x] Add a dashboard DOM test requiring the three labelled facts and explicit unavailable fallback; run it and observe failure because the facts are not authored.
- [x] Add the semantic description list, compact three-column alignment, pretty wrapping, and responsive layout without changing existing interaction behavior.
- [x] Load summary alongside devices; render localized creation date and the recorded browser/OS label. Treat a missing/older provider endpoint as unavailable metadata rather than hiding device management.
- [x] Extend the real-browser signup test to require the verified email, a non-empty localized creation date, and `Chrome on …` creation label.
- [x] Run focused Wasm account tests and compile the real-browser flow; expect success.

### Task 5: Verify the complete slice

- [x] Run `cargo fmt --all -- --check` and `git diff --check`.
- [x] Run focused native account-service tests with `tonk-account-service/helpers`.
- [x] Run focused Wasm tests for `tonk-worker-api`, `tonk-identity`, `tonk-worker`, and `tonk-ui` in the repository Nix environment.
- [x] Run `nix develop -c cargo test -p tonk-ui --no-run` to compile the updated browser flow.
- [x] Run `nix develop -c build:web`.
- [x] Inspect the built account dashboard with disposable summary/device data at desktop and mobile widths; verify label/value alignment, wrapping, unavailable copy, and no new console/layout failures.
- [x] Re-read the diff to confirm no account timestamp or device registration timestamp is presented as passkey creation time.

## Verification evidence

- Red/green: local-root metadata test failed with `left: Null`, then passed after persistence was implemented.
- Red/green: migration test failed on missing `passkey_created_at`; signed HTTP test failed with `404` on `/account/summary`; both passed after implementation.
- `nix develop -c cargo test -p tonk-account-service --features helpers`: 64 unit tests and 9 HTTP integration tests passed.
- Affected Wasm suites: 34 identity, 25 UI, 260 worker, and 21 worker-API tests passed.
- Affected native and Wasm compile checks passed; the updated real-browser signup flow compiles.
- `nix develop -c build:web`: passed after moving the new worker handler into a tracked module so the flake source snapshot included it.
- Mounted QA used disposable summary/device data at desktop and the Chrome runner's narrow viewport: three-column facts collapsed to one column and no horizontal overflow was observed. The already-running dev server retained a prior hot-reload build-error badge; the production build passed.

## Explicitly deferred

- Exact password-manager/provider naming; WebAuthn does not expose it reliably.
- Backup eligibility/state and authenticator attachment; useful later, but require registration/assertion parsing and careful user-facing semantics.
- Editing passkey metadata or backfilling legacy rows by inference.
