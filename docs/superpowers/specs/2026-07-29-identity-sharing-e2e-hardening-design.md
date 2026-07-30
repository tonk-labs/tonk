# Identity and sharing E2E hardening

Release-hardening design for the root-first identity, device revocation,
guest promotion, and targeted-invitation work. Written 2026-07-29 from a
serial staging exercise against `feat/in-band-revocation`.

The cryptographic model passed: durable authority followed
`space → root → device → session`, signed R2 artifacts overruled stale
D1 projections, and targeted invitations bound to root DIDs. The
product did not pass end to end. Several browser, HTTP, and state-machine
boundaries required DevTools workarounds or reported success after an
operation had failed.

This design closes those integration defects without changing the
authority model in
`2026-07-27-in-band-revocation-design.md`.

## Problem

The implementation has correct pieces connected by weak contracts:

- Rust values passed to `window.tonkIdentity` sometimes become JavaScript
  `Map` objects, while the identity API reads ordinary object properties.
- Browser routes silently accept misspelled JSON fields and may fall back
  from staging to a production URL.
- Raw CBOR helpers do not describe their media type, so a valid
  revocation is rejected by the relay.
- HTTP responses and UI states can report success after the remote
  rejected access.
- Join writes durable local state before proving that the invited remote
  can be read.
- The join command requires an open-invite fragment even though targeted
  invitations intentionally have none.
- Response conversion gives bodyless statuses a body stream, which the
  browser rejects.
- Route-template interpolation can leak a literal `{id}` into an API
  request.
- The Nix build depends on a Python package combination that is broken on
  the current Darwin lock.

These are not independent cosmetic bugs. Together they make a security
feature hard to exercise and harder to trust: the server may enforce a
revocation while the local API returns 200, the join page navigates to
an unusable replica, or the UI reports a failed revocation that was
already published permanently.

## Observed defects

| Area | Staging observation | Root cause |
|---|---|---|
| Root gate | The identity dialog existed but was not visible | The top-document gate has no stylesheet or stacking contract |
| Root ceremony | `missing or invalid deviceDid` | `serde_json::Value` serialized through `serde_wasm_bindgen` became a JS `Map` |
| Cross-device revoke | `missing or invalid delegationCid` | The same bridge defect affected `signRevocation` |
| Self-revoke | R2 enforcement succeeded, but the UI returned 500/403 | The route refreshed the protected device list using the credential it had just revoked |
| Invite revoke | Worker returned 500; relay returned 400 | The worker omitted `Content-Type: application/cbor` |
| Targeted join | The page remained on “Joining…” | The command required `detail.hash`; targeted URLs have no fragment |
| Revoked join | Remote returned `DEVICE_REVOKED`, then the UI opened “Model not found” | Join persisted and navigated after a failed initial pull |
| Guest promotion | Mutation committed, then fetch threw `Could not construct fetch response` | The service-worker adapter attached a stream to a 204 response |
| Invite origin | A staging mint produced a `tonk.spot` URL | The API accepted `base_url`, ignored `baseUrl`, then used the production default |
| Membership lookup | Browser requested `/api/repository/{id}/membership` | A quoted template binding preserved the braces literally |
| Manual sync | Outer route returned 200 while `/ucan/` returned 403 | Failure was encoded only as `success: false` in a nominally successful response |
| Nix build | The artifact build required a local Python 3.13 override | The locked Python 3.14 `remarshal` package is broken on Darwin |

## Goals

- Every documented identity, account, invitation, revocation, and guest
  flow works through the visible UI without DevTools.
- JavaScript and HTTP boundaries have one typed, tested wire contract.
- A successful response means the requested operation completed. A
  security denial is never hidden inside HTTP 200.
- A failed first join leaves no visible replica, roster entry, backup,
  or navigation side effect.
- All join failures reach a terminal, actionable UI state without
  exposing bearer material.
- Staging and local deployments never silently mint links for another
  environment.
