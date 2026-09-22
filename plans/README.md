# Implementation plans

Existing cold-start and Welcome plans are preserved. The CLI review plan below is grounded in `ea730cc18` and covers the two requested findings only.

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [001: CLI join review fixes](001-cli-join-review-fixes.md) | P1 / P2 | DONE | Browser ownership handoff, local-space linking, space-only status, and browser integration verified |
| [003: Both invitation types through join](003-cli-join-both-invite-types.md) | P1 | DONE; new-import design superseded by 005 | Historical implementation: commits `01da82175`..`d755e9c5f`; native gates and local-service integration passed; live browser and external hosted checks not run |
| [005: Tool-only CLI connections](005-cli-device-invitations.md) | P1 | DONE | Tool-only routing, explicit browser actions, native gates, and the feature-enabled packaged browser journeys passed |

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
