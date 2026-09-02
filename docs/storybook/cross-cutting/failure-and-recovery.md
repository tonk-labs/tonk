# Failure and recovery

## Summary

Every Tonk failure must answer three questions: what the user asked, which
durable boundaries were crossed, and what safely happens next. “Error” is not a
single state. A validation rejection before side effects, a local commit whose
push failed, a remote commit whose response was lost, and a malformed local
session all require different output and recovery.

This document is the shared error-state matrix for every journey. A feature may
declare an error not applicable, but it may not silently omit the question.

## Outcome vocabulary

| Outcome | Durable meaning | Required user result |
| --- | --- | --- |
| `unchanged` | No local or remote boundary was crossed. | Specific rejection/cancellation and corrected input or prerequisite. Safe to retry directly. |
| `committed-local` | Essential local work committed; remote work did not. | Say local work is retained/ahead/deferred and name the sync/retry action. |
| `committed-remote` | Remote service accepted; local observation/persistence failed. | Say the result may already exist and direct status/login/reconcile, never blind create/delete retry. |
| `committed` | All essential durable stages completed. | Success, even if optional notification/telemetry/output later failed. Warn about optional failures separately. |
| `partial` | A multi-target or multi-stage operation has known successes and failures. | Enumerate exact completed/failed subjects and a safe continuation. |
| `unknown` | Request was sent but commit acknowledgement was lost and state cannot yet be read. | State uncertainty explicitly; retry only through an idempotency key or reconciliation. |
| `blocked` | The operation cannot proceed until authority, service, state repair, or a product decision changes. | Name the blocking state and preserve all unrelated/local behavior. |

No output may use “failed” alone when the actual outcome is
`committed-local`, `committed-remote`, `partial`, or `unknown`.

## Error families

| Family | Examples | Before boundary | After boundary | Minimum evidence |
| --- | --- | --- | --- | --- |
| Input/usage | Missing flag, invalid email/name/DID/URL/notation/CSV, ambiguous account-space name. | `unchanged`; focus/usage points to exact field/token. | Validation should already have run; if target changed concurrently, classify conflict. | Table-driven parser/form plus process/browser error. |
| Missing prerequisite | No selected space, no account/root/provider/upstream, unconfigured/unhydrated account. | `unchanged` or `blocked`; name setup/retry. | If prerequisite disappeared concurrently, preserve earlier commit and refresh. | Every state-model coordinate. |
| User cancellation | Back/Cancel, confirmation decline, WebAuthn cancel, Ctrl-C. | `unchanged`. | Report checkpoint; cancel optional later work without lying about committed authority/data. | Before/after every non-atomic stage. |
| Authentication | Wrong/missing passkey, invalid grant/proof/signature/audience, expired activation link. | `unchanged`; no authority installed. | If remote registered before invalid/lost callback, reconcile attachment. | Identity contract plus real authenticator. |
| Authorization | Revoked device/invite, foreign owner, joined-space delete, suspended customer. | `blocked`; local data boundaries visible. | Abort stale in-flight work and ensure later retries cannot resurrect authority. | Multi-actor whole journey. |
| Conflict/stale state | Duplicate email/name, already active, stale deletion plan, concurrent head/generation change. | `unchanged` or idempotent existing result. | Reconcile exact subject/generation; never substitute by display label. | Two actors plus response-lost replay. |
| Connectivity | DNS/connect, offline, reset, timeout. | Local-only work proceeds or remote-required work stays unchanged. | Classify local/remote/unknown commit; bounded wait and retry. | Fault at connect, send, receive, and status read. |
| HTTP/protocol | Relevant 4xx/5xx, redirect, wrong content type, malformed/truncated JSON/CBOR/hex/base64. | Preserve input and show curated safe error. | Do not infer rejection from decode failure after a successful status or infer commit from an error body. | Contract table for every endpoint response class. |
| Local durability | Missing/read-only/full/locked store, unsupported version, malformed JSON, partial legacy records, atomic rename failure. | Fail before remote effect where knowable. | Recover from checkpoint; remote success remains authoritative. | Store tests plus fresh-process restart. |
| Process/page lifecycle | Reload, tab close, browser crash, SIGINT/SIGTERM/SIGHUP, kill after write. | `unchanged`. | Fresh run reconstructs from durable state and never duplicates authority/data. | Deterministic fault point at each stage. |
| Concurrency | Double click, Enter+blur, two CLI processes, two tabs, second device. | Serialize/deduplicate or reject. | Generation/head/idempotency check; no lost update or stale resurrection. | Real processes/tabs/independent actors. |
| Output channel | Broken pipe, closed stdout/stderr, unwritable/full output path, browser DOM disconnect. | May remain unchanged if output is the only product. | Output failure cannot roll back or repeat committed mutation; next status reveals truth. | Broken-pipe/file fault after commit. |
| Deployment/runtime | Wrong service origin/DID, stale service worker, mixed assets, missing config, unsupported browser/authenticator. | Visible `blocked` recovery, no indefinite busy state. | Preserve local state; coherent reload/update only. | Built/deployed browser smoke. |

