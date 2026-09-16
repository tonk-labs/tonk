# Plan 001: Give the CLI explicit access to spaces

> Executor: read this entire plan before editing. Implement the milestones in
> order, prove each with focused tests, and keep each increment reviewable.
> Update this plan and `plans/README.md` with fresh evidence.
> The protocol decision of 2026-09-16 supersedes the earlier redemption design
> and its authorizer-checkpoint proposal. Historical evidence is not a work queue.

## Status and intent

- Status: LOCAL IMPLEMENTATION VERIFIED, milestones 0–6. Milestone 7 release
  preparation is complete; authorized hosted release, propagation and published
  package/browser gates remain unrun. No deployment or publication occurred.
- Priority: P1. Effort: L. Risk: HIGH. Category: authorization / CLI / migration.
- Original baseline: `8acaa1897d3ed09a7bbde972f55060761d89f7f9`, 2026-09-15.
- Protocol revised by the user on 2026-09-16: ordinary space-scoped UCAN
  delegations, long-lived in both flows, with explicit expiry and standard
  revocation. Optimize initial UX and implementation for simplicity.
- First deliverable: a browser-generated agent invite carrying a fresh key pair
  and space authority, usable by the CLI without browser approval or CLI login.
- Second deliverable: CLI-initiated browser approval that delegates the user's
  selected spaces to a public key supplied by the CLI.
- Neither flow exports browser account/root private keys, installs account-wide
  CLI authority, transfers ownership, or implicitly selects a global CLI account.

The existing uncommitted `join --agent` implementation is a compatibility
baseline, not the target architecture. Preserve it and unrelated edits while
reconciling the new flow. `plan/cli-join-agent.md` describes that earlier work.

Before implementation, inspect instructions and the live worktree:

```sh
git status --short
git diff --stat 8acaa1897d3ed09a7bbde972f55060761d89f7f9..HEAD -- rust docs plans plan/cli-join-agent.md
git diff -- rust/tonk-cli rust/tonk-core/assets/library rust/tonk-ui rust/tonk-workspace docs/storybook
```

The baseline SHA does not describe the uncommitted starting point. Do not reset,
blanket-stage, or interpret historical tests as proof of the revised protocol.

## Accepted product contract

| Entry | Behavior | CLI authority |
| --- | --- | --- |
| `tonk connect AGENT_LINK` | Import the invitation identity and grants; mount, pull and confirm | Named space only, under the key carried by the invitation |
| `tonk --space NAME connect` | Resume an interrupted connection from saved local credentials | The same retained invitation identity and grants |
| `tonk link` | Generate a CLI key, open browser selection, receive grants | The explicitly selected spaces under the CLI's key |
| `tonk space` / `tonk status` | Report accessible spaces, selection, sync and known access status | No new authority |

Ordinary public `join` and CLI account login receive migration/deprecation only
after replacement and recovery gates pass. Browser accounts, passkeys, activation,
billing and ownership remain. `tonk space link` retains its existing local-space
ownership-adoption meaning; top-level `tonk link` is access approval.

### Flow A: browser-generated agent invite

1. A browser user in a space chooses **Connect an agent**. The user need not have
   a CLI installation, a CLI account, or any CLI spaces already configured.
2. Using valid existing authority for that space, the browser generates a fresh
   invitation key pair and issues ordinary scoped UCAN delegations to its public
   DID. Use a distinct invitation key and grant set for each independently
   revocable invite.
3. The copyable agent prompt contains a versioned link with the invitation's
   private-key material and delegation chain(s). A private seed may represent the
   key pair if the public key is deterministically reconstructed and checked
   against the delegation audience. No account/root private key is included.
4. The agent runs `tonk connect AGENT_LINK`. The CLI validates and securely saves
   the supplied identity and grants, mounts a replica with that authority, and
   pulls the space. The issuing browser may already be closed.
5. After successful synchronization and acknowledged confirmation, print
   **Agent connection confirmed**. Confirmation reports completed setup; it
   neither activates the grant nor proves continued presence.
6. Building is directed by the prompt and the user's request, not implicitly
   performed by `connect`.

The invitation is deliberately a reusable bearer credential. Another holder can
import the same key, exercise the same authority, or redelegate within its bounds.
There is no redemption call, one-time consumption, first-recipient binding, or
separate activation deadline. Direct use of valid invitation authority is expected,
including before any confirmation has been written.

Copied links represent the same invitation authority. The browser can show their
grant set and confirmations, but must not promise one exclusive physical agent.
Revoking the invitation grants affects every holder and descendant using them.
Separate invites have separate keys/grants so one can be revoked independently.

### Flow B: CLI-initiated browser linking

1. `tonk link` generates and retains a connection key pair on the CLI. It opens
   the trusted Tonk approval page with its public DID and correlated request
   information. The CLI's private key stays on the CLI.
2. The user signs into the browser if necessary and selects one, several, or all
   currently accessible spaces. Explain individually any space for which their
   authority cannot delegate the requested permissions.
3. The browser issues ordinary space-specific delegations to that exact CLI DID,
   using its valid authority for each selected space. Do not silently approve
   fewer spaces than the selection or substitute a stronger identity.
4. Deliver the grants and proof chains to the requesting CLI. It verifies subject,
   audience, permissions, expiry and signatures before installing the bundle.
5. Browser settings retain the issued grants and labels for management. Adding a
   space issues another delegation; removing access publishes standard UCAN
   revocation. The CLI need not be online when the browser makes these changes.

Approval requests can have short correlation/delivery timeouts. Those are not the
lifetime of issued space grants. Loopback, `--no-open`, and authenticated polling
may be used for delivery; none turns Flow A into an interactive approval flow.
“All spaces” is a snapshot, not a subscription to spaces acquired in the future.

### Lifetimes and UX

- Both invitation and terminal grants are long-lived with explicit absolute
  expiry. The exact default duration is still to be selected and documented in
  milestone 0; do not silently reinstate the former 10-minute redemption or
  24-hour agent limit, or impose a one-hour user-facing session.
- A child cannot outlive the effective authority of its ancestors. Issue from
  appropriate durable browser-side delegation authority, not an accidentally
  short-lived operator. Show the actual effective expiry. If the existing chain
  cannot support the intended lifetime, surface that limit without weakening
  verification or silently choosing another identity.
- Optional short-lived operator keys can rotate locally under the retained
  long-lived grant without another invite/browser ceremony. Rotation cannot
  extend the parent grant or evade revocation.
- Resume/restart uses retained credentials while valid. Expired or revoked grants
  require new authorization; never fall back to ambient account credentials.
  Advanced renewal and tighter agent-session controls are deferred.
- Access removal preserves downloaded replicas, aliases and unsynced edits.
  Revocation stops future authorized remote work, not possession of local copies.
- Separate authority contexts are application-level isolation, not an OS sandbox
  against processes running as the same Unix user.

## Protocol and authority

Use Dialog's existing UCAN delegation, invocation and revocation implementation.
A normal path is:

`space authority -> invitation key or CLI key -> optional operator key`

The browser's existing proof path may contain additional ancestors. Supplying a
public proof chain is not exporting those ancestors' private keys or giving the
CLI their standalone authority. The CLI must possess only its scoped recipient
key and any locally derived operator keys.

For each grant use the actual space subject, intended recipient DID, permitted
commands/argument policies and explicit expiry. Invocations identify the executor
and include the proof chain. Ordinary validation must establish signatures,
principal alignment, subject/command/policy attenuation, time bounds and valid
revocation status. Verify service routing using the existing trusted mechanism.

Do not add:

- Authority-bearing connection purpose markers in metadata.
- An “active connection” or “redeemed session” database consulted for data access.
- Atomic invitation consumption, recipient-race arbitration or redemption receipts.
- A connection-specific authorizer checkpoint.
- Broker custody of invitation/account signing keys.

