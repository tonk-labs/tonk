# Identity and Sharing E2E Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make root creation, account attachment, device and invitation revocation, open/targeted joining, guest promotion, and sync complete honestly through the visible browser UI, with no DevTools request repair, no false 2xx success, and no visible state left by a failed remote-backed join.

**Architecture:** Harden each weak boundary once instead of patching individual call sites. `tonk-ui` gets one typed JavaScript identity bridge and one serialized identity-gate state machine. Browser DTOs move into `tonk-worker-api`, deployment-specific service URLs come from an explicit same-origin configuration endpoint, and worker-to-service traffic goes through media-type-specific HTTP helpers that retain structured upstream failures. Join becomes a prepare/preflight/commit operation: a volatile proof store and staged repository prove audience, remote authorization, and usable initial content before the durable profile is made visible; profile indexing, roster writes, guest clearing, backup, and navigation happen only after that gate. Sync uses the same typed upstream error chain to map revocation, conflict, unavailability, and unknown failures to honest statuses and visible states.

**Tech Stack:** Rust 2024, wasm-bindgen/web-sys, Axum, dialog UCAN/repository/operator/storage, workers-rs D1/R2, Cloudflare Workers/Wrangler, thirtyfour + CDP virtual authenticators, Nix/Crane, serde JSON/CBOR.

**Design of record:** `docs/superpowers/specs/2026-07-29-identity-sharing-e2e-hardening-design.md`.

**Written against:** commit `d5269faad` on `feat/in-band-revocation`. The design and earlier revocation documents/configuration have uncommitted work in this worktree; preserve it and do not overwrite or revert it.

## Delivery order

1. Prove the temporary join and typed-upstream-error seams before committing to the refactors.
2. Land the shared identity bridge and null-body response conversion first; most browser flows depend on them.
3. Land canonical deployment/JSON and typed HTTP contracts before changing revocation or invitation UI.
4. Surface publication acknowledgements without redesigning the account-service core, which already writes R2 before projecting D1.
5. Replace the join command shape and error vocabulary, then make the shared join operation externally atomic.
6. Land typed sync outcomes after the dialog transport prerequisite is available.
7. Fix the template binding and Darwin derivation independently.
8. Run focused and repository gates before the non-destructive staging smoke. Deploy account relay changes before access/UI changes.

## Locked decisions

- **The authority model does not change.** Durable authority remains `space → root → device → session`; R2 artifacts remain canonical and D1 remains a projection.
- **One UI identity bridge.** `identity_gate.rs` and `account.rs` do not reflect into `window.tonkIdentity` or serialize ceremony inputs directly.
- **No environment inference.** Browser deployment URLs come from `GET /.well-known/tonk`; stored remote/invitation metadata selects revocation relays. No substring check for `staging` and no production fallback on an unknown host.
- **Provider metadata is the worker's account-service source.** Once attached, background backup/restore/device routes use `account::provider(state)`, not the service-worker hostname.
- **The account revocation core stays publication-first.** Reuse `RevokeOutcome`/`Projection`; only widen and forward the acknowledgement contract.
- **Join preflight uses volatile storage.** Build a session operator over `Storage<VolatileSpace>`, retain only the candidate claim plus the existing root/device/session path there, mount a staged verifier replica, pull it, and prove required content. Do not optimistically save candidate authority into the durable certificate store.
- **Staged content is promoted exactly, not replayed.** Copy the staged branch revision and every reachable archive/blob block into an unindexed durable repository without creating a new synthetic commit. If the pinned dialog APIs cannot do that, land the smallest upstream staged-install API; do not substitute CSV/artifact export that drops blobs or changes history.
- **Profile indexing is the join visibility commit.** A durable repository may be prepared under its DID, but it is not a profile replica until remote authorization/content have passed and its profile `Replica` fact is committed. Backup and navigation are post-commit effects.
- **No string matching for sync policy.** The current dialog transport erases HTTP status/code into strings; an accepted dialog-db prerequisite must preserve typed service-response data before honest sync lands.
- **Invitation management is minimal, not a FAB redesign.** Add targeted mint, active invitation rows, and revoke actions to the existing share menu; do not redesign the surrounding FAB.
- **Local Cloudflare replication is deferred.** Do not add a composite Worker, local D1/R2/S3/mail stack, or multi-profile browser matrix in this change. Staging is the final cross-service integration target.

## Global constraints

- Tests use `#[dialog_common::test]` and names `it_does_x`.
- Wasm test modules use `run_in_service_worker` for `tonk-worker` and `run_in_browser` for UI/FAB/workspace/schema crates.
- No `mod.rs`; use `foo.rs` plus `foo/` submodules.
- Do not reference task numbers or the design document in production comments.
- Do not change revocation artifact bytes or the root-first authority chain.
- Do not pass PRF output/root seeds across the window boundary.
- Never log or persist full invite URLs, URL fragments, invitation seeds, delegation bytes, ceremony inputs, or upstream bodies that may contain credentials.
- Bounded upstream response text may be returned to an immediate caller only after parsing the structured error envelope; logs contain method, path, status, and stable code only.
- Browser-facing JSON is camelCase. Snake_case aliases named in this plan are temporary input compatibility only.
- Existing frozen command descriptors are load-bearing. New optional remote/relay values ride as opportunistically-read raw facts; do not add them as required fields to old `CreateSpace`/`Invite`/`EnableSync` matched concepts.
- A body on 204, 205, 304, or a `HEAD` response is always discarded at the browser conversion boundary.
- If the dialog prerequisite changes the pin, update every workspace `dialog-*` dependency to one accepted revision and update `Cargo.lock` once; never mix dialog revisions.
- Full lint gate:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

- Full repository gates:

```bash
nix develop -c test:native:debug
nix develop -c test:web:debug
```

Release variants remain final pre-PR gates.

---

### Task 1: Prove the blocking seams

**Files:** no committed changes. Temporary ignored tests/probes must be removed.

- [ ] **Step 1: Record the baseline and focused failures**

Run:

```bash
git status --short
git rev-parse --short HEAD
cargo test -p tonk-worker-api -p tonk-account-service --features helpers
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
```

Expected: existing suites compile/pass; record unrelated failures without fixing them in this task.

- [ ] **Step 2: Prove an Axum route can receive the browser origin explicitly**

Add a throwaway wasm test around `RequestConversion` that converts `https://local.example/join`, records the parsed origin in an Axum request extension, and still routes by `/join`. Also prove a guest path rewrite changes only the path/query and retains the original origin extension.