- `nix build .#tonk-cloudflare-artifacts` succeeds from the committed
  flake on Apple Silicon.
- The full scenario runs against disposable local services and browser
  profiles without deleting shared staging data.

## Non-goals

- Changing the root-first authority chains or revocation-artifact format.
- Making open invitations single-use, read-only, or non-transferable.
- Migrating pre-root spaces or preserving the novel account-service
  staging rows created during development.
- Root rotation or lost-passkey recovery.
- Redesigning the account or FAB surfaces beyond the error and identity
  states needed here.
- Treating D1 status as authorization. It remains a projection.

## Invariants

### Authority

- Durable authority remains
  `space → root → device → session`.
- Guests may use open bearer authority without a root and receive no
  durable roster row.
- Promotion and targeted invitation record the root DID, never the
  device DID.
- R2 signed artifacts are canonical. D1 device status may lag, fail to
  update, or be changed manually without altering authorization.

### Boundary honesty

- A JavaScript API documented as taking an object receives a plain
  JavaScript object with named properties.
- Unknown JSON fields are rejected rather than ignored.
- Every non-empty body is sent with an explicit media type.
- HTTP 2xx means the requested state transition completed.
- A null-body HTTP status is converted to a browser response with a null
  body.

### Join atomicity

- Parsing, audience verification, and remote authorization happen before
  durable profile or roster changes.
- A remote-backed join is not complete until initial content is
  available locally.
- Failure leaves the user's pre-attempt profile and repository list
  unchanged.
- Bearer URLs and fragments never enter logs, error text, durable facts,
  analytics, or navigation messages.

## Design

### One identity bridge

Move all `window.tonkIdentity` calls behind one module in `tonk-ui`.
`identity_gate.rs` and `account.rs` must not each maintain their own
reflection and serialization code.

The bridge exposes typed operations:

```rust
create_root(CreateRootInput { device_did })
evaluate_root(EvaluateRootInput { device_did })
create_account(CreateAccountInput { ... })
complete_link(CompleteLinkInput { ... })
sign_revocation(SignRevocationInput {
    delegation_cid,
    path_hex,
})
```

Inputs are Rust structs with `#[serde(rename_all = "camelCase")]`.
Serialization uses
`serde_wasm_bindgen::Serializer::json_compatible()` or an equivalent
path proven to create a normal JS object. Call sites must not pass
`serde_json::Value` across this boundary.

The bridge owns:

- locating and validating `window.tonkIdentity`;
- plain-object input serialization;
- promise validation and awaiting;
- typed output deserialization; and
- stable, user-readable error classification.

Unit tests install a fake `tonkIdentity` object and assert property
access (`input.deviceDid`, `input.delegationCid`, `input.pathHex`) works.
They also assert `input instanceof Map === false`. Browser tests run the
real methods with the CDP virtual authenticator.

### A visible, accessible identity gate

The identity gate injects a dedicated stylesheet once, following the
existing account-element pattern. The gate is a top-document fixed
overlay above the Tonk iframe and FAB, with an opaque-enough backdrop,
a visible card, and explicit light/dark tokens.

When it opens:

- focus moves to the primary action;
- background content cannot receive pointer input;
- the status line announces passkey progress and errors;
- a failed ceremony leaves retry and cancel actions available;
- cancel closes the gate without replaying the durable operation; and
- success saves the root, closes the gate, and replays the original
  intent exactly once.

Concurrent identity requests remain serialized. A second request does
not replace the first intent or create another ceremony.

CLI linking uses the same bridge and visual foundation, but keeps its
challenge and copy-response content.

### Canonical browser JSON

Browser-facing JSON uses camelCase. The invite request and response are:

```json
{
  "baseUrl": "https://staging.tonk.xyz/join",
  "recipientRoot": "did:key:..."
}
```

```json
{
  "kind": "scoped",
  "url": "https://staging.tonk.xyz/join?access=...",
  "recipientRoot": "did:key:..."
}
```