Ordinary application state still exists: local credential storage and journals,
browser labels and issued-grant indexes, approval correlation, grant delivery and
the existing UCAN revocation store. None is a second source of data authority.

### Rights preset

Support the space data, schema, view, blob and sync operations needed for building,
without account administration, billing, ownership transfer or application
invitation/connection-management permissions. Ordinary UCAN redelegation remains
possible; “no invitation management” does not mean a bearer is non-transferable.

The prior source inspection suggested six exact leaf operations:

- Memory cell get/put, constrained to `space=branch/main, cell=revision`.
- Archive block get/put, constrained to `catalog=index`.
- Archive blob get/put.

This is a candidate, not a proven complete preset. Prove real command coverage
before adopting it. Use a bundle of ordinary delegations if exact command paths
require it; do not replace signed restrictions with a server-side marker allowlist
or use an unconstrained wildcard for convenience. Shared-space grants cannot
exceed the browser's upstream rights.

### Revocation and management

Retain issued delegation CIDs and use the existing authenticated UCAN revocation
path and ancestry-aware checker. Revoke the scoped grant(s), not shared browser
ancestors or unrelated sibling grants. Removing an entire terminal connection
means revoking all grants in that managed group, with truthful partial-failure
status; there is no global connection kill-switch row.

The browser list is a projection of issued delegations, their effective expiry,
confirmations and acknowledged revocations. Labels and confirmations do not
create authority. Mutating or deleting a local/list record cannot revoke a grant.
Re-adding access issues a fresh delegation identity; it never unrevokes an old CID.

For offline delivery of later additions, reuse an existing authenticated delivery
mechanism where possible. If a connection-specific mailbox/index is needed, it
carries signed grants and management information only. Authenticate readers,
restrict contents to that connection, and validate every received grant. Missing,
stale or forged delivery metadata must not expand authority or defeat revocation.
Do not require a manifest lookup to make an otherwise valid grant “active.”

Revocation is not stateless: executors must receive and check authorized UCAN
revocations. Measure the existing storage/propagation/caching behavior in the
deployed path before promising a cutoff. Improve that existing mechanism if
necessary rather than introducing a parallel connection-state gate.

### S3 transport lifetime, independent of connection setup

The earlier spike found that the pinned integrated authorizer issued one-hour
S3 URLs even for an ancestor expiring within 120 seconds. Address this independently:
bound issued descriptors by the remaining verified authority lifetime and a
documented short transport ceiling (60 seconds is a candidate, not a session
duration). Reject exhausted windows and preserve checksums/conditional headers.

A short URL lifetime does not shorten the long-lived grant: the CLI can request
new URLs using that grant while it remains valid. Already issued URLs can survive
revocation until their deadline. Report revocation propagation and URL lifetime
separately; do not claim immediate cutoff. Cover native and Cloudflare paths.

The broad checkpoint patch is not a prerequisite. If Dialog needs a change,
extract the minimal expiry-preserving signing API/fix and test it independently
with an explicit clock. Do not publish the previous patch unchanged as part of
this plan.

### Local persistence and secrets

| Record | Purpose | Authorization role |
| --- | --- | --- |
| Issued grant index | Public recipient, space, delegation CIDs, label, effective expiry | Management/discovery only |
| CLI credential store | Imported invitation secret or generated terminal secret; grants | Key possession plus validated UCAN chain |
| CLI binding/journal | Name/path, subject, credential reference, grant IDs, import checkpoints | Select authority; never grant it |
| Approval request/delivery | Exact CLI public key, request correlation, selection, bundle | Authenticate the linking ceremony |
| Confirmation | Grant/invite identity, completed sync evidence, optional local instance ID | Display only |
| UCAN revocation store | Authorized revocations of delegation CIDs | Existing standard revocation enforcement |

Keep secret-bearing invite delivery transient. Store the imported private key in
the credential store, not in resume metadata, replicated facts, clipboard history
managed by Tonk, telemetry, logs, grant indexes or confirmations. The link fragment
must not be sent to short-link servers or used to select an account-approval page.
Preserve existing credential-storage conventions; OS-keychain migration is outside
this task.

## Source anchors and scope

Inspect these live paths before changing them:

1. `rust/tonk-cli/src/bin/tonk.rs`, `handoff.rs`: existing agent flow is account
   bound (`expected_root`, `ensure_agent_account`, singleton confirmation).
2. `rust/tonk-invite/src/lib.rs`: existing `claim` redelegates a reusable open
   invitation; `visit` creates a short-lived child. Neither should silently
   define the new imported invitation identity or its long-lived grant lifetime.
3. `rust/tonk-worker/src/router/create_invite.rs` and `repository.rs`: current
   agent issuance targets an account; preserve transient secret-copy delivery.
4. `rust/tonk-cli/src/{site,identity,sync,remote,space,account_session}.rs`:
   account-root selection, registry and crash-safe local transitions. Add explicit
   scoped authority without globally disabling account checks.
5. `rust/tonk-cli/src/{account,space_link}.rs`: reuse trusted approval/correlation;
   keep ownership adoption distinct from top-level linking.
6. `rust/tonk-access-service/src/{handlers/ucan.rs,helpers/server.rs,revocation,revoke.rs}`:
   existing UCAN/provisioning gates, standard revocation, presign lifetime.
7. `rust/tonk-schema/src/{device_link,invitation}.rs`, `tonk-worker-api`:
   retained delegation labels, versioned contracts and confirmation identities.
8. `rust/tonk-core/assets/library/{core,onboarding-agent}.yaml`,
   `rust/tonk-workspace/src/ui_account_settings.{rs,html}`,
   `rust/tonk-ui/src/account_flow.rs`: prompts, management UI and E2E.
9. CLI guides/README, Storybook sources/generated indexes, this plan/index.

Scope includes the above and focused new connection modules/tests. New migrations
or delivery storage require a demonstrated need; there is no mandatory D1
connection/redemption schema or redemption-race harness. If storage does change,
use additive migrations and test the actual production adapter.

Follow `DESIGN.md` for UI and the local-first contract for offline data.
Out of scope: account/passkey/billing rewrites, ownership changes, ordinary web
invite redesign, OS isolation/keychain migration, presence/heartbeat, future-space
subscriptions, hosted-space creation by agents, single-use invites and automatic
deletion of retained credentials/data.

Keep proven increments reviewable. Commit only when requested and stage only
reviewed scope. Do not publish, deploy, push or open a PR solely because the plan
exists. The earlier publication question for the broad checkpoint is superseded.

## Milestones and verification gates

### 0. Reconcile baseline and finalize ordinary grant contracts

Inspect the dirty baseline and actual released CLI compatibility. Specify envelope
versions, exact rights, recipient/subject validation, trusted routing, error codes
and a long-lived default duration for both flows. Separate approval-request
timeouts and S3 URL lifetimes from grant lifetimes. Verify Dialog's implemented
UCAN version rather than assuming newer spec text is wire-compatible.

Rerun relevant baseline tests; record infrastructure failures separately. Existing
evidence below is historical. Replace/reclassify old tests deliberately: valid
bearer use before confirmation is now expected, not a bypass.

### 1. Prove ordinary scoped grants, revocation and transport expiry

Use browser-equivalent fixtures and the real native access-service boundary to
issue grants to (a) an invitation key and (b) a CLI-generated key. Perform real
read/write with only those grants, then publish authenticated standard UCAN
revocations and prove new requests fail. Prove ancestor expiry and sibling
isolation, preserving existing provisioning and ancestry checks.