Expected: no need to trust `Host` or reconstruct from `OriginalUri`; Task 4 can introduce a small `RequestOrigin` extension in `axum.rs`.

- [ ] **Step 3: Prove volatile join preflight with the real authority path**

In a temporary `tonk-worker` test:

1. open a real profile and root grant;
2. claim an invite to that root without saving it durably;
3. create `Storage<VolatileSpace>` and a bounded session operator derived from the same profile;
4. retain the root grant, candidate invite chain, and staged session delegation only in volatile storage;
5. mount a verifier-only repository, attach the invite's UCAN remote, pull `main`, and query the required repository name/space view;
6. add the eventual roster/provenance facts in the staged branch;
7. promote the exact staged revision plus every reachable archive/blob block into an unindexed repository in durable storage without saving the candidate chain;
8. assert the promoted tree/revision and blob reads match the stage, a later remote fetch sees the same upstream head, and the durable profile replica list/certificate store are unchanged.

Expected: the staged pull presents `space → … → root → device → staged-session`, and promotion is a local storage install rather than a second remote pull. If the current generic APIs cannot build the staged operator, pull, or promote the complete revision losslessly, stop and land the smallest dialog operator/storage prerequisite. Do not fall back to save-then-delete (the certificate store has no delete), CSV/artifact replay (it can change history/drop blobs), or a second network pull after durable mutation.

- [ ] **Step 4: Verify the typed upstream-error gap and define the upstream prerequisite**

Inspect the pinned dialog crates:

```bash
rg -n "pub enum S3Error|Service\(|impl From<S3Error> for MemoryError" \
  ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-remote-s3/src/error.rs
rg -n "Access service returned|response.status|response.text" \
  ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-remote-ucan-s3/src/site.rs
rg -n "pub enum (PullError|PushError|ResolveError|PublishError)" \
  ~/.cargo/git/checkouts/dialog-db-*/*/rust/dialog-repository/src/repository/error.rs
```

Expected on the current pin: status and JSON code become `S3Error::Service(String)` and later `MemoryError::Storage(String)`.

Before Task 10, land/reuse an upstream dialog-db change that preserves a structured service error at least through `UcanAuthorization::redeem → S3Error → MemoryError → ResolveError/PublishError → PullError/PushError`, carrying:

```rust
pub struct ServiceResponseError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}
```

The response body is bounded before parsing. Add upstream tests for 403 `CREDENTIAL_REVOKED`, legacy 403 `DEVICE_REVOKED`, 409, 503, malformed JSON, and 5xx. **STOP:** do not implement sync classification by matching `Display` strings.

- [ ] **Step 5: Confirm the Darwin dependency path**

Run:

```bash
ROOT=$(nix eval --raw .#tonk-cloudflare-artifacts.drvPath)
TARGET=$(nix-store -qR "$ROOT" | grep 'python3.14-remarshal-.*\.drv' | head -1)
nix why-depends "$ROOT" "$TARGET"
```

Expected: the path reaches `python3.14-remarshal` through Crane's Cargo Git dependency vendor derivation, so overriding top-level `pkgs.remarshal` is sufficient; Wrangler's separate flake package is not the source.

- [ ] **Step 6: Remove all probes** — nothing to commit.

---

### Task 2: Add one typed identity bridge and an accessible serialized gate

**Files:**
- Create: `rust/tonk-ui/src/identity_bridge.rs`
- Create: `rust/tonk-ui/src/identity_gate.css`
- Modify: `rust/tonk-ui/src/lib.rs`
- Modify: `rust/tonk-ui/src/identity_gate.rs`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/identity.rs`
- Modify: `rust/tonk-ui/Cargo.toml` only if web-sys features are needed

**Interfaces:**

```rust
pub(crate) struct CreateRootInput { pub device_did: String }
pub(crate) type EvaluateRootInput = CreateRootInput;
pub(crate) struct CreateAccountInput { /* existing typed fields */ }
pub(crate) struct LinkDeviceInput { /* existing typed fields */ }
pub(crate) struct CompleteLinkInput { /* existing typed fields */ }
pub(crate) struct SignRevocationInput {
    pub delegation_cid: String,
    pub path_hex: String,
}

pub(crate) async fn create_root(input: CreateRootInput) -> Result<RootOutput, IdentityBridgeError>;
pub(crate) async fn evaluate_root(input: EvaluateRootInput) -> Result<RootOutput, IdentityBridgeError>;
pub(crate) async fn create_account(input: CreateAccountInput) -> Result<CeremonyOutput, IdentityBridgeError>;
pub(crate) async fn link_device(input: LinkDeviceInput) -> Result<CeremonyOutput, IdentityBridgeError>;
pub(crate) async fn complete_link(input: CompleteLinkInput) -> Result<CeremonyOutput, IdentityBridgeError>;
pub(crate) async fn sign_revocation(input: SignRevocationInput) -> Result<RevocationOutput, IdentityBridgeError>;
```

All inputs use `#[serde(rename_all = "camelCase")]`; the private generic caller serializes with `serde_wasm_bindgen::Serializer::json_compatible()`.

- [ ] **Step 1: Add failing bridge shape tests**

Install a fake `window.tonkIdentity` and test every operation. In JavaScript, assert:

- `input instanceof Map === false`;
- `Object.getPrototypeOf(input)` is an ordinary object prototype or null;
- `deviceDid`, `delegationCid`, `pathHex`, `tokenHash`, and account fields are readable by property access;
- a non-function method, non-Promise return, rejected Promise, and malformed output become stable bridge error variants.

Run:

```bash
cargo test -p tonk-ui --target wasm32-unknown-unknown identity_bridge
```

Expected: fail because the shared module does not exist and the current two bridges create incompatible values.

- [ ] **Step 2: Implement the shared bridge and delete both local reflection helpers**

Move input/output DTOs from `account.rs`/`identity_gate.rs` where practical. Keep locating `window.tonkIdentity`, function validation, Promise awaiting, output decoding, and user-readable error classification private to the bridge. No call site passes `serde_json::Value`.

- [ ] **Step 3: Add the gate stylesheet and accessibility behavior**

Inject `identity_gate.css` once using the account element's stable style-ID pattern. The overlay is fixed to the top document, covers the viewport, and uses z-index `2147483647` (above the FAB's `2147483646`). Include explicit light/dark surface tokens, a layered shadow, balanced heading/pretty body wrapping, at least 40×40px controls, exact-property transitions, and a reduced-motion rule.