`base_url` and `recipient_root` remain input aliases for one release so
the CLI and old callers can move independently. Unknown fields produce
400 with a structured error. In particular, a misspelling cannot turn a
requested staging URL into a default production URL.

For the browser worker route, an omitted `baseUrl` is derived from the
incoming request origin as `{origin}/join`. The generic
`tonk_invite::DEFAULT_BASE_URL` remains available to non-browser callers,
but is not the browser route's fallback.

The UI, helper snippets, shared DTOs, README examples, and tests all use
the canonical field names. The response field migration follows the
same alias window where deserialization is involved.

### Typed binary HTTP

Replace the ambiguous `post_for_bytes` helper with operations that name
their protocol:

- `post_cbor` sends `Content-Type: application/cbor`;
- `post_json` sends `Content-Type: application/json`; and
- any future opaque-byte call must supply a media type explicitly.

The wasm and native implementations set identical headers, timeouts, and
error behavior. Non-2xx errors retain the upstream status, structured
error code, and bounded response text rather than reducing everything
to `account-service returned HTTP N`.

Invitation revocation posts its verified artifact with `post_cbor`.
Relay selection comes from configured remote/invitation metadata, not a
substring search for `staging`.

### Browser response conversion

`ResponseConversion` distinguishes responses that may carry a body from
responses that must not:

- 204, 205, and 304 use a null browser body;
- a response to `HEAD` uses a null browser body; and
- other statuses keep the current streaming conversion.

Headers and status survive either path. Routes returning 204 must not
encode useful result data in the discarded body; a route needing an
acknowledgement returns 200 with JSON instead.

Service-worker wasm tests cover 204, 205, 304, `HEAD`, an empty 200, and
a streamed JSON response. No conversion path uses `expect_throw`.
Conversion errors return a controlled 500 response and log the original
route/status without secret request data.

### Revocation acknowledgements

Publishing the immutable artifact is the security-bearing success.
Refreshing a mutable device list is a separate read.

`POST /api/account/devices/revoke` returns a typed acknowledgement:

```json
{
  "targetDid": "did:key:...",
  "targetCid": "bafy...",
  "published": true,
  "projection": "updated"
}
```

`projection` may be `updated` or `stale`. If R2 publication succeeded
but D1 projection failed, the route remains successful and the UI warns
that the device list may take time to catch up. It must not imply that
access remains valid.

For another device:

1. The root signs the exact recorded grant CID through the typed identity
   bridge.
2. The account service verifies and publishes the artifact.
3. The UI may refresh the list because the caller remains authorized.

For this device:

1. The device mints its self-revocation without a passkey prompt.
2. The account service verifies and publishes it.
3. The worker returns the acknowledgement without calling
   `/devices/list` with the newly revoked credential.
4. The UI shows a terminal success state and stops authenticated account
   refreshes for that device.

Re-publishing the same artifact is idempotent and returns success.
Cross-device and invitation revocation follow the same publication
acknowledgement semantics.

### Honest sync responses

Refactor branch synchronization into a core operation returning a typed
`Result`, then map that result at the HTTP boundary.

Successful reconciliation returns 200. Deliberate no-ops such as
`paused` and `offline` remain 200 but identify the skipped state in the
response.

Failures use non-2xx statuses and stable codes:

| Condition | Status | Code |
|---|---:|---|
| A presented chain contains a revoked CID | 403 | `CREDENTIAL_REVOKED` |
| Branch histories cannot reconcile | 409 | `SYNC_CONFLICT` |
| Revocation state or remote is temporarily unavailable | 503 | `SYNC_UNAVAILABLE` |
| An unclassified upstream failure | 502 | `UPSTREAM_ERROR` |

The implementation preserves typed upstream failures rather than
matching display strings. The response may still contain `before`,
`after`, and an error description, but `success: false` is not hidden in
HTTP 200.