For Flow A, two independent imports of the same bearer must both work while
valid; revoking the invitation grants denies both and their descendants. Distinct
invitations remain independent. Test optional operator rotation beneath each
long-lived identity without renewing its grants.

Adversarial cases: wrong subject, missing recipient key, forged/tampered proof,
command or argument escalation, expired ancestor, unauthorized revoker, stale
local management rows and revoked ancestor. No custom marker, redemption database
or successful confirmation may be needed to pass a valid request.

Prove the required build operations and narrow rights bundle. Independently bound
presigned URLs; test effective expiry, already-issued URLs, revocation propagation
and restart using explicit clocks/barriers where applicable, not sleeps. Extend
the existing UCAN/revocation path if needed; never infer hosted consistency from
an in-memory fixture.

**Verify:** focused invite and service `connection` tests select nonzero relevant
cases and pass. Check the Wasm service counterpart. Exit requires actual signed
remote reads/writes and ordinary revocation denial, not fixture-only policy state.

### 2. Import and resume invitation authority without CLI account state

Implement versioned invite parsing/import and explicit credential bindings. The
CLI imports the key in the invitation; it does not generate a replacement session
key and redeem it. Validate all grants and audience/key correspondence before
mounting. Save secret material and grants durably before remote work; use a local
journal for import, replica registration and final directory binding.

Resume by stored credential references and grant IDs without retaining the bearer
URL in the journal or invoking a browser. Repeated import of the same invite
should resume deterministically unless a separate local binding is explicitly
requested. Distinct authority contexts of the same subject initially use separate
replica storage to avoid silent privilege mixing.

Thread authority selection through every relevant data/sync/blob/schema/view
command. Never call account-login/attachment flows, recover ambient root authority,
stamp account membership, or silently switch via `TONK_SPACE` on this path.
Existing unrelated CLI accounts/spaces and unsynced changes remain intact.
Keep expired/revoked local replicas usable offline, with truthful remote errors.

**Verify:** add and register CLI `tests/connections.rs` (`autotests = false`).
Process tests cover empty CLI state, unrelated accounts, existing same-subject
replicas, crashes at import checkpoints, restart, repeated import, separate
invitations and no fallback after revoke. Existing handoff/join tests remain valid
for their explicit legacy formats.

### 3. Complete browser issuance and the agent journey

Issue a fresh invitation key and standard grant bundle for the current space.
Snapshot account/space during issuance; refuse if they change. Do not mint fresh
keys on every render. Retain only public grant-management data; copy secrets
through the transient boundary. Import must work after the browser closes.

New prompts use `tonk connect AGENT_LINK`, with no CLI account-login or approval
instructions. Preserve playground page/build scope. Confirmation is keyed by
invitation/grant set rather than the singleton `id:tonk:agent-connection`; multiple
holders of one invite must not be portrayed as one exclusive process.

List agent invites/access in existing settings/share surfaces with labels, scope,
actual expiry, confirmation evidence and revoke action. Revocation uses the
existing authenticated UCAN mechanism for every grant in the set. Show pending,
partial failure or confirmed delivery honestly; do not treat a UI mutation as
remote enforcement.

**Verify:** worker prompt contracts plus browser `connection` E2E:
copy -> close issuer browser -> real CLI with no account login -> pull/build/push
-> confirmation -> standard revoke -> denied new remote request.
Cover two separate invites, two holders of one invite, unrelated CLI account,
secret non-disclosure, desktop/mobile layout, keyboard focus and reduced motion.
Record clipboard/screenshots separately from source tests.

### 4. Implement CLI-initiated selected-space linking

Persist a CLI-owned key and correlated approval request, open trusted browser
selection, and deliver the exact approved bundle to that key. Borrow the existing
loopback/no-open/polling machinery where appropriate. Bind approvals to the
request and recipient, handle cancellation/timeout/replay, and never send the
CLI private key to the browser or return an account-wide delegation.

Support one/many/all-current selections, including shared spaces within upstream
rights. Stage the complete selected bundle before marking initial approval
successful. A failed/cancelled flow preserves the previous local configuration.

**Verify:** actual CLI process/browser tests for the selections, wrong callback
key, replay, cancellation, timeout, browser sign-in and remote/headless delivery.
Prove unselected spaces and account operations remain unauthorized.

### 5. Manage grants and display access truthfully

Retain grant groups for terminal/agent management. Add space access by issuing
new grants to the existing terminal public key. Remove access by standard
revocation; revoke-all tracks every grant and reports partial failures. Re-add
with fresh delegation CIDs. Authenticate delivery of additions while the CLI is
offline without exposing the account catalogue or creating a data-access ACL.

Show local-only, invite-backed, terminal-linked and legacy bindings accurately
in `space`/`status`; retain versioned camelCase JSON and shared rows conventions.
Offline cached status is not proof of current remote access. Alias collisions
cannot replace another connection's authority, rename a space, or delete data.
Separate browser accounts may manage independent grant groups.

**Verify:** offline additions/removals, stale/forged delivery metadata, replay,
fresh grant re-addition, partial revoke-all, independent grant groups, retained
offline work and no ambient authority fallback. If persistence/delivery adapters
change, test their real SQLite/D1/other backend rather than asserting consistency
from a mock. No single-use redemption tests are required.

### 6. Migrate public commands and existing installations

Inventory account commands before hiding them. Distinguish ordinary scoped access
from administrative workflows: local-space ownership adoption, backup, account
deletion, recovery and hosting/provisioning need executable compatibility paths
or tested browser destinations.

Convert existing account-linked CLIs only through explicit browser selection to a
fresh CLI key. Install and verify the selected scoped grants before deactivating
the local account-wide attachment. Retain old credentials/data for recovery;
remote device revocation is a separate explicit action, not implicit cleanup.
Never revoke a credential used by another client.

Preserve local-only spaces, aliases, directory bindings, subjects, branch heads,
ownership and unsynced edits. Failed conversion leaves the previous mode usable.
Expose `connect` and `link`; retain any shipped `join --agent` spelling as explicit
compatibility. Legacy account-scoped handoffs keep their behavior or a precise
upgrade path, never silent reinterpretation as new bearer invites. Deprecate
ordinary `join`/`account` rather than abruptly removing recovery workflows.

**Verify:** migration fixtures for old registries, partial handoffs, accountless
replicas, inactive accounts, duplicate subjects, cancellation and success. Compare
files/heads/local changes before and after. Run the full CLI suite at this checkpoint.

### 7. Release compatibly and record operational evidence

Ship any required standard-revocation/transport-expiry fixes first, then a CLI
understanding old/new envelopes, then switch browser issuance/prompts after
verifying the published CLI. No custom connection-gate capability advertisement
is required; verify ordinary protocol/envelope compatibility and enforcement.
Update existing seeded prompts using established reconciliation/provenance without
overwriting authored views or home state.

Rollback must preserve standard grant/revocation behavior and the documented URL
bound. Keep legacy compatibility until explicitly retired. Record local, CI,
staging, npm and production evidence separately. Release/deployment and external
credential revocation still require the operator's authorization.

**Revocation index compatibility audit (local source, 2026-09-16):**

- Before the authority-subject correction, `tonk-identity::revocation::verify`
  returned the invocation signer as `VerifiedRevocation.subject`, and
  `tonk-access-service/src/revoke.rs` recorded that DID. The pinned Dialog
  `4c16de9` verifier checks each target against its ancestor issuers plus its
  immediate audience. A signer on a different branch therefore produced an
  ineffective fact even when the service acknowledged it.