On open:

- focus the primary button;
- set `aria-modal`, title/status relationships, and temporarily make non-gate body siblings inert;
- block background pointer input;
- show progress in the live status;
- retain retry and cancel after failure;
- restore focus/inert state on close.

- [ ] **Step 4: Replace the `ACTIVE` boolean with an intent queue**

Use one thread-local gate state with an active request and FIFO pending intents. A second message cannot overwrite the active intent or start another ceremony. Each active intent has a replay guard; success persists the root and replays once, cancel closes without replay, and the next queued request opens afterward.

The CLI `/identity/link` route uses the bridge and stylesheet but preserves challenge/device/copy-response content.

- [ ] **Step 5: Make durable replay complete its UI outcome**

For `DurableJoin`, parse `JoinResponse` and navigate to `/space/{repository.name}` only on success. For create, keep the current typed response. Never include the invite URL in status/error text.

- [ ] **Step 6: Add gate behavior and real-ceremony tests**

Cover focus, cancel, retry, inert restoration, FIFO concurrency, and exactly-once replay. Update `identity.rs` to invoke the real root/account/revocation methods with complete plain-object inputs through CDP.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-ui --target wasm32-unknown-unknown identity_bridge
cargo test -p tonk-ui --target wasm32-unknown-unknown identity_gate
cargo test -p tonk-identity --target wasm32-unknown-unknown install
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-ui
git commit -m "fix(tonk-ui): unify identity ceremonies behind a typed gate"
```

---

### Task 3: Make browser response conversion null-body safe

**Files:**
- Modify: `rust/tonk-worker/src/axum.rs`
- Modify: `rust/tonk-worker/src/worker.rs`

**Interfaces:** `ResponseConversion` receives both `Method` and `AxumResponse<Body>`. It uses `None` for 204, 205, 304, and every `HEAD` response; all other statuses retain streaming conversion.

- [ ] **Step 1: Add failing service-worker tests**

Cover 204, 205, 304, `HEAD` with an accidental body, empty 200, streamed JSON 200, headers on both paths, and a deliberately invalid response-construction input.

Expected: null-body statuses currently throw `Could not construct fetch response` because a stream is always attached.

- [ ] **Step 2: Carry the request method through dispatch**

Capture the Axum request method before `router.call(request)`. Construct `ResponseConversion::new(method, response)` after CORS/client headers are applied.

- [ ] **Step 3: Remove throwing conversion paths**

Replace `unwrap_throw`/`expect_throw` in response header/body construction with `JsError` propagation. For null-body statuses call the browser constructor with no body. Preserve status and headers in both cases.

- [ ] **Step 4: Return a controlled conversion failure**

If conversion fails, log only method, URI path, and original response status, then return a browser 500 with a fixed text/JSON body. Do not let the service-worker fetch reject and do not log request query/body.

- [ ] **Step 5: Run tests**

```bash
cargo test -p tonk-worker --target wasm32-unknown-unknown axum
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
```

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker/src/{axum.rs,worker.rs}
git commit -m "fix(tonk-worker): omit browser bodies for null-body responses"
```

---

### Task 4: Add explicit deployment configuration and canonical invite JSON

**Files:**
- Create: `rust/tonk-worker-api/src/deployment.rs`
- Create: `rust/tonk-worker-api/src/invite.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Modify: `rust/tonk-worker-api/Cargo.toml` (add workspace `url`)
- Create: `rust/tonk-access-service/src/handlers/config.rs`
- Modify: `rust/tonk-access-service/src/handlers.rs`
- Modify: `rust/tonk-access-service/src/lib.rs`
- Modify: `rust/tonk-access-service/src/helpers/server.rs`
- Create: `rust/tonk-ui/src/deployment.rs`
- Modify: `rust/tonk-ui/src/lib.rs`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-worker/src/axum.rs`
- Modify: `rust/tonk-worker/src/router/create_invite.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `wrangler.toml`
- Modify: invite/account/UI READMEs and snippets that show the old fields

**Interfaces:**

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfig {
    pub account_service_url: Url,
    pub revocation_relay_url: Url,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInviteRequest {
    #[serde(default, alias = "base_url")]
    pub base_url: Option<Url>,
    #[serde(default, alias = "recipient_root")]
    pub recipient_root: Option<Did>,
}
```

`CreateInviteResponse` is tagged by `kind`, serializes `recipientRoot`, and temporarily accepts `recipient_root` when deserializing.

- [ ] **Step 1: Add failing DTO tests**

Test canonical camelCase, the two documented snake_case aliases, malformed URLs/DIDs, unknown-field rejection, response camelCase, and that a misspelled `baseURL`/`recipientRot` cannot fall through to defaults.

- [ ] **Step 2: Add `GET /.well-known/tonk`**

The access worker reads `ACCOUNT_SERVICE_URL` and `REVOCATION_RELAY_URL` variables, validates them as absolute URLs, and returns `DeploymentConfig`. Missing/invalid configuration is 500 with internal detail only in logs. Add production/staging values to `wrangler.toml` and route this path through the worker before static assets.

The native helper accepts explicit settings so local tests never inherit production URLs.

- [ ] **Step 3: Cache deployment configuration in the top document**

`tonk-ui::deployment` fetches the same-origin endpoint once and returns typed values. An explicit `<tonk-account service=...>` remains a test/operator override; otherwise account creation waits for configuration instead of matching hostnames or falling back to production. Attached worker operations later use the persisted provider URL.

- [ ] **Step 4: Preserve request origin in conversion**

Add a `RequestOrigin` Axum extension in `RequestConversion`, derived from the browser request URL before any guest path rewrite. Tests prove only scheme+authority are retained and no query/fragment enters it.

- [ ] **Step 5: Move invite DTOs into `tonk-worker-api` and derive omitted base URL from origin**

The route takes `Extension<RequestOrigin>`. When `baseUrl` is absent, build `{origin}/join`; only generic non-browser `tonk-invite` callers retain `DEFAULT_BASE_URL`. Keep inline JSON parsing so errors retain the structured worker envelope.

Use `request.base_url.is_some()` only to decide whether shortening was explicitly requested if that remains desired; origin-derived browser links may also be shortened once tests pin the intended behavior.

- [ ] **Step 6: Update callers/examples and route tests**