## Browser response contract

Browser API clients must branch on HTTP status before decoding a success type.
For every endpoint, test:

1. success status with valid body;
2. success status with missing/malformed body;
3. each curated client error with a valid error envelope;
4. unauthorized/revoked/expired with stable safe copy;
5. conflict/already-complete;
6. rate limit and retry metadata where supported;
7. server error with valid envelope;
8. non-JSON/truncated body;
9. redirect or wrong content type;
10. connection failure and deadline expiry; and
11. request accepted but response connection lost, followed by a status read.

Current account reads such as identity, root status, customer state, and account
status deserialize the success type without first checking status. A worker
error envelope can therefore become a generic JSON decode error in the account
page. This is tracked in [bug triage](../bug-triage.md#b-03-browser-account-reads-can-hide-service-errors-as-json-decoder-errors).

## CLI result contract

Every process-level test should record:

- exact argv, working directory, environment, and whether stdin/stdout/stderr
  are TTYs or pipes;
- exit code;
- stdout bytes and whether they form the promised JSON/notation/CSV/HTML/blob;
- stderr, including warnings and verbose chain;
- local registry/profile/session/site/branch/output state after process exit;
- remote/account state when touched; and
- a second invocation proving retry/idempotency or the documented block.

Signals must be injected at named readiness points, not after arbitrary sleeps.
Use a pipe/socket/barrier emitted immediately before the fault point. Process
tests should cover SIGINT for graceful cancellation and SIGTERM/forced exit for
restart durability where the platform permits.

## Account fault checkpoints

### Browser create

Inject failure or termination:

1. before Add-profile rotation;
2. after rotation, before WebAuthn;
3. during WebAuthn create;
4. after credential exists, before local root save;
5. after root save, before remote account request;
6. after remote account commit, before response;
7. after response, before provider attachment;
8. after attachment, before customer enrollment;
9. after enrollment, before custody queue/provision;
10. before dashboard settlement;
11. while activation is pending; and
12. after activation, before receipt/queued work is observed.

For every checkpoint assert credential count, selected profile DID, root DID,
provider attachment, remote account/device rows, customer status, custody
state, account-repository state, and safe next action.

### Browser login and same-account relogin

Inject before/after passkey assertion, remote device link, grant receipt, local
root/provider save, account hydration, customer enrollment, queued custody, and
dashboard render. Run with same root/device row, different root, revoked device,
deleted account, service timeout, and lost response.

### CLI handoff

Inject before/after callback bind, URL print, browser registration, callback
POST, callback receive, payload decode, grant validation, grant store, root
store, provider store, session activation, hydration, authority retention, and
push. Restart a new process at each checkpoint and assert one attachment at
most.

### Logout, revoke, and deletion

Inject before/after local session clear, provider detach send/accept; revocation
mint, each publication, account-fact removal; deletion plan load, arming,
passkey, request send, each owned-space result, account removal, local profile
rotation, and response reporting.

## Space and sync fault checkpoints

- create/adopt: site create/read, repository initialize, registry write,
  binding write, account ownership, hosting, upstream, first push;
- account link: validate local-only, retain authority, account directory,
  provider hosting, remote/upstream, sync;
- account pull/join: fetch/claim, site materialization, registry, binding,
  retained authority, remote setup;
- write: pre-pull, analyze/plan, local transaction, ref update, post-push;
- remove: confirmation, binding removal, registry removal, data deletion;
- invite: authority mint, repository push, URL construction, shortcut PUT;
- revoke: revocation mint, immutable publish to each service, recipient's next
  local and remote operations; and
- import/migration/update: each item/file, durable checkpoint, atomic publish,
  cleanup/rollback.

## Concurrency matrix

| Target | Actor A | Actor B | Required invariant |
| --- | --- | --- | --- |
| Browser account create | Submit Create. | Same tab double click or second tab same email. | At most one account and intended credential policy; both UIs settle unambiguously. |
| Account display name | Enter/blur rename. | Second device rename. | Defined winner/current fact; no stale UI claiming the loser. |
| Browser profile | Switch/delete current. | Other tab runs account action. | Old profile action cannot commit into new selected profile. |
| CLI account session | Login/logout/revoke. | Second process login/logout/sync. | Exactly one active generation; locks do not deadlock; stale work cannot reactivate. |
| Device | Revoke. | Device performs account/space request. | Defined ordering; every request after committed revocation is denied. |
| Space registry/site | Create/link/pull/rm. | Second process same name/subject. | One registration/site owner; no silent overwrite/orphan. |
| Branch | Local write/push. | Second local process or remote write. | Atomic local commit; explicit ahead/behind/diverged; no lost update. |
| Invite | Claim. | Second claim or revoke. | Defined claim/revoke ordering; no post-revocation authority resurrection. |
| Delete plan | Confirm delete. | Rename/delete/ownership change. | Stale plan rejected or exact subject preserved; scope never widens. |
| Service worker | Load old build. | Update activates. | One coherent asset generation; recoverable reload. |

### Service-worker generation recovery

A browser release is one sealed generation: the outer worker policy, worker
glue/Wasm, top document, lazy assets, sealed guests, guide, and Storybook share
one stamped identity. Installation verifies the complete graph in private
staging caches and publishes it only after every member succeeds. A failed or
interrupted candidate therefore leaves the incumbent generation and unrelated
browser storage intact.

An older document can continue read-only work through its incumbent worker.
Classified writes carrying a different valid page build are refused as a typed
`409 stale-build`; malformed or duplicate build headers are refused as a typed
`400 invalid-build-header`. The trusted top document presents the update
action, including when the signal originated in a nested guest. Once a
successor is installable, the retiring worker refuses new query or language
server streams with `503 {"control":"update-pending"}` so reconnects cannot pin
it indefinitely.

Claim and reload are serialized through `tonk-update-safety-v1`. An absent hold
is compatible with the pre-producer account flow, while malformed, unreadable,
future, or live holds fail closed. Recovery never unregisters a worker, clears
CacheStorage or IndexedDB, or resets credentials. Remote withdrawal likewise
terminalizes the named generation and offers update/reload while preserving
its caches and all local state.

## Recovery rules

1. **Read before replay.** When a request may have committed, fetch status or
   inspect local durable state before sending the mutation again.
2. **Use stable identity.** Retry by account generation, attachment ID,
   repository subject, operation ID, or branch head—not display name/email
   alone.
3. **Make idempotency explicit.** “Safe to rerun” requires a regression test
   that reruns after success and after lost acknowledgement.
4. **Do not roll back authority by deletion.** If remote authority committed
   but local persistence failed, recover or explicitly revoke/detach that exact
   generation.
5. **Preserve local-first boundaries.** Provider failure cannot erase or hide
   a local root, local-only space, retained local replica, or local commit.
6. **No silent partial success.** Enumerate per-stage or per-subject results.
7. **Fresh-process oracle.** The final assertion comes from a new page/process
   and, for shared facts, a second actor after sync.
8. **Bound waits.** Every callback/network/hydration/update wait has a deadline
   or user cancellation and reports the remaining durable state.
9. **Keep unrelated scope.** Fixtures always include a second account/profile,
   owned space, joined space, local-only space, and unrelated device when the
   operation could accidentally broaden.

## Open questions and verification

- Establish stable idempotency keys for account create, device registration,
  space link/hosting, revocation publication, and deletion operations.
- Establish one structured CLI error/result schema and exit taxonomy.
- Decide whether browser top-level navigation should be disabled during account
  transitions or whether every stage will be restart-reconciled.
- Decide the supported behavior for local writes after invite/device revocation.
- Build deterministic fault controls into the test services rather than relying
  on timing sleeps and arbitrary network throttling.
- Run the P1 matrix first; no failure/recovery claims here have been hand
  verified in the current worktree.

Source audit pinned to Tonk commit `a3f8670b1`.