- Definite legacy callers are browser
  `router/account_devices.rs::delegated_revocation` and CLI
  `account.rs::revoke_in_observed` when another account device withdraws a
  `Subject::Any` account-to-device grant. The pinned revocation verifier accepts
  that target shape without a witness walk; the current invocation still proves
  account authority, but the old index stored the sibling signer instead of the
  account. This is a source-established possibility, not evidence that any hosted
  deployment contains affected records. The existing CLI delegated-revocation
  test checks that listing returns promptly, not that remote access is denied.
- Own-device `mint_self_revocation` records used the target's immediate audience,
  so the old key remains effective. Root-signed ceremony/rotation withdrawals
  have identical signer and authority and are unchanged. Legacy invitation and
  member removal use `revoke_invite.rs::publish_revocation`: an issuing/ancestor
  signer on the target path already yields an effective old key. An off-path
  signer on a subject-specific target normally fails the old helper's witness
  check before acknowledgement; do not count such refusals as stale records.
- No automatic repair is present. Both browser and CLI device removal retract
  device-list rows after successful publication. The CLI explicitly returns
  `AlreadyRevoked` without POST when the row is absent, even if its public grant
  remains retained. The browser endpoint can rebuild a retained device grant and
  publish again when explicitly addressed, but the removed row is no longer a
  normal UI retry. Invitation/member removal also retracts its leaf/management
  rows; no background replay queue exists. Artifacts are not retained by these
  publishers for automatic retransmission.
- Milestone 6 action: an explicit legacy device-revocation retry should republish
  a recoverable retained grant rather than unconditionally return
  `AlreadyRevoked`. Preserve the user's explicit target; do not silently perform
  mass revocation or infer withdrawal intent for other retained grants.
- KV stores only empty `revoked/{target CID}/{DID}` facts, not signed artifacts,
  proofs, or the authenticated authority. It cannot safely infer replacement
  account keys. Do not migrate or reinterpret old DID keys. Any operational
  repair needs the original signed evidence or a newly authorized withdrawal
  with retained public target proofs. Reposting a still-valid old artifact to
  the corrected service can add the authority key; it may return `recorded:true`
  despite the older signer key. The current-prf gate still applies to this new
  fact, so withdrawn caller authority cannot repair it by itself.
- Release the service fix before enabling the new browser flow. Its wire shape
  is unchanged, but receipt `subject` now means authenticated authority as
  documented. Existing legacy publishers parse receipts without comparing that
  field. Existing correctly scoped KV facts remain readable without migration.
  Rolling back the service writer reintroduces ineffective sibling-device
  revocations and the revoked-prf recording defect; retain these fixes in any
  rollback; rollback to the broken subject-index verifier is not acceptable.
  Do not delete old or corrected facts. Milestone 7 requires an authorized
  operator inventory and evidence-backed republication where recoverable.
  Hosted prevalence, repair, and cross-region propagation remain unmeasured and
  unrun operator gates.

**Verify:** exact published CLI/browser staging matrix, both journeys, long-lived
grants through restart/operator rotation, revocation propagation and stale URL
behavior. Name unrun environments and artifact identities.

## Commands and test conventions

Run from the repository root. Add new named tests before invoking their filters;
zero-test success is not evidence. Old checkpoint characterization tests do not
satisfy the revised protocol gates.

| Purpose | Command |
| --- | --- |
| Parser baseline | `cargo test -p tonk-cli --bin tonk account_spaces_parser_tests -- --test-threads=1` |
| Legacy handoff/join | `cargo test -p tonk-cli --test handoff --test join_profile -- --test-threads=1` |
| Legacy account constraints | `cargo test -p tonk-cli --lib handoff -- --test-threads=1` |
| New CLI connections | `cargo test -p tonk-cli --test connections -- --test-threads=1` |
| Ordinary service grants/revocation | `cargo test -p tonk-access-service --features helpers connection` |
| Invitation contracts | `cargo test -p tonk-invite` |
| Prompt contract | `cargo test -p tonk-worker --test standard_library` |
| Worker targets | `cargo check -p tonk-worker -p tonk-access-service --target wasm32-unknown-unknown` |
| CLI integration checkpoint | `cargo test -p tonk-cli -- --test-threads=1` |
| Browser journeys | `nix develop . -c test:e2e -E 'test(connection)'` |
| Wasm tests | `nix develop . -c test:web:debug` |
| Formatting | `cargo fmt --all -- --check` |
| Repository lint | `nix --accept-flake-config develop .#ci -c lint` |
| Storybook generation | `python3 docs/storybook/scripts/build.py` |
| Storybook freshness/links | `python3 docs/storybook/scripts/build.py --check`; `python3 docs/storybook/scripts/check-links.py docs/storybook` |
| Scope/whitespace | `git diff --check`; `git diff --stat` |

Register CLI integration binaries explicitly. Use isolated CLI homes and the
actual access-service boundary. Run focused tests per increment; broad suites at
integration checkpoints. New storage requires its real adapter/harness checks,
not an unconditional new D1 redemption subsystem.

## Done criteria

- [x] Both flows use ordinary scoped UCAN grants and standard revocation.
- [x] Agent link contains a fresh invitation key pair and grant bundle; CLI import
      works with the browser closed, no login, and unrelated CLI state present.
- [x] Multiple holders of the same invite work as intended and lose derived
      access when its grants are revoked; separate invites remain independent.
- [x] CLI linking keeps its private key local and grants exactly the selection.
- [x] Both flows use documented long-lived expiry bounded by upstream authority;
      internal operator rotation needs no new invitation.
- [x] All build/sync/blob operations use the chosen scoped authority without
      account fallback, management escalation or unselected-space access.
- [x] Local standard revocation and S3 lifetime bounds have real evidence.
- [ ] Hosted revocation propagation and the published CLI/browser release matrix
      have operational evidence; see the explicit external release gates.
- [x] Grant management and optional delivery metadata do not authorize data access.
- [x] Confirmation reports completed setup, not activation or exclusive presence.
- [x] Migration preserves data/ownership/credentials and has tested failure paths.
- [x] Help, prompts, JSON and Storybook describe both flows and compatibility.
- [x] Required native/Wasm/browser and changed storage-adapter checks are recorded.
- [x] Release/rollback preserve ordinary authority; unrun external gates are named.

## Stop and resolve

- A new path needs account/root private keys or falls back to broader authority.
- A scoped grant can authorize another subject or forbidden commands/arguments.
- The browser cannot delegate a requested space or the intended long lifetime.
- Standard revocation is not enforced on some transport, or its propagation/URL
  bounds cannot support the promised UX. Resolve within the existing mechanism.
- A design reintroduces single-use invites, custom marker enforcement or an
  authoritative active-session record. That is a new product/protocol decision.
- A migration deletes local work, changes ownership or revokes shared credentials.
- Shipped-client/prompt compatibility is not understood before switching issuance.

## Current execution (2026-09-16)

The user requested agent orchestration of this revised plan. Work is split into
invitation contracts, native service grants/revocation, and transport expiry;
the coordinating agent owns CLI build-coverage checks and integration evidence.
Milestones remain sequential. No commits, publication or deployments are implied.

- Preserved starting point: HEAD `8acaa1897d3ed09a7bbde972f55060761d89f7f9`,
  14 tracked dirty files (251 additions, 106 deletions), plus the plans and
  historical untracked spike artifacts. The baseline-to-HEAD comparison is empty.
- Fresh baseline: parser/help 37 passed; handoff 11 passed; join-profile 1 passed;
  library handoff/account constraints 6 passed, using the commands above.