Every browser/UI example sends `baseUrl`/`recipientRoot` or omits `baseUrl`; no canonical example uses snake_case. Add a route test whose request origin is local/staging-like and assert the returned URL uses that exact origin, plus a typo test that returns 400 rather than `tonk.spot`.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-worker-api invite deployment
cargo test -p tonk-access-service --features helpers config
cargo test -p tonk-worker --target wasm32-unknown-unknown create_invite
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
python3 -c "import tomllib; tomllib.load(open('wrangler.toml','rb')); print('ok')"
```

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-worker-api rust/tonk-access-service rust/tonk-worker rust/tonk-ui wrangler.toml
git commit -m "fix(invite): make browser origins and JSON contracts explicit"
```

---

### Task 5: Replace ambiguous byte POSTs with typed HTTP operations

**Files:**
- Create: `rust/tonk-worker/src/router/http.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/error.rs`
- Modify: `rust/tonk-worker/src/router/account_backup.rs`
- Modify: `rust/tonk-worker/src/router/account_devices.rs`
- Modify: `rust/tonk-worker/src/router/revoke_invite.rs`
- Modify: relevant native/wasm tests

**Interfaces:**

```rust
pub(crate) async fn post_cbor(endpoint: &Url, body: &[u8]) -> Result<HttpResponse, HttpError>;
pub(crate) async fn post_json(endpoint: &Url, body: &[u8]) -> Result<HttpResponse, HttpError>;

pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub(crate) struct UpstreamFailure {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}
```

A private common function requires an explicit media type; there is no public untyped byte helper.

- [ ] **Step 1: Add failing native and wasm parity tests**

A scripted HTTP server/fake fetch records method, body, content type, and timeout behavior. Test CBOR/JSON headers, success bytes, structured non-2xx parsing, malformed error JSON, body truncation, transport timeout, and status preservation.

- [ ] **Step 2: Implement identical transport policy**

Use a ten-second timeout on both targets. Native uses reqwest timeout. Wasm uses `AbortController` and a service-worker timer, clearing the timer after completion. Add only the required web-sys features.

Bound error bodies (for example 8 KiB) before JSON/text handling. Parse `{ "error": { "code", "message" } }` when present. Do not log body text.

- [ ] **Step 3: Preserve upstream status/code at the worker boundary**

Add a `TonkWorkerError::Upstream` shape whose `IntoResponse` keeps the upstream status where safe and emits the stable code. Best-effort callers may log method/path/status/code and swallow the error; proxy routes return it.

- [ ] **Step 4: Move every invocation/artifact caller**

Use `post_cbor` for:

- `/chains/put`, `/chains/list`, `/chains/get` invocation containers;
- `/devices/list` and `/devices/revoke` invocation containers;
- raw invitation revocation artifacts.

Keep `post_json` for actual JSON-only calls. Delete `post_chains_put` and `post_for_bytes`.

Change `account_service_url` to read `account::provider(state)` at call sites. Keep a native explicit test override only where a state-less helper requires it; there is no native production default.

- [ ] **Step 5: Run tests**

```bash
cargo test -p tonk-worker router::http
cargo test -p tonk-worker --target wasm32-unknown-unknown router::http
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
```

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-worker
git commit -m "fix(tonk-worker): type binary service requests and upstream errors"
```

---

### Task 6: Persist relay metadata and add targeted/revocable invitation controls

**Files:**
- Modify: `rust/tonk-worker-api/src/repository.rs`
- Modify: `rust/tonk-worker-api/src/invite.rs`
- Modify: `rust/tonk-schema/src/domain.rs` (invitation/remote executor attributes and optional raw command fact names)
- Create: `rust/tonk-schema/src/invitation_execution.rs`
- Create: `rust/tonk-schema/src/remote_execution.rs`
- Modify: `rust/tonk-schema/src/lib.rs`
- Modify: `rust/tonk-worker/src/router/repository.rs`
- Modify: `rust/tonk-worker/src/router/wire_compat.rs`
- Modify: `rust/tonk-worker/src/router/create_invite.rs`
- Modify: `rust/tonk-worker/src/router/revoke_invite.rs`
- Modify: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-workspace/src/default_remote.rs`
- Modify: `rust/tonk-workspace/Cargo.toml`
- Modify: `rust/tonk-fab/src/logic.rs`
- Modify: `rust/tonk-fab/src/share.rs`
- Modify: `rust/tonk-fab/src/markup.rs`
- Create: `rust/tonk-fab/src/invitations.rs`
- Modify: `rust/tonk-fab/src/lib.rs`, `fab.css`, and `Cargo.toml`
- Modify: `rust/tonk-cli/src/remote.rs`
- Modify: `rust/tonk-cli/src/invite.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`

**Stored contract:** keep `Invitation` and `Remote` backward-readable. Add `InvitationExecution { this: Invitation::this, kind, revocation_url }` and `RemoteExecution { this: Remote::this, revocation_url }` companion concepts. Newly created records always assert their companion atomically; old records remain queryable but cannot publish a revocation until explicitly configured.

- [ ] **Step 1: Add failing metadata round-trip tests**

Cover:

- remote access URL + relay URL round trip through worker API, meta facts, and CLI records;
- old remotes/invitations with no companion metadata still parse;
- newly minted open/scoped invites record kind and relay;
- claimed invites retain the relay parsed from the public query parameter;
- no stored fact contains the invite URL or fragment.

- [ ] **Step 2: Carry explicit relay config through create/enable-sync**

`RemoteConfiguration` gains optional `revocation_url`. `<tonk-default-remote auto>` fills both the access endpoint (`origin + /ucan/`) and a hidden relay field from `DeploymentConfig`. FAB routeless `enable-sync` claims carry relay as an optional raw fact; worker handlers read it with the existing `text_fact` pattern so frozen command descriptors still match.

A configured sync remote without relay metadata is share-unavailable with an actionable configuration error; it is never guessed from the access hostname.

- [ ] **Step 3: Resolve one typed remote execution record**

Replace `resolve_remote_url` with a result carrying both access and relay URLs. Backup callers consume access only; invite mint consumes both. Both HTTP and command mint paths attach `Invite::with_revocation_url` and assert the invitation companion record.

- [ ] **Step 4: Make invitation revoke use stored metadata and return JSON**

`revoke_invite` finds the exact recorded target, loads its companion relay (or explicit remote relay where unambiguous), locally verifies the artifact, and calls `post_cbor`. Delete every production/staging string/host check. Return 200 JSON with canonical target/artifact CID and publication/idempotence fields; no useful result is put in a discarded 204 body.