`CREDENTIAL_REVOKED` replaces the overly specific
`DEVICE_REVOKED`: the access service knows that a delegation CID was
revoked, not whether the hop represented a device, invitation, or other
credential. Callers add context. The join UI renders it as an invite
revocation; device sync renders it as device access revoked. Clients
accept the legacy code during the rollout window.

The sync chip consumes the same typed result and renders revoked,
offline, conflict, and retryable states distinctly. A manual API caller
can determine success from the status without inspecting console logs.

### One full-URL join command

Replace the join command's required `search` and `hash` fields with one
required `url` field populated from top-document `location.href`.

This keeps the open-invite fragment available while also representing a
targeted URL whose fragment is empty. It matches the existing
`POST /api/profile/join` DTO, removes URL reconstruction from
`JoinHandler`, and prevents blank optional fields from stopping command
dispatch.

The full URL exists only in transient event detail and function memory.
Logs and durable facts use the repository subject, invitation target
CID, or a redacted URL without `access` and fragment.

### Transactional join state machine

Open visit, durable open-invite promotion, and targeted join share a
state machine:

```text
idle
  → parsing
  → verifying audience
  → authorizing remote
  → fetching initial content
  → committing local state
  → navigating
```

Any failure before `committing local state` transitions to a terminal
error and leaves durable state unchanged.

For a remote-backed invite:

1. Parse the URL and derive the invitation record without writing.
2. Verify that the current root is the targeted audience, or that the
   open bearer seed matches.
3. Claim the chain in temporary memory.
4. Use that chain to resolve and fetch the remote branch into a staged
   repository or temporary proof store.
5. Only after the remote authorizes and the initial content is usable,
   persist the chain, mount the replica, record guest or durable
   membership, back up the claim, mark initialized, and navigate.

The implementation may use a staged repository or an explicit rollback,
but its externally observable behavior is fixed:

- a 403 leaves no profile replica, roster row, guest credential, claim
  backup, or navigation;
- a network failure leaves no half-installed replica and offers retry;
- a successful navigation always lands on content containing the
  required space model; and
- an existing replica remains unchanged if a renewal invite fails
  validation.

A local-only invitation has no remote preflight and commits after local
cryptographic verification.

Guest promotion is an existing-replica transition: the guest credential
and guest membership remain in place until the durable root claim passes
remote authorization. `clear_guest` is part of the final commit, never
an optimistic preflight write.

If an invite is revoked after a successful join, existing local data
remains available but subsequent sync enters the visible revoked state.
Revocation controls remote access, not local erasure.

### Join error UX

The join overlay always reaches success, identity-required, or failure.
There is no indefinite spinner.

Failures are classified before display:

| Kind | User message |
|---|---|
| `malformed` | This invite link is invalid. |
| `audience-mismatch` | This invite was issued to a different identity. |
| `revoked` | This invite has been revoked. |
| `unavailable` | Tonk could not reach this spot. Try again. |
| `claim-failed` | Tonk could not join this spot. |

Retry is offered only for retryable failures. The UI never displays the
raw URL, bearer seed, delegation bytes, or upstream response body.

An identity-required result opens the identity gate with the original
full URL retained in memory. After a successful ceremony the join is
replayed once through the same state machine.

### Template bindings are data, not strings

The space chrome passes the repository binding as `space={id}`, not
`space="{id}"`. The FAB receives the resolved DID and the membership API
encodes it as one path segment.

A renderer test mounts the space template with a known repository DID
and asserts every network-bearing custom-element attribute is fully
resolved. The test fails if a value containing an unresolved `{name}`
reaches `tonk-fab`, `tonk-site`, or an API URL.

This guard is intentionally narrow. Literal braces remain legal in text
content and code examples.

### Reproducible Darwin build

Carry the working Python choice in the flake rather than in a developer's
temporary overlay. Override only the affected `remarshal` package (or
the derivation that brings it into the Cloudflare artifact closure) to
Python 3.13 while the locked package is incompatible with Python 3.14.