- Public npm metadata reports `@tonk/cli` version `0.6.15`, tarball
  `https://registry.npmjs.org/@tonk/cli/-/cli-0.6.15.tgz`, integrity
  `sha512-+wYGdcTn0GLSoYeaaElut7vb676LCSj5deKINbkrYCACPt85vuctrcH5KAggfxX1j6ME83OJCMK1hJVaQ7Zpjw==`.
  The initial sandbox lookup failed `ENOTFOUND`; the unchanged network-enabled
  retry succeeded. Downloaded `@tonk/cli-darwin-arm64@0.6.15` without package
  scripts, integrity
  `sha512-4MwOOU9iPJEByYEkJhmaPg1MX8NsVCQas+BxzmCsfVJVA8TzCFqaqWHS82S+gW2gUhp557U/SiFbwYFu1OjfOw==`.
  Its actual executable reports `tonk 0.6.15`; `connect --help` exposes the
  account-approval options and `join --help` requires URL/name without `--agent`.
  Version/help ran with telemetry disabled and isolated registry state. A real
  compatibility process test also passed: the released executable's default
  profile DID was verified and held broader authority for the same subject,
  but `push` refused the scoped outer layout (`failed to load repository 'main'`)
  and retained the new replica's offline edits unchanged.
  Local tag source agrees and retains account login. Other platforms and real
  browser-to-published-CLI journeys remain unverified.
- Current CLI remote setup and sync also transfer `meta`. The proposed six
  grants allow only the `main` revision cell. The registered CLI `connections`
  test now passes actual schema/data/view authoring, signed push/pull and blob
  upload/readback with only those target-space grants and unrelated local
  authority present. Adopt the six-leaf preset for main-only connections; do not
  transfer `meta` or broaden account/management permissions on this path.
  This is library execution through the real native service; invitation process
  import/resume remains milestone 2.
- Default requested grant lifetime selected for the additive contract: 90 days,
  subject to every ancestor. A shorter available lifetime is reported explicitly.
  Approval correlation and the 60-second S3 transport ceiling are separate.
- The pinned authorizer exposes only an already-signed permit. A single-crate
  local vendor patch is integrated to bound signed transport expiry by
  verified authority and 60 seconds without importing the old checkpoint API.
  Root patch configuration preserves the existing Dialog source identity and
  registry versions. Upstream 37 tests, native/Wasm service checks, and the
  signed S3 write/readback consumer test pass. Patch/vendor reproduction passes.
- Ordinary service connections: 3 tests pass, covering all six signed S3 leaf
  operations; argument/command/subject/key/proof/expiry rejection; authenticated
  standard revocation of both bearer holders and derived operators; independent
  invitation/CLI grants; and continued use of already issued bounded URLs.
  Native listeners required the unchanged test retry outside the socket sandbox.
- A local production-Wasm test passes authenticated revocation and restart with
  persistent KV/D1 storage. It applies existing migrations and adds no connection
  database. Global Cloudflare KV propagation is not established by this emulator.
  Its cache behavior and S3 URL lifetime must be reported separately.
- Final invite checks: 40 native tests and Wasm library check pass. These include
  the actual Profile/Operator issuance path: six 90-day grants omit the one-hour
  operator suffix, respect upstream expiry, and refuse write issuance from
  read-only shared authority. Root/browser-key export and unsafe carrier URLs
  are rejected. See [the contract](001-grant-contract.md).
- Milestone 2 is split across scoped credential/site loading, atomic registry
  transitions, and command/confirmation wiring. All existing legacy paths remain.
  [Import implementation notes](001-cli-import-notes.md) identify the local APIs
  and compatibility layout; they are design notes, not completion evidence.
- Milestone 2 process checks: four `connection_commands` tests passed with
  `--features integration-tests`. These run signed invitation imports against
  the native service, restart without the link, preserve unrelated cached account
  state, and verify grant-set-specific confirmation. Actual process interruption
  after Credentials/Mounted and Ready-before-registration resumes from the saved
  credentials and original directory. Three trusted-discovery tests pass.
- Registry checks: 52 focused tests passed, including concurrent mutations and
  the create/account initialization lock-order regression. Review found and
  corrected missing exact `origin/main` tracking validation and directory-parent
  fsync. All six importer tests now pass, and five focused registry tests pass
  including exact old-writer field repair and rejection of mismatched/unmarked
  sites. The existing CLI-space 59, handoff 11 and join-profile 1 tests pass after
  integration; `cargo fmt --all -- --check` and `git diff --check` pass.
  The account-authority integration suite passes all 12 tests. The final four
  connection process tests also pass after six authenticated revocations, with
  a broader same-subject default-profile grant verified through CLI `identity`:
  new pull/push/connect fail, offline edits remain, and no account fallback or
  misleading resume/success advice appears. Expired retained credentials reopen
  offline and reject remote sync. Final Wasm, invalid-path preflight and typed
  expired-denial checks pass. Milestone 2's local gate is closed.
- Milestone 3 is split across profile-backed public grant management and the
  ordinary six-leaf issuer, transient prompts/confirmation, real browser/CLI
  tests, and the settings surface. New issuance is opt-in until a compatible CLI
  is published. The starting dirty legacy prompt already uses `join --agent`,
  which the inspected published 0.6.15 binary does not support; preserving that
  baseline is not release-compatibility evidence. Reconcile its command spelling
  with published clients before release as part of milestones 6–7.
- Milestone 3 first browser slice passes: signup and a real remote space,
  visible copied v1 prompt, issuer browser closed, accountless CLI import,
  schema edit and URL-free resume with acknowledged synchronization. This used
  the first opt-in artifact; final full journey/revoke/layout checks remain.
  The existing Welcome smoke passed playground opening/reload and its scoped
  page instructions, then failed a later offline navigation among the eight
  bundled pages, reproduced twice with the new artifact. The old runtime passes
  that same smoke after adapting only its expected legacy command spelling.
  A diagnostics-only rerun of the identical first new artifact later passed
  (20.69 seconds), without invoking the failure handler; the failure is
  intermittent and its cause remains under investigation. A prior
  `web-integration-tests` invocation selected the
  wrong Wasm harness; the corrected native invocation uses `integration-tests`.
- The second-browser revocation gate found two ordinary-protocol integration
  defects: its own device grant must be present in the standard witness pool,
  and the index must record the verified invocation's authority subject instead
  of its off-path signing device. The additive witness helper and verifier fix
  now pass real native denial for both browser groups with sibling isolation,
  24 identity revocation tests and 8 service revoke tests. Production-Wasm
  direct/delegated revocation plus KV/D1 restart passes after correcting the
  fixture's CID byte-array comparison; siblings remain usable and replay returns
  `recorded: false`. A further native regression demonstrates that a revoked
  browser-device proof still permitted a new delegated revocation (HTTP 200).
  Current invocation-proof screening now passes the focused native regression
  and all 8 revoke tests: revoked current proof refused (401), historical revoked
  witness with live authority accepted, prior receipt replay idempotent, and
  original issuer withdrawal still accepted. The final production-Wasm rebuild
  also passes both revocation modes, persisted KV/D1 restart, sibling isolation
  and receipt replay (3.23 seconds). The browser ledger's 3 focused tests pass
  with same-account/same-subject lookup through a persisted reopen.
- Browser management persistence passes partial-delivery/restart/retry tests,
  including a retained request with zero acknowledgements. The full standard
  library passes 33 tests. Browser remounts are being made to reuse the transient
  invite; an explicit new-invite action generates a separate grant group.
- The first opt-in artifact also passes the complete one-group browser slice
  (17.75 seconds): reopen the issuing profile, pull the exact confirmation, click
  Settings revoke, receive six acknowledgements, close the browser, then observe
  a new CLI process refused by the service while offline schema editing works.
  Initial desktop/narrow/dark screenshots were inspected separately. True mobile
  viewport, keyboard focus, two holders and sibling-invite isolation are still
  pending on the final artifact; the initial narrow window was 500px wide.