- [ ] **Step 5: Add invitation list API and FAB management**

Add a typed `GET /api/repository/{repo}/invites` response containing only target CID, kind, recipient root for scoped invitations, and display/projection status—never path bytes or URLs.

In the existing share menu:

- retain the one-click open-share control;
- add a root-DID field and “Invite identity” action using canonical `recipientRoot` JSON;
- copy the returned targeted URL on success;
- render invitation rows with at least 40×40px revoke actions;
- POST the exact target CID and show published/idempotent success without exposing relay responses.

Use one path-segment encoder for repository and CID values.

- [ ] **Step 6: Add CLI parity**

`tonk remote add` accepts/stores `--revocation-url`; invite mint requires the selected remote's explicit relay and removes staging hostname inference. Existing records without relay produce a clear configuration error and remain listable.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-schema -p tonk-worker-api -p tonk-cli
cargo test -p tonk-fab
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo check -p tonk-fab --target wasm32-unknown-unknown --tests
```

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-worker-api rust/tonk-schema rust/tonk-worker rust/tonk-workspace rust/tonk-fab rust/tonk-cli
git commit -m "fix(invite): route targeted and revoked links through explicit metadata"
```

---

### Task 7: Surface publication acknowledgements without post-revoke reads

**Files:**
- Modify: `rust/tonk-worker-api/src/account.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Modify: `rust/tonk-account-service/src/core/devices.rs` only if a target field/helper is needed
- Modify: `rust/tonk-account-service/src/handlers/devices.rs`
- Modify: `rust/tonk-account-service/src/helpers/server.rs`
- Modify: `rust/tonk-account-service/tests/service.rs`
- Modify: `rust/tonk-worker/src/router/account_devices.rs`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify: `rust/tonk-ui/src/account.rs`
- Modify: `rust/tonk-ui/src/account.html`
- Modify: `rust/tonk-ui/src/account.css`

**Browser acknowledgement:**

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeDeviceAcknowledgement {
    pub target_did: String,
    pub target_cid: String,
    pub published: bool,
    pub projection: RevocationProjection, // updated | stale
}
```

The account service may return a backward-compatible superset retaining `artifactCid`, `stored`, and `attestation` for old callers.

- [ ] **Step 1: Add failing account-service contract tests**

Test first publication, idempotent repeat, projection updated, projection failure/stale, and exact target DID/CID. Both native helper and Worker handler serialize the same camelCase shape and remain successful after R2 publication even when D1 projection fails.

- [ ] **Step 2: Widen the account-service response without changing core ordering**

Reuse `revoke_device`: verified immutable publication remains before `project_revoked`. Add `targetDid` and `published: true` at the handler boundary; retain compatibility fields for one rollout.

- [ ] **Step 3: Change the worker route to return the acknowledgement directly**

Parse the service acknowledgement, verify the target CID is present, and return `RevokeDeviceAcknowledgement`. Delete the mandatory `fetch_devices` after revoke. Self-revocation therefore never calls `/devices/list` with the credential it just withdrew.

- [ ] **Step 4: Separate UI mutation success from list refresh**

For another device, render publication success first, warn on `projection: stale`, and then perform a separate best-effort list refresh. A refresh failure cannot replace success with an error.

For this device, show a terminal “remote access revoked” state, disable account/device refresh actions, and make no further authenticated account calls. Include retry only for failures before an acknowledgement. Repeated publication remains success.

- [ ] **Step 5: Add UI tests**

Cover updated/stale copy, cross-device refresh failure after success, self-revoke with zero list fetches, and idempotent acknowledgement. Keep controls accessible and status announced.

- [ ] **Step 6: Run tests**

```bash
cargo test -p tonk-account-service --features helpers revoke
cargo test -p tonk-worker-api account
cargo test -p tonk-worker --target wasm32-unknown-unknown account_devices
cargo test -p tonk-ui --target wasm32-unknown-unknown account
```

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-worker-api rust/tonk-account-service rust/tonk-worker rust/tonk-ui
git commit -m "fix(revocation): acknowledge publication before refreshing projections"
```

**Deployment note:** account worker first, then access/UI worker.

---

### Task 8: Replace split join URL fields and make failure UI terminal

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs`
- Modify: `rust/tonk-schema/src/command.rs`
- Modify: `rust/tonk-worker-api/src/join.rs`
- Modify: `rust/tonk-worker-api/src/lib.rs`
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-workspace/src/page.rs`
- Create: `rust/tonk-workspace/src/join_retry.rs`
- Modify: `rust/tonk-workspace/src/lib.rs`
- Modify: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-ui/src/identity_gate.rs`
- Modify: `rust/tonk-ui/styles.css`

**Command:** `tonk_schema::command::Join` has one `url` field backed by `dom.event.detail/href`. Remove `Search`, `Hash`, URL reconstruction, and `build_invite_url`.

**Failure vocabulary:** a shared enum serializes `malformed`, `audience-mismatch`, `revoked`, `unavailable`, and `claim-failed`; only `unavailable` is retryable initially. Producers accept legacy `DEVICE_REVOKED` while access services roll to `CREDENTIAL_REVOKED`.

- [ ] **Step 1: Add failing command extraction tests**

Use `<tonk-page>` location detail for an open URL with a fragment and a targeted URL with an empty fragment. Both must decode a `Join { url }`; neither requires `detail.hash`. Assert Debug/log output redacts the value.

- [ ] **Step 2: Change schema and seeded profile descriptor atomically**

Update Rust command/domain types and `profile.yaml` in one commit/task. The command remains transient. The full URL is never asserted durably or copied to failure facts.

- [ ] **Step 3: Add typed, fixed join failures**

Map parse errors, audience mismatch, credential revocation (new and legacy code), remote unavailability, and unclassified commit failures to stable kinds and fixed user messages from the design. `JoinFailure.reason` receives the fixed copy, never `error.to_string()` from an upstream response.

- [ ] **Step 4: Add retry without a bearer navigation message**

`<tonk-join-retry>` dispatches a detail-free `tonk:join-retry` page event. The mounted `<tonk-page>` responds by rebuilding its in-memory location detail and re-dispatching `mount`; it does not navigate, log, or persist the URL. Render the retry control only for retryable kinds. A failure always replaces the spinner with the callout/actions.

