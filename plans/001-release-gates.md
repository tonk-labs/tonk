# CLI space connections: release and rollback gates

Status: local implementation and integration checks verified. External release
gates remain unrun. This file does
not authorize deployment, publishing, or withdrawal of external credentials.
The controlling contract is [plan 001](001-cli-space-connections.md).

## Ordering

1. Release and verify the standard revocation authority-subject correction,
   current-caller-proof check, and maximum 60-second signed S3 descriptors.
   Preserve existing revocation facts. Measure propagation using separate
   holders and an independent still-authorized grant group.
2. Apply additive mailbox migrations 0007–0009 with the delivery-capable
   service. Verify authenticated initial delivery and additions against the real
   deployed storage adapter, including restart and account deletion cleanup.
   Route `/connection/*` to the service before the SPA asset fallback.
3. Publish a CLI that accepts both legacy account-scoped handoffs and v1 scoped
   bearer invitations and implements selected terminal linking. Verify the exact
   published package and platform executable integrity; a workspace build does
   not satisfy this gate.
4. Verify the published CLI with the candidate browser on staging: copied bearer
   after issuer shutdown, two holders, independent invites, terminal one/many/all
   selections, explicit conversion, offline additions/removals and revocation.
5. Only then enable `connection-invites` for browser issuance. The default build
   keeps legacy issuance until that compatibility gate is recorded. Legacy copied
   instructions use the already-supported `connect` spelling. Library seed
   reconciliation must preserve authored views and space home choices.

## Compatibility matrix to record

| Browser / CLI | Required observation | Evidence state |
| --- | --- | --- |
| Legacy issuance / downloaded CLI 0.6.15 | Existing account-scoped `connect` flow and explicit mismatch handling | Local baseline artifact and `connect --help` inspected; actual historical compatibility results in main plan. |
| Legacy issuance / candidate CLI | Existing handoffs keep their authority and recovery behavior | Final full CLI suite passed 628 tests across 33 targets; separately enabled released-CLI compatibility passed one test. |
| Scoped issuance / candidate CLI | Accountless reusable bearer, six rights, closed issuer, independent CLI account | Local milestone 3 browser/CLI and final ordinary-grant service checks passed; candidate staging gate remains unrun. |
| Scoped issuance / CLI 0.6.15 | Refuse unsupported scoped layout before ambient account authority | Local published-executable compatibility fixture passed; no new envelope support is claimed. |
| Selected browser linking / candidate CLI | Exact CLI key, complete selection, bounded request, durable resume and cancellation | Final snapshot artifact passes one/many/all, conversion, decline and management; historical intermittent-refusal attribution remains qualified below. |
| Candidate browser / newly published candidate CLI | Both journeys and migration with exact package integrity | Unrun: publication and staging require operator release authorization. |

## Verified local checkpoint (2026-09-16)