- Final opt-in artifact: `967bd0b4c66d8df1`, worker
  `d96c72c7e5fe6d1e`; the final prompt/fallback standard-library suite passes
  33 tests. Two-holder, reload and final layout journeys are running against
  this artifact. A separate smoke retry failed TLS before the app loaded
  (`ERR_SSL_PROTOCOL_ERROR`), which supplies no evidence about offline routing.
- The final transient-cache test passes concurrent reuse, refusal after overlay
  loss, rejection of a replacement link and unchanged durable content; generated
  onboarding matches its source. Storybook now tracks scoped invitations as
  `ACCT-C14` / `HANDOFF-21`, separately from legacy `ACCT-C13`. Generation,
  freshness and 178 local links pass; final browser evidence remains pending.
- Final-artifact Welcome/playground/offline smoke passes (22.61 seconds) with
  its original assertions; the earlier intermittent failure is retained above.
  The two-invite clipboard timeout was traced to clicking the second copy while
  WebAwesome still had `isCopying=true` from the first success feedback. The
  fixture is being changed to wait for the real control's ready state; clipboard
  contents and exact invitation comparisons remain required. No product fix is
  inferred from that test timing failure.
- Artifact `967bd0b4c66d8df1` passes the complete two-holder journey (21.43s):
  issuer closed, same-invite holders under empty/unrelated CLI accounts, schema,
  data, view and blob readback, independent invite, restart without bearer
  retention or implicit issuance, both confirmations, both holders denied after
  Settings revocation, sibling still syncing, and unchanged unrelated account.
  Chrome/CLI ordinary-invite redaction assertions pass. The one-holder revoke
  journey separately passes (17.26s), including result focus and offline edits.
- The optional keyboard gate exposed a pre-existing account menu handler that
  dismissed Settings on Tab. Native browser diagnostics showed the real Refresh
  keydown, then `settingsHidden=true` and focus returning to the account header.
  A new focused Wasm DOM test reproduced that failure, then passed with the
  one-line Settings guard; the existing menu keyboard test also passes. A fresh
  artifact is rebuilding for final two-holder/layout verification after this fix.
- Milestone 3 local gate is closed: final guarded artifact
  `b32834be7306c317` (worker `d96c72c7e5fe6d1e`, guest
  `0f376527840324b9`, manifest
  `8bdf4d6528b531352b43b464ec6b9eca3867a020d0a04e1723061dbf0c80d05a`)
  passes the full two-holder journey with layout enabled (23.55s). Desktop and
  true 390px tall/short dark screenshots were visually inspected. Real Tab moves
  from Refresh to a visible 44px revoke target; focus ring, reduced motion and
  no inner/outer horizontal overflow pass. Final source formatting, Storybook
  freshness/links and diff checks pass. The intermittent earlier Welcome failure
  and all unrun external/CI/Safari gates remain explicitly recorded.
- Milestone 4 starts with a signed request/complete approval and one-space
  installation slice: CLI codec/key/journal/import, real SQLite/D1 delivery,
  selected-space worker issuance, and browser selection UI. The
  [delivery draft](001-terminal-delivery-notes.md) describes the intended trust
  boundaries. No pending anonymous server writes or data-access activation row
  is planned; implementation and verification remain in progress.

- Milestone 4 intermediate gates pass: signed request/approval codec; CLI private
  key, durable interrupted resume and all-or-nothing registry publication (four
  tests); browser native-checkbox selection (one Wasm DOM test); worker exact
  recipient, real selected-space publication/retry/decline and no-redirect tests
  (three); immutable SQLite restart/race/quota (three); actual HTTP delivery
  (one); production Wasm/D1 restart/recipient/revoked-approval checks (one).
  The actual CLI/browser one/many/all-current test compiles and awaits the
  stable executable/browser artifact. This is not yet the whole milestone gate.
- Milestone 5 additive authenticated offline-delivery and management slices are
  being implemented by the service, worker and CLI owners. Browser management
  lists public terminal groups and exposes add/remove/revoke-all; checks pending.
  First browser artifact attempt failed during a concurrent worker rebuild,
  without a Trunk compiler diagnostic; a stable source checkpoint is required.
- Milestone 6 targeted retry regression reproduces the legacy missing-row
  shortcut: an explicit retained device grant returned AlreadyRevoked without
  publication. The shortcut is removed; the same real-service regression is
  rerunning. No external credentials were touched.

- The explicit retained-device retry now passes against the same real service
  (one test, 1.75s), including a second receipt showing the exact authority fact
  was already recorded. Default copied prompts now use `connect`; the downloaded
  CLI 0.6.15 help was freshly checked for `connect` and `--switch-account`.
  Two focused scoped-prompt/receipt tests pass; full prompt suite is pending.
- Browser integration preparation found missing `/connection/*` routing in
  Cloudflare assets, the Nix development/test proxies and the performance proxy.
  Those routes now reach the access service. The cached Caddy wrapper used for
  the local test was updated equivalently in a temporary script.
- Browser artifact attempts remain unsuccessful: one API field mismatch during
  concurrent source edits was followed by a concrete KnownCustody missing
  Clone/Debug compile error in the new internal-repository guard. These are not
  claimed as passing browser evidence. Worker correction and rebuild are pending.
  The superseded overlapping Trunk build was stopped before it could race the
  replacement artifact output; unrelated compilers were left running.
- Release ordering, compatibility matrix and rollback constraints are recorded in
  [the release gate checklist](001-release-gates.md). No deployment, publication,
  external credential withdrawal or hosted inventory has been performed.

- Current intermediate checks now include the final terminal selection DOM and
  pending-addition/partial-revocation DOM tests (one each), six CLI terminal core
  tests, and the exact-attachment conversion helper (one). Production Wasm/D1
  small offline-addition delivery passes (3.45s): active wrong-account refusal,
  recipient-only reads, restart, current issuer revocation and idempotent replay.
- Exact account-catalogue subject grants were reproduced as accepted by both
  terminal codecs, then explicitly rejected with a passing focused codec test.
  The worker additionally refuses known account/profile/custody/ledger identities.
  Its new query type needed both derives and public visibility; corrected native
  worker check passes, and the first complete browser artifact is rebuilding.
- D1's published row limit cannot hold the protocol's largest complete payload
  in one row. The service owner is implementing atomic payload chunks and an
  above-row-limit production-adapter fixture; the four-MiB complete envelope
  bound remains. Account deletion cleanup will remove service mailbox/chunk
  copies while preserving standard revocations and other accounts.

- Final service storage gates now pass: six real SQLite storage tests, four
  ordinary-grant HTTP tests, one signed-delivery HTTP test, seven account-deletion
  regressions, and the production Worker/D1 large-payload harness. The harness
  delivers 334 selected spaces in signed initial/addition payloads over 2.1 MB,
  verifies atomic chunks, exact restart/replay, injected rollback, account purge,
  other-account isolation and retained revocation facts. A late publication
  after purge first reproduced a recreated mailbox; the atomic INSERT now also
  requires the customer to remain Active. Strict service Clippy passed.
- Browser build `1365cb24735d657d` completed (worker `66af191ab60baf2b`, manifest
  `fe1946788da74c584709b10ca658d7885febb829a40db6a69c6ba85a88012d0a`).
  It includes a browser-reproduced terminal management hit-area correction from
  13 px to the existing 44 px management control rule. Whole browser gates are
  still running. Fixture corrections include awaiting remote attachment,
  matching complete success text, using the actual signed-out account ceremony,
  and explicitly disabling eval auto-sync before testing revoked writes.
- CLI recovery now verifies exact already-published selections after an
  interrupted journal checkpoint, retains staged data for expired additions,
  and records explicit rejected delivery outcomes before accepting a fresh grant.
  Nine core tests, three typed expiry/revocation classification tests, ten
  inventory tests, one conversion helper and 38 parser/help tests pass. Final
  whole CLI regression and strict workspace lint are running; these focused
  results do not establish their completion.