- [ ] **Step 5: Keep identity replay on the same path**

An identity-required result retains the `IdentityIntent::DurableJoin` only in gate memory. Successful ceremony replays once through the HTTP join operation and navigates from the typed `JoinResponse`.

- [ ] **Step 6: Add standard-library and browser tests**

Test open and targeted command dispatch, all fixed messages, retry visibility, no raw URL in facts/log capture, and no indefinite pending state.

- [ ] **Step 7: Run tests**

```bash
cargo test -p tonk-schema command::tests
cargo test -p tonk-workspace --target wasm32-unknown-unknown page
cargo test -p tonk-worker --target wasm32-unknown-unknown join
cargo test -p tonk-worker --test standard_library
```

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-schema rust/tonk-core rust/tonk-workspace rust/tonk-worker-api rust/tonk-worker rust/tonk-ui
git commit -m "fix(join): carry the full invite URL into terminal UI states"
```

---

### Task 9: Make remote-backed join externally atomic

**Files:**
- Modify: root `Cargo.toml` and `Cargo.lock` only if Task 1 required a dialog staged-install prerequisite
- Modify: `rust/tonk-worker/Cargo.toml` if volatile storage types need a direct feature/import
- Modify: `rust/tonk-worker/src/session.rs` (extract a generic bounded-session builder)
- Rewrite focused portions of: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-worker/src/router/restore.rs` for extracted mount helpers
- Modify: `rust/tonk-worker/src/router/account_backup.rs` for post-commit backup/capture tests
- Modify: `rust/tonk-worker/src/router/repository.rs` only to split hidden repository preparation from profile-meta visibility
- Modify: `rust/tonk-worker/src/router/navigate.rs` only if test capture needs a seam
- Modify: join route/command tests and helpers

**Core state machine:**

```text
parse → verify audience → build candidate chain → stage proof/repository
      → authorize remote → pull, mutate, and validate staged content
      → install staged content → commit authority/profile/guest state
      → backup → navigate
```

Use owned redacting types:

```rust
struct PreparedJoin { /* no Debug URL */ }
struct StagedJoin { /* candidate chain + verified content metadata */ }
enum JoinMode { GuestVisit, Durable }
async fn prepare_join(...) -> Result<PreparedJoin, JoinFailure>;
async fn stage_join(...) -> Result<StagedJoin, JoinFailure>;
async fn commit_join(...) -> Result<JoinOutcome, JoinFailure>;
```

- [x] **Step 1: Add zero-side-effect failure tests first**

Snapshot before/after:

- profile `Replica` rows/repository list;
- membership/role/provenance rows;
- guest credential site;
- durable candidate certificate presence where observable;
- backup request capture;
- initialized status;
- navigation messages.

Test malformed URL, wrong targeted root, revoked route/403, network outage, unusable initial content, and failed renewal. Every pre-commit failure leaves snapshots equal and emits one terminal classification.

- [x] **Step 2: Extract a generic bounded staging session**

Generalize the core of `session::open` over `Storage<S>` so production still uses `DefaultSpace` while join preflight can use `Storage<VolatileSpace>`. The staging profile is the active profile (same device signer), but its certificate/repository storage is volatile and uses a fresh bounded operator DID.

Retain only:

- the existing `root → device` grant;
- the candidate root-terminated invite chain (durable), or a fresh open-invite delegation to the staged operator (guest);
- the staged profile/session delegation.

- [x] **Step 3: Stage remote authorization, content, and content-branch mutations**

Mount a verifier-only staged repository, attach the exact invite remote, pull `main`, and validate that navigation requirements resolve (at minimum repository identity/name plus the standard `tonk/space` view/model). While candidate authority exists only in the volatile proof store, add the invitation/membership/name/provenance facts to the staged branch and validate the resulting content. A 403 is classified before any durable write. A local-only invitation skips the remote pull but still stages its cryptographically verified content transition.

- [x] **Step 4: Split staged installation from profile visibility**

Refactor `mount_replica` and, if Task 1 proved it necessary, pin the accepted dialog staged-install prerequisite. Helpers must:

1. install the staged branch's exact revision and every reachable archive/blob block under the subject DID in durable storage without asserting profile `Replica` facts or saving candidate authority;
2. verify the installed branch is byte/tree/revision equivalent and usable;
3. assert profile meta and initialized status as the visibility commit.

Do not export/import artifacts into a synthetic commit. On retry, an unindexed installed repository is verified and resumed or replaced deterministically; it never appears in the Hub.

- [x] **Step 5: Commit durable join in safe order**

After staged success:

1. install and verify the staged repository while it is still unindexed;
2. save the accepted candidate chain durably;
3. assert profile replica + initialized state;
4. clear the guest credential only now;
5. dispatch backup only after the local commit;
6. return/navigate only after all required local state is usable.

All content-branch roster/provenance mutations were already part of the promoted staged revision, so no fallible remote/content operation occurs after candidate authority is saved and before visibility.

For an existing replica, stage the renewal first, then save only the accepted chain and update provenance idempotently; a rejected renewal changes nothing. Guest promotion keeps guest authority/marker until step 5.

- [x] **Step 6: Make open visits use the same preflight**

For an open invite, mint a staged guest delegation to the staged operator for preflight, then mint the bounded delegation to the real operator only after success. A revoked or unavailable visit records no guest credential and no visible replica.

- [x] **Step 7: Use one operation from HTTP, command, and promotion routes**

`POST /api/profile/visit`, `POST /api/profile/join`, the `JoinHandler`, and `join_guest` all call the state machine. Remove the swallowed `pull_joined_content`; pull failure is now a typed failure before navigation. Return 200 JSON for promotion acknowledgement instead of useful data behind 204.

- [x] **Step 8: Add success/idempotence tests**

Cover new open visit, guest promotion, targeted join, successful renewal, local-only join, concurrent duplicate attempt, and a post-success invite revocation that leaves local data readable while later sync becomes revoked.

- [x] **Step 9: Run tests**