These results describe the dirty worktree based on
`8acaa1897d3ed09a7bbde972f55060761d89f7f9`, not a published release or clean commit.
Exact browser artifact identities and the full execution checkpoints
remain in [the execution log](001-cli-space-connections.md#current-execution-2026-09-16).

| Boundary | Local result |
| --- | --- |
| Ordinary grants and signed delivery | Required `cargo test --offline -p tonk-access-service --features helpers connection -- --test-threads=1` passed six storage, four ordinary-grant HTTP and one signed-delivery HTTP tests. The two scripted Worker fixtures are excluded by the normal test filter. |
| Account deletion | Seven existing deletion regressions passed. Real SQLite verifies account-isolated mailbox/chunk cleanup and refuses a late initial write after completed purge. |
| Production Worker storage adapter | `sh scripts/test-terminal-delivery-worker.sh` rebuilt the final guarded Worker and passed disposable D1/KV/R2 tests with 334 spaces: 2,111,908-byte signed approval and 2,111,941-byte signed addition, concurrent complete-or-absent reads, immutable replay/conflict, restart, forced chunk rollback, authenticated purge, other-account preservation and retained revocation facts. This is local production-handler evidence. |
| Schema and service lint | All three schema tests passed; strict service Clippy passed with helpers and all targets. |
| Shared invite codec | Full `tonk-invite` suite passed 46 tests, covering signed selections, addition identity pins, account-catalogue refusal, historical inspection and current grant bounds. |
| Browser library contracts | All 33 worker `standard_library` tests passed using the freshly built full test executable, including legacy `connect` copy and scoped prompt separation. |
| CLI full regression and lifetime separation | Full suite passed 628 tests across 33 targets, zero failures and two ignored. The released-CLI compatibility test passed separately; the remaining ignored test is the preexisting post-#447 schema-analyzer port. Eleven focused recovery tests include expired management authorization with independently live shared-space grants. |
| Final browser journeys | Navigation artifact `63775cbb5273ce15` passed actual one/many/all selection (29.24 s) and explicit conversion (39.84 s). Final management passed (26.50 s); fresh-sign-in decline also passed. These passes do not erase the intermittent pre-staging refusal investigation. |
| Workspace strict lint | Final strict workspace Clippy passed (2m20s) after the snapshot fix and test-only diagnostics; formatting and whitespace checks also pass. |

Migration `0007_connection_delivery.sql` stores immutable signed initial
approvals addressed to the exact CLI recipient, with no anonymous pending rows.
`0008_connection_additions.sql` adds immutable addressed additions and transport
cursors pinned to the original account and recipient. Neither row type grants or
withdraws data access. `0009_connection_payload_chunks.sql` adds encoded payload
sizes and ordered 512 KiB chunks, at most 16 per payload. One transaction/batch
commits the parent and complete children; reads verify digest, length and order.
The signed format remains bounded to 4 MiB and 1,024 selected spaces, without
truncation. The large local D1 fixture crosses the published 2,000,000-byte row
limit; it is not a hosted maximum-capacity measurement.

The account-purge race was demonstrated before correction: a publisher already
past the outer customer check could recreate a mailbox after deletion. Initial
publication now checks the existing customer's `Active` state inside the atomic
INSERT too. A lost race reports account unavailable; additions require the
surviving addressed initial parent. Completed account purge deletes only its
mailbox copies and cascading chunk children, preserving other accounts,
revocations and client-retained proofs/data.

`Approval::inspect` and `Addition::inspect` authenticate the original signed
publication at `issued_at` for historical management and immutable identity pins.
They do not establish live authority. First service publication still requires
current validation, active account state and standard invocation verification
against the revocation index. The CLI rechecks each received space bundle at the
current time before new installation; data operations remain subject to normal
service revocation enforcement. An expired independent management proof does not
shorten an otherwise valid space grant. Crash recovery may recognize an exact
already-installed selection historically, but cannot use that shortcut to install
expired grants or manufacture success receipts.

## Remaining local and external gates

Final artifact `d7f3967a6db1fa1a` (worker `4ebdb9f0b3ee3f13`, manifest
`e96677dc59ffedfad02552587c6f5aea28e75abaaa18defcba49786359f04c00`)
passes one/many/all-current selection (26.97s), fresh-sign-in decline (8.83s),
offline management with a revoked queued addition and fresh re-add (36.98s),
and explicit legacy-account conversion (13.95s), all with the final local CLI.
The same-path/new-fragment navigation fix passes repeated requests. The final
worker gate passes four tests, strict all-target/all-feature workspace Clippy
passes, and formatting/whitespace checks pass.

The original full candidate DTO snapshot could change on display-name or
availability-text updates, unnecessarily rejecting unchanged reviewed identities.
A deterministic regression failed before correction and passes after it. The
fingerprint now pins the account proof and sorted repository/subject membership;
selected grants still undergo current authority validation before staging. An
account or actual space membership change still requires renewed review.

Earlier intermittent initial refusals had no captured original response. A saved
retry found no staged approval, but that does not identify the initial failure.
Do not claim all those failures were conclusively traced to the snapshot bug.
Final affected journeys pass, and first-response failure diagnostics remain for
staging monitoring. No reproduced failure remains on the final artifact.

The management test proves an unsynced edit using `eval --no-sync`; an earlier
post-removal no-op push was a fixture error. Exact local queries prove retained
edits after withdrawal, without assuming uncached remote indexes are available.
Final browser logs are `/private/tmp/tonk-terminal-snapshot-{selected,decline,
management,conversion}.log`; final strict lint is
`/private/tmp/tonk-plan001-clippy-final-snapshot.log`. The whole CLI suite passed
628 tests with two ignored; released-CLI compatibility was then run separately
and passed. The remaining ignored schema-introspection test requires the
pre-existing post-#447 analyzer port. Logs are local, not hosted evidence.

CI, hosted schema migration, service deployment, staging, newly published CLI
registry/package integrity, Safari/real-device behavior and global revocation
propagation remain unrun. No release authorization or automatic publication is
implied. Existing local registry and historical-executable tests do not establish
new package-registry publication integrity.

## Revocation repair

The old service could acknowledge a sibling-device withdrawal under the signer
rather than the authenticated account. Empty KV facts do not contain enough
information to infer replacement account keys. Do not relabel, delete, or bulk
replay them. An authorized operator must inventory actual affected records and
recover original signed evidence or deliberately issue a new withdrawal for an
explicit retained target. The CLI's targeted retry now republishes a retained
grant even if its catalogue row is absent; this is not a mass repair job.

Record the original target CID, actual verified authority, service artifact
identity and receipt for each authorized repair. Never record bearer invitation
seeds, private keys, passkey material, account content, or activation URLs.
Hosted prevalence and cross-region propagation have not been measured locally.

## Rollback constraints

Browser issuance can return to legacy prompts while existing scoped credentials
and public management records remain. Keep the compatible CLI available for
already-issued v1 grants. Do not remove local replicas, aliases, unsynced edits,
retained credentials, standard revocation facts, or public proof chains merely
because browser issuance is disabled.

Keep the revocation fixes and signed URL bound in any service rollback. Returning
to the old signer-index writer or bypassing current caller revocation recreates
known defects. Retain additive mailbox schema/data while rolling back UI exposure;
account deletion still removes its service mailbox copies. Apply migration 0009
before enabling the chunk writer: the new reader accepts legacy inline values,
but an older service reader cannot decode new chunk references. Any rollback
must keep a chunk-aware reader for mailbox availability. A failed deployment
must not turn delivery records into an authorization ACL or shorten ordinary
long-lived grants.

## Evidence boundaries

Local Rust/Chrome, native HTTP/SQLite/S3, and Miniflare Worker/D1 results are
separate evidence classes. Record source revision plus dirty-worktree state,
browser and CLI artifact identities, test names, outcomes and known failures.
CI, staging, npm publication, production deployment, Safari/device behavior and
global propagation need their own dated evidence. None is implied by local passes.