Do not downgrade the repository's global `python3` or the whole package
set. The override includes a comment naming the upstream incompatibility
and is removed once the locked nixpkgs package passes on Python 3.14.

`nix build .#tonk-cloudflare-artifacts` must pass on a clean Apple
Silicon machine using only the committed flake and lock file.

## Local E2E environment

The staging runbook remains a deployment smoke test, not the primary
correctness suite. A deterministic local harness starts:

- the UI/service worker;
- the account worker with disposable D1 and chain/revocation R2 buckets;
- the access worker bound read-only to the same revocation bucket;
- a mail-code stub; and
- isolated Chromium contexts with CDP virtual authenticators.

Every run owns its database, buckets, ports, browser profiles, and test
email namespace. Teardown removes only paths created under the run's
temporary directory. No command can resolve to a shared staging
resource.

The harness exposes named profiles matching the manual model:

| Profile | Purpose |
|---|---|
| A | Owner and first device |
| B | Restored second device |
| C | Self-revoking device |
| G | Rootless guest, then promoted member |
| T | Targeted recipient |
| W | Wrong targeted recipient |

Tests use UI actions for product flows. Direct API calls are reserved
for setting up server projections, such as deliberately making D1 stale.
No test injects a corrected request that the UI itself cannot produce.

The visible browser mode remains available for diagnosis, but headless
is the default gate. Browser startup is owned by the test process and
uses one explicit binary/profile pair so an unrelated Chrome lifecycle
cannot make the runner report “Google Chrome is not open”.

## Test matrix

### Unit and service-worker tests

- Every identity input is a plain JS object with the expected camelCase
  properties.
- Invite JSON accepts canonical camelCase, temporarily accepts documented
  aliases, rejects unknown fields, and derives the current origin when
  `baseUrl` is absent.
- Wasm and native CBOR helpers send the same content type and preserve
  upstream errors.
- Browser response conversion handles all null-body cases.
- Self-revocation returns an acknowledgement without listing devices.
- Sync failures map to non-2xx statuses and stable codes.
- A resolved space template never emits `{id}` in an attribute or URL.

### Local browser scenarios

1. Create A's root through the visible gate while the account service is
   unavailable; create and sync a spot without account traffic.
2. Attach and disconnect an account; assert root, device, grant CID, and
   grant bytes never change.
3. Restore B through the existing passkey; assert same root, new device,
   new grant, and restored synced spaces.
4. Change only B's D1 status; assert sync still works.
5. Revoke B through A; make D1 stale-active; assert B receives a visible
   403 state and A still syncs.
6. Self-revoke C without a passkey prompt; assert the UI reports success,
   does not refresh with C's credential, and C later receives 403.
7. Visit as G without a root; assert guest membership, no durable guest
   roster row, and successful read/write/sync.
8. Promote G through the visible gate; assert durable membership uses
   G's root, the request completes cleanly, and no 204 conversion error
   occurs.
9. Revoke the open invitation through the UI; assert G and a completely
   fresh visitor receive a visible revoked error and no new replica is
   recorded.
10. Mint a targeted invitation to T; assert W sees the
    audience-mismatch error with no replica, while T joins through the UI
    and is recorded by root DID.
11. Force a remote outage during join; assert retry UI and zero durable
    side effects.
12. Force a sync conflict and an upstream outage; assert their HTTP and
    UI states are distinct from revocation.

### Staging smoke

After the local suite passes:

- deploy account relay, then access/UI worker;
- use fresh site data and disposable accounts;
- preserve `tonk-spaces-staging`;
- run one owner, second-device, guest, and targeted-recipient path;
- make D1 stale-active after one signed revocation; and
- confirm the access worker rejects the revoked CID within one refresh
  interval.

Staging reset is limited to novel account and revocation resources when
the schema requires it. Existing spot data is never deleted as a
convenience for this suite.

## Ownership