```bash
cargo test -p tonk-worker join
cargo test -p tonk-worker --target wasm32-unknown-unknown join
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo clippy -p tonk-worker --all-targets --all-features -- -D warnings
```

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock rust/tonk-worker
git commit -m "fix(join): commit replicas only after remote content is usable"
```

---

### Task 10: Return honest typed sync outcomes

**Prerequisite:** Task 1's dialog-db service-response error change is accepted and available at one revision.

**Files:**
- Modify: all `dialog-*` pins in root `Cargo.toml` and `Cargo.lock` if needed
- Modify: `rust/dialog-reactor/src/error.rs` only if the local wrapper must retain a new upstream variant
- Modify: `rust/tonk-access-service/src/error.rs`
- Modify: `rust/tonk-access-service/src/handlers/ucan.rs`
- Modify: `rust/tonk-access-service/README.md`
- Modify: `rust/tonk-worker-api/src/sync.rs`
- Modify: `rust/tonk-worker/src/error.rs`
- Rewrite focused portions of: `rust/tonk-worker/src/router/sync.rs`
- Modify: `rust/tonk-ui/src/api.rs`
- Modify: `rust/tonk-schema/src/replica.rs`
- Modify: `rust/tonk-workspace/src/sync.rs`
- Modify: `rust/tonk-workspace/src/ui_sync_status.rs`
- Modify: `rust/tonk-ui/styles.css`
- Modify: `rust/tonk-fab/src/fab.css` if the FAB disc needs matching states

**Success contract:** 2xx contains only completed or deliberate skipped (`offline`/`paused`) outcomes. Keep `success: true` for one compatibility window if needed, but never serialize `success: false` under 2xx.

**Failure contract:**

| HTTP | Code | Typed source |
|---:|---|---|
| 403 | `CREDENTIAL_REVOKED` | upstream 403 with new or legacy revocation code |
| 409 | `SYNC_CONFLICT` | non-fast-forward/version conflict after bounded retry |
| 503 | `SYNC_UNAVAILABLE` | transport failure or upstream revocation/service 503 |
| 502 | `UPSTREAM_ERROR` | other classified upstream failure |

- [ ] **Step 1: Change access-service vocabulary with rollout compatibility**

Emit `CREDENTIAL_REVOKED` instead of `DEVICE_REVOKED`; the access service knows a CID, not product context. Derive Deserialize for the structured error types needed by the dialog transport. Client classification accepts both codes during rollout. Keep `REVOCATION_UNAVAILABLE` as the access-service code and map it to `SYNC_UNAVAILABLE` at the worker boundary.

- [ ] **Step 2: Pin the accepted typed-error dialog revision**

Update all dialog dependencies together and refresh only required lock entries. Add a focused test that a real access-service 403 reaches `ReactorError::Pull/Push` with status/code intact. **STOP:** if any conversion reduces it to text, fix upstream before continuing.

- [ ] **Step 3: Consolidate duplicate sync DTOs**

Delete router-local `SyncResponse`/`SyncStatusResponse` and use `tonk-worker-api`. Add an explicit success disposition (`completed`, `offline`, `paused`) plus `before`/`after`; no-op callers can distinguish why no reconciliation occurred.

- [ ] **Step 4: Extract a typed reconciliation core**

Make pull, push, and full sync call pure-ish core operations returning `Result<SyncSuccess, SyncFailure>`. Preserve `ReactorError` variants until one mapping function classifies them. Keep the existing bounded head-moved retry, but classify exhausted version/non-fast-forward errors as conflict.

- [ ] **Step 5: Map HTTP only at route boundaries**

Directional and full sync routes return 200 for completed/paused/offline. Failures return the table's status/code with optional before/after and a safe message. Background `sync_repository` consumes the same result; it no longer inspects a boolean buried in 200.

- [ ] **Step 6: Publish distinct live status facts**

Add `sync:revoked`, `sync:conflict`, and `sync:unavailable` beside idle/pending/local/offline/paused. Revoked is terminal until authority changes; conflict is actionable; unavailable is retryable and remains distinct from browser-offline/no-remote.

- [ ] **Step 7: Update all UI consumers**

`tonk-ui::api` parses stable error codes. The workspace pill/badge and `ui-sync-status` disc render revoked, conflict, unavailable, offline, paused, and normal drift distinctly with fixed labels/ARIA text. Unknown values still fail safely but do not masquerade as offline in tests.

- [ ] **Step 8: Add route and UI tests**

Use scripted typed errors, not strings. Assert HTTP status alone distinguishes success, and test legacy/new revocation code, conflict, revocation-source outage, generic upstream 5xx, paused, and browser offline.

- [ ] **Step 9: Run tests**

```bash
cargo test -p tonk-access-service --features helpers
cargo test -p tonk-worker-api sync
cargo test -p tonk-worker sync
cargo test -p tonk-worker --target wasm32-unknown-unknown sync
cargo test -p tonk-workspace --target wasm32-unknown-unknown sync
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock rust/dialog-reactor rust/tonk-access-service rust/tonk-worker-api rust/tonk-worker rust/tonk-schema rust/tonk-workspace rust/tonk-ui rust/tonk-fab
git commit -m "fix(sync): expose authorization and reconciliation failures honestly"
```

---

### Task 11: Resolve template bindings before they reach network consumers

**Files:**
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-worker/tests/standard_library.rs`
- Modify: `rust/tonk-display/src/render.rs`
- Modify: `rust/tonk-fab/src/element.rs`
- Modify: `rust/tonk-fab/Cargo.toml` (add a path-segment encoding dependency only if needed)
- Modify: FAB tests

- [ ] **Step 1: Add failing source and renderer guards**

In `standard_library.rs`, assert the profile space chrome uses `space={id}` and contains no quoted network-bearing `{id}` binding.

In `tonk-display`'s browser renderer tests, mount the actual multi-root space-chrome shape with a known DID and assert `tonk-site`, `tonk-fab`, and network-bearing custom-element attributes contain no unresolved `{name}`. Literal braces in text/code remain legal.

- [ ] **Step 2: Fix the template**

Change only:

```html
<tonk-fab with="main@profile:tonk" space={id}></tonk-fab>
```

Keep the other bindings and routing context unchanged.

- [ ] **Step 3: Encode membership paths as one segment**

Extract a pure `membership_endpoint(space)` helper. Reject empty/unresolved brace values and percent-encode the resolved DID as one path segment before GET/POST. Use it for both membership check and guest promotion.

- [ ] **Step 4: Run tests**

