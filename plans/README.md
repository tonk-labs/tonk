# Implementation plans

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