- Full invite verification passes 46 tests and the complete worker standard
  library target passes 33. The final CLI focused gate passes 11 terminal recovery
  tests, including management-proof expiry distinct from still-valid shared-space
  grants, plus all 63 CLI space/help/provenance tests. The first full CLI checkpoint
  found three stale hidden-command/access-column assertions; these were updated
  to the intended public compatibility behavior and the full suite is rerunning.
- Actual explicit browser conversion passes (10.68s): fresh selected key/grants,
  exact retained legacy files/bindings, inactive account attachment, preserved
  unsynced work, refused legacy push and successful scoped push. Fresh browser
  signup followed by explicit decline passes (8.02s), preserving prior CLI state.
- Actual browser management passes (31.70s): offline addition, removed-grant
  refusal with an unsynced commit, independent committed push, a queued re-add
  revoked before CLI import, explicit rejected-delivery receipt/cursor, newest
  fresh CIDs and separate alias, exact retained local note query and 24/24
  revoke-all acknowledgements. Broad schema enumeration may need uncached remote
  indexes; the retained local note was verified through its exact local query.
  Desktop/narrow/short-dark captures pass overflow, real keyboard focus and
  44 px controls. Top-document reduced-motion emulation does not reach the opaque
  guest's media query; no guest reduced-motion preference assertion is claimed.
- One/many/all browser testing exposed a second-request navigation defect: a new
  fragment on the same Settings pathname retained the first completed request's
  guest context. The terminal route now remounts its approval view on hash change;
  its final artifact and complete selection rerun are pending. Earlier intermittent
  unconfirmed-delivery failures remain recorded; they are not attributed to this
  navigation defect without matching diagnostics.


- Final CLI integration checkpoint passes **628 tests, 0 failures, 2 ignored**
  across 33 targets. The released CLI 0.6.15 compatibility test was then run
  separately with its required executable and passed (1 test, 0.64s). The other
  ignored test is the pre-existing post-#447 schema-introspection analyzer port;
  it remains unrun. The prior ownership-list failure was a stale expectation
  that the now-required ACCESS column be absent, corrected with explicit legacy
  and local-only provenance assertions. All eight ownership-link tests pass.
- Strict workspace Clippy with all targets/features and `-D warnings` passes
  (8.17s); final test-only diagnostic edits will receive a final check. This is
  the Rust lint command, not a claim that full `nix flake check` or CI ran.
- Navigation artifact `63775cbb5273ce15` (worker `d9de3fc28f1b5483`, manifest
  `4b560a0893107c2a90e758bc8e1bf7e43454a01ff75d135120b82e214b34a493`)
  passes actual one/many/all-current CLI/browser selections (29.24s), including
  repeated requests on the same pathname with different fragments. Exact
  conversion passes again (39.84s), and full offline management with the final
  CLI passes (26.50s). The retained final CLI is a local artifact, not a release.
- Subsequent decline checks pass (6.11s, 6.22s, 9.48s). To investigate the earlier
  intermittent pre-staging refusal, test diagnostics now observe the original
  guest fetch status/public error instead of retrying the approval or querying
  candidates before clicking. With that observation, fresh-sign-in decline
  passes (20.61s) and complete management passes (37.02s). These passes do not
  explain the earlier failure. The worker's full candidate DTO fingerprint can
  change with display names or transient eligibility text; a deterministic
  regression is investigating this separate, concrete refusal mechanism.
- Nine terminal selection/management screenshots and their exact provenance are
  retained in `docs/storybook/capture/terminal-connections-2026-09-16/`.
  Updated ACCT-C15/HANDOFF-22 records distinguish tested local journeys from
  unresolved findings and external gates; generation and 198 local links pass.

- The old fingerprint now has a deterministic failing regression: an unchanged
  account proof and identical repository/subject membership received a different
  snapshot after display/availability fields changed. The corrected v2
  fingerprint covers only that proof and the sorted repository/subject identities.
  The exact selected subjects still undergo current remote and grant validation
  before staging. Account/proof changes, membership changes and subject/repository
  substitution still invalidate review. This proves and corrects a refusal
  mechanism; it does not retrospectively identify the earlier browser response.
  Final focused worker, artifact/browser and strict lint checks are running.

- Final snapshot worker gate passes 4 tests (7.21s): presentation-stable identity
  fingerprint, recipient/rights enforcement, complete selected issuer/management
  fixture and redirect refusal. The initial socket-sandbox attempt passed the
  two pure checks and failed two listener tests with `Operation not permitted`;
  the unchanged gate passed with local-listener access. Strict workspace Clippy
  passes after the final production/test edits (2m20s), as do formatting and
  whitespace checks.
- Final Wasm artifact builds successfully: `d7f3967a6db1fa1a`, worker
  `4ebdb9f0b3ee3f13`, manifest
  `e96677dc59ffedfad02552587c6f5aea28e75abaaa18defcba49786359f04c00`.
  This artifact adds the snapshot correction to the previously verified
  navigation and management UI; actual browser selection/decline reruns started.

- **Final local browser checkpoint passes on artifact `d7f3967a6db1fa1a`:**
  one/many/all-current selections with repeated fragment navigation (26.97s),
  fresh browser sign-in and decline (8.83s), complete offline additions/revocations
  including a revoked queued delivery and fresh re-add (36.98s), and explicit
  legacy-account conversion preserving exact local edits/bindings (13.95s).
  All use the final local CLI. Selection and management again exercise desktop,
  narrow and short dark layouts, focus, hit area and overflow checks. Historical
  screenshots retain their original artifact provenance in Storybook.
- The earlier intermittent pre-staging refusal was not captured with its original
  response, so its precise cause is not retrospectively established. The
  deterministic stale-snapshot defect is fixed and the final affected journeys
  pass. Failure-only original-response diagnostics remain in the test fixture;
  monitor this boundary in staging instead of claiming every historical failure
  was traced. This is a qualification of the evidence, not a remaining reproduced
  failure on the final artifact.
- Local implementation/release preparation is verified. Full Nix flake checks,
  CI, newly published package integrity and staging compatibility, hosted
  migrations/deployment, authorized historical revocation repair, Safari/device
  behavior and cross-region propagation remain unrun. The `connection-invites`
  browser flag remains off by default until the release-order gates pass. No
  commits, pushes, package publication, deployments or external credential
  withdrawals were performed; the starting dirty work and local data remain.

- Final native browser fixture compilation passes after the worker correction
  (3m16s). Storybook marks ACCT-C15/HANDOFF-22 locally verified with the final
  execution artifact separately from the retained screenshot provenance.
  Generation/freshness passes for 26 screens, 81 journeys and 120 verification
  items; all 198 local links pass. Final formatting and whitespace checks pass.

## Superseded work and evidence

The former single-use bootstrap/redemption design, short agent lifetimes,
connection-state authorization tables, mandatory D1 redemption race gate and
broad authorizer checkpoint are superseded by the 2026-09-16 decision.

Preserve these artifacts for reference; do not publish or integrate them unchanged:

- [Historical authorizer proposal](001-connection-authorizer-proposal.md).
- `patches/dialog-connection-authorizer.patch`.
- `scripts/test-connection-authorizer.sh` and `spikes/connection-policy.rs`.
- Existing service/invite characterization tests.

The signed-S3 expiry finding remains relevant. The fixture policy's bootstrap
denial, marker lookup and session-state revoke are not the accepted protocol.
An ordinary valid bearer working before confirmation is intended behavior.
Rework tests around this contract in milestone 0/1; do not delete unrelated work.