```bash
cargo test -p tonk-worker --test standard_library
cargo test -p tonk-display --target wasm32-unknown-unknown render
cargo test -p tonk-fab
cargo check -p tonk-fab --target wasm32-unknown-unknown --tests
```

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-core rust/tonk-worker/tests/standard_library.rs rust/tonk-display rust/tonk-fab
git commit -m "fix(fab): resolve and encode repository bindings before fetch"
```

---

### Task 12: Pin Crane's remarshal dependency to Python 3.13 on Darwin

**Files:**
- Modify: `flake.nix`

- [ ] **Step 1: Add the narrow overlay**

Add one overlay to the repository `pkgs` import that changes only top-level `remarshal` on Darwin:

```nix
(final: prev: prev.lib.optionalAttrs prev.stdenv.isDarwin {
  # Remove when nixpkgs remarshal passes with Python 3.14 on Darwin.
  remarshal = final.python313Packages.remarshal;
})
```

Use the exact nixpkgs attribute spelling confirmed by `nix eval`. Do not replace global `python3`, `commonBuildInputs`, Crane's whole package set, or Wrangler's package set.

- [ ] **Step 2: Verify the derivation closure**

Run:

```bash
nix fmt flake.nix
ROOT=$(nix eval --raw .#tonk-cloudflare-artifacts.drvPath)
nix-store -qR "$ROOT" | grep remarshal
```

Expected on Darwin: the relevant remarshal derivation is Python 3.13; unrelated Python 3.14 tools may remain.

- [ ] **Step 3: Build from the committed flake shape**

```bash
nix build .#tonk-cloudflare-artifacts --no-link
```

Expected: succeeds without an external overlay. Record cache/network failures separately from package build failures.

- [ ] **Step 4: Commit**

```bash
git add flake.nix
git commit -m "fix(nix): build crane remarshal with python 3.13 on darwin"
```

---

### Task 13: Complete documentation, full gates, and staging smoke

**Files:**
- Modify: `rust/tonk-ui/README.md`
- Modify: `rust/tonk-worker/README.md`
- Modify: `rust/tonk-access-service/README.md`
- Modify: `rust/tonk-account-service/README.md`
- Modify: `rust/tonk-invite/README.md` if present
- Modify: CLI help/examples and deployment runbook documentation
- Modify: `docs/superpowers/plans/implementation-notes.md` only for durable deviations discovered during execution

- [ ] **Step 1: Document the final contracts**

Document canonical browser JSON/alias removal date, deployment config endpoint, explicit relay metadata, media types, revocation acknowledgements/projection semantics, sync statuses/codes, join preflight/commit behavior, staging deployment order, and non-destructive smoke policy.

- [ ] **Step 2: Run focused crate gates**

```bash
cargo test -p tonk-identity
cargo test -p tonk-worker-api -p tonk-schema -p tonk-cli
cargo test -p tonk-account-service --features helpers
cargo test -p tonk-access-service --features helpers
cargo test -p tonk-fab
cargo test -p tonk-worker --test standard_library
cargo check -p tonk-worker --target wasm32-unknown-unknown --tests
cargo check -p tonk-ui --target wasm32-unknown-unknown --tests
```

- [ ] **Step 3: Run workspace/Nix gates**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
nix build .#tonk-cloudflare-artifacts --no-link
nix develop -c test:native:debug
nix develop -c test:web:debug
```

Then, resources permitting:

```bash
nix develop -c test:native:release
nix develop -c test:web:release
```

- [ ] **Step 4: Deploy in safe order**

1. account relay/service response additions;
2. access service typed error vocabulary/config endpoint;
3. UI/service worker.

During the mixed window, account responses are supersets, invite input aliases remain accepted, and clients recognize both revocation codes.

- [ ] **Step 5: Run the non-destructive staging smoke**

With fresh browser data and disposable account emails:

- owner/root + synced spot through visible gate;
- second-device restore;
- cross-device revoke with D1 forced stale-active;
- self-revoke acknowledgement;
- guest visit/promotion/open-invite revoke;
- targeted T success/W mismatch;
- manual sync returns non-2xx typed revocation;
- no literal `{id}`, indefinite spinner, `Model not found`, or conversion exception.

Confirm R2 enforcement within one refresh interval and sibling authorization remains. Preserve `tonk-spaces-staging`; reset only novel account/revocation rows/objects when schema compatibility requires it.

- [ ] **Step 6: Remove compatibility aliases only in a later release**

Do not remove `base_url`/`recipient_root` aliases or legacy `DEVICE_REVOKED` client acceptance in this branch. File/follow the cleanup once every shipped caller uses canonical fields/codes.

- [ ] **Step 7: Commit documentation**

```bash
git add docs rust/tonk-ui/README.md rust/tonk-worker/README.md rust/tonk-access-service/README.md rust/tonk-account-service/README.md rust/tonk-invite rust/tonk-cli
git commit -m "docs(identity): document hardened sharing and revocation operations"
```

---

## Final review checklist

- [ ] No UI module except `identity_bridge.rs` accesses `window.tonkIdentity`.
- [ ] Every ceremony input is a plain object with tested camelCase properties.
- [ ] Identity gate is visible above FAB/portals, focus-safe, cancellable, retryable, and exactly-once.
- [ ] Unknown invite JSON fields return 400; omitted browser base URL uses the request origin.
- [ ] Unknown/local hosts never default to production account or relay URLs.
- [ ] Every non-empty raw body has an explicit media type and bounded timeout/error body.
- [ ] 204/205/304/HEAD browser responses carry no body stream and conversion never throws.
- [ ] Self-revoke publishes and acknowledges without a post-revoke authenticated read.
- [ ] Projection failure cannot turn immutable publication success into UI failure.
- [ ] Invitation relay selection comes only from explicit remote/invitation metadata.
- [ ] Targeted invite, invitation list, and invite revoke are exercisable through the FAB.
- [ ] Join command carries one full URL and no fragment field is required.
- [ ] Wrong-recipient/revoked/unavailable joins leave profile list, roster, guest marker, backup, and navigation unchanged.
- [ ] Successful navigation occurs only after required content is usable.
- [ ] Guest promotion clears guest state only in the final durable commit.
- [ ] Sync 2xx means completed or deliberate no-op; failure status/code is sufficient for callers.
- [ ] New and legacy revocation codes classify as credential revocation during rollout.
- [ ] Revoked, conflict, unavailable, offline, and paused are distinct visible states.
- [ ] No network-bearing attribute/path contains unresolved `{id}` and repository DIDs are encoded as one segment.
- [ ] `nix build .#tonk-cloudflare-artifacts` uses Python 3.13 only for the narrow remarshal dependency.
- [ ] The non-destructive staging smoke passes with `tonk-spaces-staging` intact.
