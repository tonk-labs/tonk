# Implementation plans

## Identity discovery research

[2026-10-20 investigation](2026-10-20-identity-discovery-research.md):
email-to-root discovery and scoped invitation APIs already exist. Recommended
order is an email/contact sharing slice, domain binding, then optional AT Protocol
association. Research complete. The [first native integration spike](2026-10-20-identity-discovery-spike.md)
connects confirmed email lookup to targeted invitations. The
[sender-attribution fix](2026-10-20-invitation-sender-attribution.md) binds new
worker invitations to the minting account; the updated spike verifies Bob as
inviter while Alice remains founder. Local verification passed (1 integration
test and 64 focused/library tests). The [CLI follow-up](2026-10-20-cli-invitation-sender-attribution.md)
adds the same signed sender attribution, with seven CLI checks passing and the
worker spike rerun successfully. Legacy links retain their attribution. Estimates,
evidence, trust boundaries and proposed verification gates are in the research note.

[Signup invitation follow-up](2026-10-21-email-invitation-signup-spike.md):
prove delivery to a subsequently confirmed account while the inviter is offline,
retaining the original sender separately from the delivery service. Product
work will extend the existing Address Book in desktop redesign #1134.

## Release gates

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [012: Promote verified candidates through staging to stable](012-release-gates.md) | P1 | READINESS IMPLEMENTED LOCALLY; enforcement deferred | Candidate reports and manual verification; hosted product validation and channel migration remain |

Browser and CLI first. Prepared at `e7388a98f7` on 2026-10-13 using current
workflow source and read-only GitHub policy inspection. Checkpoints depend on
the preceding checkpoint; no other plan is a prerequisite. Existing release
workflows and remote settings remain unchanged. The plan records rejected
branch-promotion and mutable-checklist alternatives, plus the GitHub Release
metadata/CLI channel boundary that must be resolved before enforcement is
claimed complete.

## CI compile time

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [012: Shorten PR verification and retain optimized release gates](012-ci-compile-time.md) | P1 | IMPLEMENTED; hosted validation pending | Matched test caches; shared E2E artifacts and parallel checks; PR profiles and exact-tag release gates |

The [2026-10-13 audit](2026-10-13-ci-compile-audit.md) records hosted timing evidence. All five recommendations are implemented; shipping profiles remain unchanged and optimized suites now run after merge/on tags. Local verification passed 113 focused archive tests, 8 gate tests and 16 artifact/server tests. The plan records the unchanged lifecycle-test timeout and outstanding full hosted verification and measurements.

## Agent access management

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [007: Separate invitations from named space access](007-agent-access-management.md) | P1 | DONE (checkpoints 1–3) | Confirmed-only settings, named resumable receipts, and per-space shared-holder access verified |

007 preserves 005's invitation authority model for its first three checkpoints.
Separate links are already independently revocable. Single-use redemption is
out of scope; copies of one link continue to share its revocation boundary.
Initial source drift was empty against `503a5d637`. Checkpoints 1–3 are complete:
native, Wasm, and all four focused packaged browser/local-service journeys passed
(browser retries=0), including revoked remote access and preserved downloaded
data. Desktop/narrow/dark captures, artifact identities, setup failures, and
unrun old-CLI/hosted checks are recorded in 007's execution log. Post-rebase
settings layout and filtering checks also passed; remote journeys were not
rerun after the rebase. This work is prepared for a pull request; deployment
remains outside scope.

Existing cold-start and Welcome plans are preserved. The CLI review plan below is grounded in `ea730cc18` and covers the two requested findings only.

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [001: CLI join review fixes](001-cli-join-review-fixes.md) | P1 / P2 | DONE | Browser ownership handoff, local-space linking, space-only status, and browser integration verified |
| [003: Both invitation types through join](003-cli-join-both-invite-types.md) | P1 | DONE; new-import design superseded by 005 | Historical implementation: commits `01da82175`..`d755e9c5f`; native gates and local-service integration passed; live browser and external hosted checks not run |
| [005: Tool-only CLI connections](005-cli-device-invitations.md) | P1 | DONE | Tool-only routing, explicit browser actions, native gates, and the feature-enabled packaged browser journeys passed |
| [005: Update hub and settings to latest wireframes](005-update-hub-wireframes.md) | P1 | IN PROGRESS | Responsive collection, settings, create metadata, rename, and removal implemented; account readiness/sync, copy/Discover, Safari, and visual matrix remain |

No additional findings were audited or rejected. Earlier CLI connection plans removed from this branch are historical context, not active prerequisites.

## Current CLI design decision

Plan 005 was implemented in the uncommitted worktree based at `d4003e8d6` on
2026-09-22. Tool-only routing, account isolation, explicit browser actions, and
native gates are complete. The approved E2E task extension enables
`connection-invites`, serves the preview artifact, and executed the two named
tool-connection journeys plus the existing person-membership regression.
Existing replicas and supported agent links remain intact. Ordinary person
invitations remain a browser flow.

Rejected alternatives: automatically create a CLI account for ordinary links;
reinterpret ordinary links as tool connections; redelegate agent invitations to
a shared CLI device key. Each obscures the explicit person-versus-tool boundary
or changes the agreed invitation identity. These are design decisions, not new
audit findings. Existing cold-start and Welcome plans are unchanged.

## FABB v0.17.0 update

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [005: Update the in-space UI to FABB v0.17.0](005-fabb-v017-ui-update.md) | P1 | PARTIAL | Bar, attached panels and contained tasks implemented; member graph awaits authoritative relationship/activity data |

Grounded in Tonk `d4003e8d6` and Gooey `f74da0e` on 2026-09-22. Full members-map parity depends on an authoritative relationship/activity data contract; the current roster projection does not provide it.

Earlier `plan/fabb-conformance.md`, `plan/fabb-mobile.md`, and `plan/fabb-share.md` remain historical references; 005 supersedes their conflicting in-space anatomy and presentation targets when executed. Preserve their still-applicable lifecycle, authority and viewport guarantees.

Considered and excluded: changes/review/history (outside the current reference MVP); wholesale Hub restyling (outside this FABB-focused plan); importing mock authentication or fabricated member activity (not production data contracts).

Plan 005 is an independent UI handoff added on 2026-09-22. It compares the latest local hub prototype with the current schema-driven UI. Its capability spikes must precede claims of full preview/Discover parity; its first layout increment has no dependency on those capabilities. It intentionally excludes obsolete prototype search/sort controls, simulated account state, and a second SPA architecture.

## Hosted MCP client interoperability

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [008: Client-neutral hosted MCP](008-client-neutral-mcp.md) | P1 | IMPLEMENTED LOCALLY; acceptance gaps remain | Registry, DCR, shared host/Apps and Node/Python native tests pass; packaging passes; external clients and CIMD remain unverified/deferred |