| Boundary | Primary code |
|---|---|
| Identity bridge and gate | `rust/tonk-ui/src/identity_gate.rs`, `account.rs`, new shared bridge/style |
| Invite DTO and origin | `rust/tonk-worker/src/router/create_invite.rs`, shared worker API |
| CBOR publication | `rust/tonk-worker/src/router/account_backup.rs`, `revoke_invite.rs` |
| Revocation acknowledgement | `account_devices.rs`, account service device handlers, account UI |
| Response conversion | `rust/tonk-worker/src/axum.rs` |
| Join command and transaction | `rust/tonk-core/assets/library/profile.yaml`, schema command/domain types, `router/join.rs` |
| Membership binding | profile space view and FAB membership consumer |
| Sync status | `rust/tonk-worker/src/router/sync.rs`, shared sync DTO, sync UI |
| Reproducible build | `flake.nix` and locked package inputs |
| Local E2E | Nix test apps, UI integration harness, local worker configuration |

## Rollout

1. Land boundary tests that reproduce every staging failure.
2. Fix the shared identity bridge and response conversion first; later
   UI flows depend on both.
3. Land typed HTTP publication and revocation acknowledgements.
4. Replace the join event shape and make join externally atomic.
5. Fix template binding and honest sync responses.
6. Add the narrow Nix override and local E2E app.
7. Run the local matrix, then the non-destructive staging smoke.
8. Remove snake_case aliases after all shipped clients use the canonical
   DTO.

The account worker deploys before the access/UI worker when response
shapes change. During the short mixed-version window, aliases and
backward-readable acknowledgements keep old callers functional.

## Rejected alternatives

### Patch the two failing `serde_json::Value` call sites

Rejected. The account and gate modules already duplicated the same
reflection bridge. A third call would recreate the bug. One typed bridge
makes the JavaScript shape testable.

### Keep `200 { "success": false }` for sync

Rejected. It made a revoked device look successful to the runbook and
ordinary callers. Security denials and retryable outages need honest
statuses and codes.

### Treat initial pull failure as a successful join

Rejected. It leaves a durable but unusable replica and turns a clear 403
into “Model not found”. Local state may be retained only after the
remote-backed invitation is usable.

### Require an empty hash field for targeted invitations

Rejected. Optional URL components should not control command dispatch.
Passing the full top-document URL is simpler and matches the HTTP API.

### Infer staging from host substrings or use production as a browser fallback

Rejected. Environment is configuration, not a naming convention. The
incoming browser origin and explicit remote metadata are authoritative.

### Refresh the device list as part of revocation

Rejected. Mutation success and list readability have different
authorization lifetimes. Coupling them necessarily misreports
self-revocation.

### Downgrade all Python packages

Rejected. The incompatibility is narrow. A global downgrade hides which
dependency needs removal and perturbs unrelated tools.

### Keep staging as the only full-system test

Rejected. The serial run requires shared infrastructure, passkey
profiles, manual D1 changes, and minute-long refresh waits. A disposable
local harness makes failures reproducible without risking spot data.

## Acceptance criteria

The branch is ready when:

- every local browser scenario passes with no Console intervention;
- the UI produces no literal `{id}` request, indefinite join spinner,
  `Model not found` landing, or response-conversion exception;
- root and cross-device ceremonies receive plain JS objects;
- self-, root-, and invite-revocation UI actions publish successfully
  and report their true result;
- wrong-recipient, revoked, unavailable, and malformed joins leave no
  durable side effects and render distinct errors;
- successful joins navigate only after usable content is present;
- sync exposes remote authorization failures as non-2xx structured
  responses;
- a stale-active D1 row cannot restore an R2-revoked route;
- sibling devices remain authorized after one device is revoked;
- account attachment and disconnection preserve existing authority;
- `nix build .#tonk-cloudflare-artifacts` passes without an external
  overlay; and
- the non-destructive staging smoke passes while
  `tonk-spaces-staging` remains intact.