The following original log is retained verbatim as history. Its “next” steps,
stop gates, publication request and completion claims apply only to the abandoned
checkpoint experiment, not the current milestones. Nothing below establishes
completion of the revised two-flow design.

### Historical execution log (2026-09-15 through 2026-09-16; superseded)

Continuation: the user requested proceeding with the proposed authorizer change.
The main Dialog checkout has unrelated staged changes and an unresolved merge;
it is untouched. `.wt/dialog-connection-authorizer` is an isolated local clone of
the exact pinned revision `4c16de9e345d2b2d888d1008c5d3f0ca990c4807`.
The next tested increment is the additive verified-chain policy API and deadline
clamping. Its portable patch and an isolated consumer test harness will live in
ordinary repository files, so the work does not depend on editing Cargo's cache
or on leaving a local-path dependency in the user's lockfile. Publication and
the production dependency pin remain separate from local verification.

Continuation results:

- Durable artifacts: `patches/dialog-connection-authorizer.patch` (four upstream
  files), `scripts/test-connection-authorizer.sh`, and
  `spikes/connection-policy.rs`. The patch matches the isolated upstream diff
  byte-for-byte and applies cleanly to the recorded revision.
- Upstream `cargo test -p dialog-remote-ucan-s3 --lib --no-default-features`:
  41 passed, including 7 new policy/deadline tests. Upstream Wasm library check,
  crate formatting, and whitespace passed. Existing native server tests required
  local-network access; the unchanged retry passed.
- Harness `--prepare-only` passed and generated 21 Dialog package overrides in
  an isolated source copy, avoiding duplicate dependency types. It registers the
  consumer test only in that copy. Tonk's original `Cargo.toml`/`Cargo.lock` still
  match HEAD; the main Dialog checkout is untouched.
- In the prepared snapshot, with its generated `--config` file:
  `cargo test -p tonk-access-service --features helpers --test connection_policy_spike`
  passed 1 test after the final edit. It proves signed S3 PUT/GET, refusal for the
  bootstrap and another session key, marker shadow resistance, two distinct
  operator keys beneath the bound session, and denial of new read/write permits
  after fixture revocation. Issued URLs remain usable within the <=60-second
  lifetime; retained downloaded bytes remain available.
- The consumer's first compile exposed a fixture signature type mismatch
  (`AnySignature` vs `Ed25519Signature`), corrected in the fixture. The native
  S3 listener then hit sandbox `Operation not permitted`; unchanged rerun with
  local-network access passed. Review prompted a second operator key to prove
  rotation rather than merely one operator hop; the final rerun passed.
- The copied service's existing `--test connections` probes passed 2 tests,
  confirming the legacy authorizer remains compatible. The copied service's
  `cargo check -p tonk-access-service --target wasm32-unknown-unknown` passed.
  Existing dependency deprecation/unused-import warnings remain.
- Full workspace formatting, explicit spike formatting, shell syntax and
  whitespace are checked independently; no CI, D1, browser, or hosted checks
  are implied. Fixture management is not authenticated issuance/redemption or
  SQLite/D1 durability, and Tonk's production handler still uses the legacy API.
  The original milestone 1 race/revoke gate and milestones 2–7 remain incomplete.
- The recommended API change is concrete and reviewable. Publishing it and
  moving Tonk's git pin requires the external publication authorization excluded
  by this plan. No dependency publication, push, PR, deployment, or credential
  revocation has occurred. Local patch validation is not a published dependency.

- Live HEAD remains `8acaa1897d3ed09a7bbde972f55060761d89f7f9`.
  The pre-existing tracked diff contains 14 files (251 additions, 106 deletions):
  CLI parser/handoff/tests/README, both prompt assets, worker prompt tests,
  browser account-flow tests/settings copy, and Storybook sources/indexes.
  `plan/cli-join-agent.md`, this plan, and `plans/README.md` were untracked.
  These changes are the preserved starting point, not this execution's work.
- Agent work is bounded by the sequential gates: baseline and compatibility
  verification; access-service security spike; invitation/attenuation contract.
  CLI replacement, migration, and browser integration wait for the protocol gate.
- The current service identity uses its configured `did:key`; its `did:web`
  document publishes that key but is not yet the invocation audience. A new
  envelope must bind the actual verified service audience and trusted routing.
- Existing control migrations end at `0006_account_schema.sql`. No local D1 race
  harness was found in the service, scripts, or CI paths inspected; milestone 2
  must add the specified disposable harness before claiming D1 evidence.
- Fresh baseline checks passed: parser/help 37 tests; handoff 11 tests;
  `join_profile` 1 test; library handoff/account constraints 6 tests.
  Commands are the three baseline Cargo commands in the table below. The
  committed drift comparison is empty and `git diff --check` passed.
- Local release-source evidence: tag `v0.6.15` resolves to
  `8e4d17aacae547f478eafb2dda322f906194efd0` and exposes public `connect`,
  ordinary `join`, and `account login`, without `join --agent`. The exact new
  spelling has no introducing commit in local history. The npm wrapper's
  checked-in version differs from the workspace version; published-registry
  compatibility remains unverified and must not be inferred from either.
- The pinned Dialog revision is `4c16de9e345d2b2d888d1008c5d3f0ca990c4807`.
  Its high-level authorizer verifies then immediately presigns, returning only
  a `Permit`. Core verification and ancestor inspection are separately public,
  so signed-chain inspection is possible. Descriptor expiry is the immediate
  limitation: the high-level API offers no lifetime setting and its translated
  S3 requests default to 3,600 seconds.
- The plan's explicit authorizer/deadline stop condition applies. See
  [the bounded authorizer API proposal](001-connection-authorizer-proposal.md)
  for the recommended verified-chain policy hook, a smaller deadline-only
  fallback, and why duplicating the dispatcher is not adopted. No new protocol
  envelope, production grant issuance, CLI authority replacement, or migration
  has been implemented. Milestone 0's wire/rights contract is still provisional;
  milestone 1's security exit gate has not passed.
- Added isolated characterization tests under
  `rust/tonk-access-service/tests/connections.rs` and
  `rust/tonk-invite/tests/connection.rs`. They document existing primitive/API
  limitations, not completed session enforcement. No dependency/lockfile or
  pre-existing source edits were changed by this execution.
- Test setup corrections: the original parameterized service helper was excluded
  by the default integration-test configuration; an explicit local-server fixture
  makes the required filter select both probes. Its first run hit sandbox
  `Operation not permitted` at local server setup; the unchanged command passed
  after retry with local-server access.
- Final fresh checks:
  - `cargo test -p tonk-access-service --features helpers connection`: 2 passed.
    The real local `/ucan/` endpoint returned a GET descriptor to an unredeemed,
    signed, marked bootstrap grant. Both probes observed `X-Amz-Expires=3600`
    despite an ancestor expiring within 120 seconds. This is descriptor issuance
    evidence, not a successful S3 object read/write or a secure session test.
  - `cargo test -p tonk-invite connection`: 2 passed. Signed ancestor metadata
    survives a descendant's shadowing, and generic claims remain repeatable to
    different keys while retaining a fixed historical ancestor deadline.
  - `cargo test -p tonk-invite`: 27 unit tests and 2 integration tests passed.
  - `cargo fmt --all -- --check` and `git diff --check`: passed after final test
    edits. The initial formatting check identified only the new service test;
    formatting was corrected before the final check.
- Not established: redemption races, remote writes/readback, revocation ordering,
  operator rotation, deterministic service clock boundaries, D1/Miniflare, Wasm,
  new CLI process flows, browser E2E, published npm compatibility, CI, staging or
  production. Those gates remain outstanding. No releases or external changes
  were performed.
